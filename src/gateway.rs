use std::time::{Duration, Instant};

use axum::{
    Json,
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use bytes::Bytes;
use chrono::{DateTime, Utc};
use futures_util::StreamExt;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    affinity::{lookup_affinity, remember_affinity},
    app::AppState,
    auth::GatewayAuth,
    db::ChannelHealthEventInput,
    error::{AppError, AppResult},
    events::{TOPIC_CHANNELS, TOPIC_DASHBOARD, publish_channel_owner_event},
    models::{
        Channel, ChannelQuotaWindow, GatewayContext, GatewayReservation, LedgerEvent, ProviderKind,
        Usage,
    },
    pricing::{fire_sale_discount, select_price, settle},
    protocol::{
        ClientProtocol, ProviderProtocol, client_response_body, extract_usage,
        parse_client_request, provider_protocol, response_has_semantic_content, same_wire_protocol,
        stream_chunk_has_semantic_content, translate_stream_chunk, upstream_body, upstream_path,
    },
    routing::{RouteDecision, choose_channel},
    settings::RuntimeSettings,
};

const SECONDS_PER_HOUR: f64 = 60.0 * 60.0;

pub async fn openai_chat_completions(
    State(state): State<AppState>,
    GatewayAuth(auth): GatewayAuth,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> AppResult<Response> {
    handle_gateway(state, auth, headers, body, GatewayEndpoint::openai_chat()).await
}

pub async fn openai_responses(
    State(state): State<AppState>,
    GatewayAuth(auth): GatewayAuth,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> AppResult<Response> {
    handle_gateway(
        state,
        auth,
        headers,
        body,
        GatewayEndpoint::openai_responses(),
    )
    .await
}

pub async fn anthropic_messages(
    State(state): State<AppState>,
    GatewayAuth(auth): GatewayAuth,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> AppResult<Response> {
    handle_gateway(
        state,
        auth,
        headers,
        body,
        GatewayEndpoint::anthropic_messages(),
    )
    .await
}

pub async fn gemini_generate_content(
    State(state): State<AppState>,
    GatewayAuth(auth): GatewayAuth,
    headers: HeaderMap,
    Path(model_action): Path<String>,
    Json(body): Json<Value>,
) -> AppResult<Response> {
    let (model, action, stream) = parse_gemini_model_action(&model_action)?;
    handle_gateway(
        state,
        auth,
        headers,
        body,
        GatewayEndpoint::gemini(action, model, stream),
    )
    .await
}

#[derive(Debug, Clone)]
struct GatewayEndpoint {
    client_protocol: ClientProtocol,
    request_path: &'static str,
    path_model: Option<String>,
    path_stream: Option<bool>,
}

impl GatewayEndpoint {
    fn openai_chat() -> Self {
        Self {
            client_protocol: ClientProtocol::OpenAiChatCompletions,
            request_path: "/v1/chat/completions",
            path_model: None,
            path_stream: None,
        }
    }

    fn openai_responses() -> Self {
        Self {
            client_protocol: ClientProtocol::OpenAiResponses,
            request_path: "/v1/responses",
            path_model: None,
            path_stream: None,
        }
    }

    fn anthropic_messages() -> Self {
        Self {
            client_protocol: ClientProtocol::AnthropicMessages,
            request_path: "/v1/messages",
            path_model: None,
            path_stream: None,
        }
    }

    fn gemini(request_path: &'static str, model: String, stream: bool) -> Self {
        Self {
            client_protocol: ClientProtocol::GeminiGenerateContent,
            request_path,
            path_model: Some(model),
            path_stream: Some(stream),
        }
    }
}

fn parse_gemini_model_action(model_action: &str) -> AppResult<(String, &'static str, bool)> {
    let Some((model, action)) = model_action.rsplit_once(':') else {
        return Err(AppError::BadRequest(
            "gemini route requires model:generateContent or model:streamGenerateContent"
                .to_string(),
        ));
    };
    match action {
        "generateContent" => Ok((model.to_string(), "/v1beta/models/:generateContent", false)),
        "streamGenerateContent" => Ok((
            model.to_string(),
            "/v1beta/models/:streamGenerateContent",
            true,
        )),
        _ => Err(AppError::BadRequest(format!(
            "unsupported gemini action: {action}"
        ))),
    }
}

async fn handle_gateway(
    state: AppState,
    auth: crate::models::AuthContext,
    headers: HeaderMap,
    raw_body: Value,
    endpoint: GatewayEndpoint,
) -> AppResult<Response> {
    let client_protocol = endpoint.client_protocol;
    let request_path = endpoint.request_path;
    let mut parse_body = raw_body.clone();
    if let Some(model) = endpoint.path_model {
        parse_body["_model"] = Value::String(model);
    }
    if let Some(stream) = endpoint.path_stream {
        parse_body["_stream"] = Value::Bool(stream);
    }
    let request = parse_client_request(client_protocol, &parse_body)?;
    let api_key = auth.api_key.clone().ok_or(AppError::Unauthorized)?;
    ensure_api_key_model_allowed(&api_key, &request.model)?;
    state.db.refresh_channel_windows().await?;
    let settings = state.db.runtime_settings().await?;
    let global_prices = state.db.global_price_book().await?;
    let reserve_price = select_price(&request.model, &global_prices, &settings);
    let token_estimate = crate::tokenizer::estimate_request_tokens(&request);
    let reserve = token_estimate.tokens as f64 * reserve_price.input_price_per_1k
        / settings.pricing_unit_tokens;
    ensure_affordable(&auth.user, &api_key, reserve)?;

    let gateway_context = GatewayContext::default();
    let affinity_hit = lookup_affinity(
        &state.db,
        &state.affinity_cache,
        request_path,
        &headers,
        &raw_body,
        &request,
        &gateway_context,
    )
    .await?;

    let request_id = Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string();
    let route_count = api_key.allowed_channel_ids.len();
    let max_attempts = route_count.clamp(1, settings.routing_max_attempts);
    let retry_cooldown = Duration::from_secs(settings.routing_retry_cooldown_seconds);
    let mut last_retry_error: Option<AppError> = None;

    for attempt_index in 0..max_attempts {
        state.db.refresh_channel_windows().await?;
        let channels = state.db.list_route_channels_for_api_key(api_key.id).await?;
        let decision = match choose_channel(
            &channels,
            &request.model,
            affinity_hit.clone(),
            &state.router_state,
            settings.routing_fire_sale_weight_multiplier,
        )
        .await
        {
            Ok(decision) => decision,
            Err(err) => {
                if let Some(last_retry_error) = last_retry_error {
                    return Err(last_retry_error);
                }
                return Err(err);
            }
        };
        let price = select_price(
            &request.model,
            &state.db.price_book_for_channel(decision.channel.id).await?,
            &settings,
        );
        let attempt_surge_multiplier = surge_multiplier_from_channels(
            &settings,
            &channels,
            state.metrics.tokens_last_hour(),
            Utc::now(),
        )
        .0;
        let selected_reserve =
            token_estimate.tokens as f64 * price.input_price_per_1k / settings.pricing_unit_tokens;
        ensure_affordable(&auth.user, &api_key, selected_reserve)?;
        let reservation = match state
            .db
            .reserve_gateway_request(
                auth.user.id,
                api_key.id,
                decision.channel.id,
                token_estimate.tokens,
                selected_reserve,
            )
            .await
        {
            Ok(reservation) => reservation,
            Err(err)
                if is_retryable_reservation_error(&err)
                    && can_retry_after_failure(&decision)
                    && has_retry_left(attempt_index, max_attempts) =>
            {
                state
                    .router_state
                    .mark_cooldown(decision.channel.id, retry_cooldown)
                    .await;
                last_retry_error = Some(err);
                continue;
            }
            Err(err) => return Err(err),
        };

        let attempt_started = Instant::now();
        let upstream = match send_upstream(&state, &decision, client_protocol, &raw_body, &request)
            .await
        {
            Ok(response) => response,
            Err(err) => {
                let error = err.to_string();
                record_channel_health(
                    &state,
                    &decision,
                    GatewayHealthEvent {
                        request_id: &request_id,
                        status: "down",
                        http_status: None,
                        ttft_ms: None,
                        total_latency_ms: Some(elapsed_ms(attempt_started)),
                        error: Some(error.as_str()),
                    },
                )
                .await;
                state.db.release_gateway_reservation(&reservation).await?;
                state
                    .router_state
                    .mark_cooldown(decision.channel.id, retry_cooldown)
                    .await;
                if can_retry_after_failure(&decision) && has_retry_left(attempt_index, max_attempts)
                {
                    last_retry_error = Some(err);
                    continue;
                }
                return Err(err);
            }
        };

        let status = upstream.status();
        if is_retryable_upstream_status(status) {
            state
                .router_state
                .mark_cooldown(decision.channel.id, retry_cooldown)
                .await;
            if can_retry_after_failure(&decision) && has_retry_left(attempt_index, max_attempts) {
                let health_status = if status == StatusCode::TOO_MANY_REQUESTS {
                    "degraded"
                } else {
                    "down"
                };
                let error = format!("HTTP {status}");
                record_channel_health(
                    &state,
                    &decision,
                    GatewayHealthEvent {
                        request_id: &request_id,
                        status: health_status,
                        http_status: Some(status.as_u16() as i64),
                        ttft_ms: None,
                        total_latency_ms: Some(elapsed_ms(attempt_started)),
                        error: Some(error.as_str()),
                    },
                )
                .await;
                state.db.release_gateway_reservation(&reservation).await?;
                last_retry_error = Some(AppError::Upstream(format!(
                    "upstream channel {} returned {status}",
                    decision.channel.id
                )));
                continue;
            }
        }
        let finish = FinishContext {
            state: state.clone(),
            auth: auth.clone(),
            api_key: api_key.clone(),
            decision: decision.clone(),
            request: request.clone(),
            client_protocol,
            request_id: request_id.clone(),
            price: price.clone(),
            reservation: reservation.clone(),
            surge_multiplier: attempt_surge_multiplier,
            attempt_started,
        };
        match finish_response(finish, upstream).await? {
            FinishOutcome::Response(response) => return Ok(response),
            FinishOutcome::Retry(err) => {
                state
                    .router_state
                    .mark_cooldown(decision.channel.id, retry_cooldown)
                    .await;
                if can_retry_after_failure(&decision) && has_retry_left(attempt_index, max_attempts)
                {
                    last_retry_error = Some(err);
                    continue;
                }
                return Err(err);
            }
        }
    }

    Err(last_retry_error.unwrap_or_else(|| {
        AppError::BadRequest("no healthy channel for requested model".to_string())
    }))
}

fn has_retry_left(attempt_index: usize, max_attempts: usize) -> bool {
    attempt_index + 1 < max_attempts
}

fn can_retry_after_failure(decision: &RouteDecision) -> bool {
    !matches!(
        &decision.affinity_hit,
        Some(hit) if hit.rule.skip_retry_on_failure && hit.channel_id == Some(decision.channel.id)
    )
}

fn is_retryable_upstream_status(status: StatusCode) -> bool {
    status == StatusCode::REQUEST_TIMEOUT
        || status == StatusCode::TOO_MANY_REQUESTS
        || status.is_server_error()
}

fn is_retryable_reservation_error(err: &AppError) -> bool {
    matches!(
        err,
        AppError::BadRequest(message)
            if message == "channel token quota no longer has enough room for the estimate"
    )
}

fn empty_response_error(channel_id: i64) -> AppError {
    AppError::Upstream(format!(
        "upstream channel {channel_id} returned an empty semantic response"
    ))
}

fn empty_stream_error(channel_id: i64) -> AppError {
    AppError::Upstream(format!(
        "upstream channel {channel_id} ended the stream before semantic content"
    ))
}

fn stream_error_before_content(channel_id: i64, err: reqwest::Error) -> AppError {
    AppError::Upstream(format!(
        "upstream channel {channel_id} stream failed before semantic content: {err}"
    ))
}

fn stream_chunk_has_done_marker(bytes: &Bytes) -> bool {
    let text = String::from_utf8_lossy(bytes);
    text.lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .any(|data| data.trim() == "[DONE]")
}

fn elapsed_ms(started: Instant) -> i64 {
    started.elapsed().as_millis().min(i64::MAX as u128) as i64
}

async fn record_channel_health(
    state: &AppState,
    decision: &RouteDecision,
    event: GatewayHealthEvent<'_>,
) {
    if state
        .db
        .record_channel_health_event(ChannelHealthEventInput {
            channel_id: decision.channel.id,
            request_id: Some(event.request_id),
            status: event.status,
            http_status: event.http_status,
            ttft_ms: event.ttft_ms,
            total_latency_ms: event.total_latency_ms,
            error: event.error,
        })
        .await
        .is_ok()
    {
        publish_channel_owner_event(
            state,
            decision.channel.owner_user_id,
            [TOPIC_DASHBOARD, TOPIC_CHANNELS],
        );
    }
}

struct GatewayHealthEvent<'a> {
    request_id: &'a str,
    status: &'a str,
    http_status: Option<i64>,
    ttft_ms: Option<i64>,
    total_latency_ms: Option<i64>,
    error: Option<&'a str>,
}

async fn retryable_empty_finish(finish: &FinishContext, err: AppError) -> AppResult<FinishOutcome> {
    finish
        .state
        .db
        .release_gateway_reservation(&finish.reservation)
        .await?;
    Ok(FinishOutcome::Retry(err))
}

fn ensure_api_key_model_allowed(
    api_key: &crate::models::ApiKeyRecord,
    model: &str,
) -> AppResult<()> {
    if api_key.allowed_models.is_empty() {
        return Ok(());
    }
    let allowed = api_key.allowed_models.iter().any(|pattern| {
        pattern == "*" || pattern == model || model.starts_with(pattern.trim_end_matches('*'))
    });
    if allowed {
        Ok(())
    } else {
        Err(AppError::Forbidden)
    }
}

fn ensure_affordable(
    user: &crate::models::User,
    api_key: &crate::models::ApiKeyRecord,
    reserve: f64,
) -> AppResult<()> {
    if user.points_balance < reserve {
        return Err(AppError::BadRequest(
            "insufficient points for estimated input tokens".to_string(),
        ));
    }
    if let Some(limit) = api_key.spend_limit_points
        && api_key.spent_points + reserve > limit
    {
        return Err(AppError::BadRequest(
            "api key spend limit would be exceeded".to_string(),
        ));
    }
    Ok(())
}

async fn send_upstream(
    state: &AppState,
    decision: &RouteDecision,
    client_protocol: ClientProtocol,
    raw_body: &Value,
    request: &crate::protocol::TextRequest,
) -> AppResult<reqwest::Response> {
    let provider_protocol = provider_protocol(&decision.channel.provider);
    let path = upstream_path(provider_protocol, &request.model, request.stream);
    let mut body = upstream_body(client_protocol, provider_protocol, raw_body, request)?;
    if provider_protocol == ProviderProtocol::GeminiGenerateContent
        && client_protocol != ClientProtocol::GeminiGenerateContent
    {
        body = normalize_gemini_image_parts(&state.http, body).await?;
    }
    let url = format!(
        "{}{}",
        decision.channel.base_url.trim_end_matches('/'),
        path
    );
    let mut builder = state.http.post(url).json(&body);
    builder = apply_provider_headers(
        builder,
        &decision.channel.provider,
        &decision.channel.api_key_secret,
    );
    builder
        .send()
        .await
        .map_err(|err| AppError::Upstream(err.to_string()))
}

async fn normalize_gemini_image_parts(
    client: &reqwest::Client,
    mut body: Value,
) -> AppResult<Value> {
    if let Some(contents) = body.get_mut("contents").and_then(Value::as_array_mut) {
        for content in contents {
            normalize_gemini_content_parts(client, content).await?;
        }
    }
    if let Some(system_instruction) = body.get_mut("systemInstruction") {
        normalize_gemini_content_parts(client, system_instruction).await?;
    }
    Ok(body)
}

async fn normalize_gemini_content_parts(
    client: &reqwest::Client,
    content: &mut Value,
) -> AppResult<()> {
    let Some(parts) = content.get_mut("parts").and_then(Value::as_array_mut) else {
        return Ok(());
    };
    for part in parts {
        normalize_gemini_part(client, part).await?;
    }
    Ok(())
}

async fn normalize_gemini_part(client: &reqwest::Client, part: &mut Value) -> AppResult<()> {
    let Some(map) = part.as_object_mut() else {
        return Ok(());
    };
    let Some(file_data) = map.get_mut("fileData").and_then(Value::as_object_mut) else {
        return Ok(());
    };
    let Some(file_uri) = file_data.get("fileUri").and_then(Value::as_str) else {
        return Ok(());
    };
    if !should_fetch_remote_image(file_uri) {
        return Ok(());
    }
    let provided_mime = file_data
        .get("mimeType")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let response = client
        .get(file_uri)
        .send()
        .await
        .map_err(|err| AppError::Upstream(err.to_string()))?
        .error_for_status()
        .map_err(|err| AppError::Upstream(err.to_string()))?;
    let mime_type = resolve_image_mime_type(
        response.headers().get(reqwest::header::CONTENT_TYPE),
        provided_mime,
    )
    .ok_or_else(|| {
        AppError::BadRequest("unable to determine image mime type for Gemini".to_string())
    })?;
    let bytes = response
        .bytes()
        .await
        .map_err(|err| AppError::Upstream(err.to_string()))?;
    let data = STANDARD.encode(bytes);
    map.remove("fileData");
    map.insert(
        "inlineData".to_string(),
        json!({
            "mimeType": mime_type,
            "data": data,
        }),
    );
    Ok(())
}

fn should_fetch_remote_image(file_uri: &str) -> bool {
    let lower = file_uri.to_ascii_lowercase();
    (lower.starts_with("http://") || lower.starts_with("https://"))
        && !lower.contains("generativelanguage.googleapis.com/v1beta/files/")
}

fn resolve_image_mime_type(
    header_value: Option<&reqwest::header::HeaderValue>,
    provided_mime: Option<String>,
) -> Option<String> {
    if let Some(mime) = provided_mime {
        let mime = mime.split(';').next().unwrap_or("").trim().to_string();
        if mime.starts_with("image/") {
            return Some(mime);
        }
    }
    header_value
        .and_then(|value| value.to_str().ok())
        .map(|value| value.split(';').next().unwrap_or("").trim().to_string())
        .filter(|value| value.starts_with("image/"))
}

fn apply_provider_headers(
    builder: reqwest::RequestBuilder,
    provider: &ProviderKind,
    api_key: &str,
) -> reqwest::RequestBuilder {
    match provider {
        ProviderKind::OpenAi => builder.bearer_auth(api_key),
        ProviderKind::Anthropic => builder
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01"),
        ProviderKind::Gemini => builder.header("x-goog-api-key", api_key),
    }
}

#[derive(Clone)]
struct FinishContext {
    state: AppState,
    auth: crate::models::AuthContext,
    api_key: crate::models::ApiKeyRecord,
    decision: RouteDecision,
    request: crate::protocol::TextRequest,
    client_protocol: ClientProtocol,
    request_id: String,
    price: crate::models::ModelPrice,
    reservation: GatewayReservation,
    surge_multiplier: f64,
    attempt_started: Instant,
}

enum FinishOutcome {
    Response(Response),
    Retry(AppError),
}

struct LedgerContext<'a> {
    state: &'a AppState,
    auth: &'a crate::models::AuthContext,
    api_key: &'a crate::models::ApiKeyRecord,
    decision: &'a RouteDecision,
    request: &'a crate::protocol::TextRequest,
    request_id: &'a str,
    price: crate::models::ModelPrice,
    reservation: &'a GatewayReservation,
    surge_multiplier: f64,
}

async fn finish_response(
    finish: FinishContext,
    upstream: reqwest::Response,
) -> AppResult<FinishOutcome> {
    let status = upstream.status();
    if finish.request.stream {
        return finish_streaming_response(finish, upstream).await;
    }

    let provider_protocol = provider_protocol(&finish.decision.channel.provider);
    if !status.is_success() {
        let error = format!("HTTP {status}");
        record_channel_health(
            &finish.state,
            &finish.decision,
            GatewayHealthEvent {
                request_id: &finish.request_id,
                status: if status == StatusCode::TOO_MANY_REQUESTS {
                    "degraded"
                } else {
                    "down"
                },
                http_status: Some(status.as_u16() as i64),
                ttft_ms: None,
                total_latency_ms: Some(elapsed_ms(finish.attempt_started)),
                error: Some(error.as_str()),
            },
        )
        .await;
        finish
            .state
            .db
            .release_gateway_reservation(&finish.reservation)
            .await?;
    }
    let value = match upstream.json::<Value>().await {
        Ok(value) => value,
        Err(err) => {
            if status.is_success() {
                let error = err.to_string();
                record_channel_health(
                    &finish.state,
                    &finish.decision,
                    GatewayHealthEvent {
                        request_id: &finish.request_id,
                        status: "down",
                        http_status: Some(status.as_u16() as i64),
                        ttft_ms: None,
                        total_latency_ms: Some(elapsed_ms(finish.attempt_started)),
                        error: Some(error.as_str()),
                    },
                )
                .await;
                finish
                    .state
                    .db
                    .release_gateway_reservation(&finish.reservation)
                    .await?;
            }
            return Err(AppError::Upstream(err.to_string()));
        }
    };
    if !status.is_success() {
        return Ok(FinishOutcome::Response(
            (status, Json(value)).into_response(),
        ));
    }
    if !response_has_semantic_content(&value, provider_protocol) {
        record_channel_health(
            &finish.state,
            &finish.decision,
            GatewayHealthEvent {
                request_id: &finish.request_id,
                status: "empty",
                http_status: Some(status.as_u16() as i64),
                ttft_ms: None,
                total_latency_ms: Some(elapsed_ms(finish.attempt_started)),
                error: Some("semantic empty response"),
            },
        )
        .await;
        return retryable_empty_finish(&finish, empty_response_error(finish.decision.channel.id))
            .await;
    }
    let (body, usage) = client_response_body(finish.client_protocol, provider_protocol, value);
    if status.is_success() {
        record_channel_health(
            &finish.state,
            &finish.decision,
            GatewayHealthEvent {
                request_id: &finish.request_id,
                status: "available",
                http_status: Some(status.as_u16() as i64),
                ttft_ms: Some(elapsed_ms(finish.attempt_started)),
                total_latency_ms: Some(elapsed_ms(finish.attempt_started)),
                error: None,
            },
        )
        .await;
    }
    settle_success(&finish, usage).await?;
    Ok(FinishOutcome::Response(
        (status, Json(body)).into_response(),
    ))
}

async fn settle_success(finish: &FinishContext, usage: Usage) -> AppResult<()> {
    if let Err(err) = enqueue_ledger(
        LedgerContext {
            state: &finish.state,
            auth: &finish.auth,
            api_key: &finish.api_key,
            decision: &finish.decision,
            request: &finish.request,
            request_id: &finish.request_id,
            price: finish.price.clone(),
            reservation: &finish.reservation,
            surge_multiplier: finish.surge_multiplier,
        },
        normalized_usage(&finish.request, usage),
        "success",
    )
    .await
    {
        finish
            .state
            .db
            .release_gateway_reservation(&finish.reservation)
            .await?;
        return Err(err);
    }
    if let Some(hit) = &finish.decision.affinity_hit
        && hit.rule.switch_on_success
    {
        remember_affinity(
            &finish.state.db,
            &finish.state.affinity_cache,
            hit,
            finish.decision.channel.id,
        )
        .await?;
    }
    Ok(())
}

async fn finish_streaming_response(
    finish: FinishContext,
    upstream: reqwest::Response,
) -> AppResult<FinishOutcome> {
    let status = upstream.status();
    let provider_protocol = provider_protocol(&finish.decision.channel.provider);
    if !status.is_success() {
        let error = format!("HTTP {status}");
        record_channel_health(
            &finish.state,
            &finish.decision,
            GatewayHealthEvent {
                request_id: &finish.request_id,
                status: if status == StatusCode::TOO_MANY_REQUESTS {
                    "degraded"
                } else {
                    "down"
                },
                http_status: Some(status.as_u16() as i64),
                ttft_ms: None,
                total_latency_ms: Some(elapsed_ms(finish.attempt_started)),
                error: Some(error.as_str()),
            },
        )
        .await;
        finish
            .state
            .db
            .release_gateway_reservation(&finish.reservation)
            .await?;
    }
    let mut stream = upstream.bytes_stream();
    let mut usage = Usage {
        input_tokens: 0,
        output_tokens: 0,
        cache_tokens: 0,
    };
    let mut buffered = Vec::new();
    if status.is_success() {
        let mut saw_semantic = false;
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(bytes) => {
                    merge_usage_from_sse(&bytes, &mut usage);
                    saw_semantic = stream_chunk_has_semantic_content(&bytes, provider_protocol);
                    let done = stream_chunk_has_done_marker(&bytes);
                    buffered.push(bytes);
                    if saw_semantic || done {
                        break;
                    }
                }
                Err(err) => {
                    let error = err.to_string();
                    record_channel_health(
                        &finish.state,
                        &finish.decision,
                        GatewayHealthEvent {
                            request_id: &finish.request_id,
                            status: "down",
                            http_status: None,
                            ttft_ms: None,
                            total_latency_ms: Some(elapsed_ms(finish.attempt_started)),
                            error: Some(error.as_str()),
                        },
                    )
                    .await;
                    return retryable_empty_finish(
                        &finish,
                        stream_error_before_content(finish.decision.channel.id, err),
                    )
                    .await;
                }
            }
        }
        if !saw_semantic {
            record_channel_health(
                &finish.state,
                &finish.decision,
                GatewayHealthEvent {
                    request_id: &finish.request_id,
                    status: "empty",
                    http_status: Some(status.as_u16() as i64),
                    ttft_ms: None,
                    total_latency_ms: Some(elapsed_ms(finish.attempt_started)),
                    error: Some("semantic empty stream"),
                },
            )
            .await;
            return retryable_empty_finish(&finish, empty_stream_error(finish.decision.channel.id))
                .await;
        }
    }
    if status.is_success() {
        record_channel_health(
            &finish.state,
            &finish.decision,
            GatewayHealthEvent {
                request_id: &finish.request_id,
                status: "available",
                http_status: Some(status.as_u16() as i64),
                ttft_ms: Some(elapsed_ms(finish.attempt_started)),
                total_latency_ms: Some(elapsed_ms(finish.attempt_started)),
                error: None,
            },
        )
        .await;
    }
    let finish_for_stream = finish.clone();

    let output = async_stream::stream! {
        for bytes in buffered {
            let bytes = translate_stream_chunk(
                bytes,
                provider_protocol,
                finish_for_stream.client_protocol,
                &finish_for_stream.request.model,
            );
            yield Ok::<Bytes, std::io::Error>(bytes);
        }
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(bytes) => {
                    merge_usage_from_sse(&bytes, &mut usage);
                    let bytes = translate_stream_chunk(
                        bytes,
                        provider_protocol,
                        finish_for_stream.client_protocol,
                        &finish_for_stream.request.model,
                    );
                    yield Ok::<Bytes, std::io::Error>(bytes);
                }
                Err(err) => {
                    yield Ok::<Bytes, std::io::Error>(Bytes::from(format!("event: error\ndata: {err}\n\n")));
                    break;
                }
            }
        }
        if status.is_success() {
            let final_usage = normalized_usage(&finish_for_stream.request, usage.clone());
            if enqueue_ledger(
                LedgerContext {
                    state: &finish_for_stream.state,
                    auth: &finish_for_stream.auth,
                    api_key: &finish_for_stream.api_key,
                    decision: &finish_for_stream.decision,
                    request: &finish_for_stream.request,
                    request_id: &finish_for_stream.request_id,
                    price: finish_for_stream.price.clone(),
                    reservation: &finish_for_stream.reservation,
                    surge_multiplier: finish_for_stream.surge_multiplier,
                },
                final_usage,
                "success",
            )
            .await
            .is_err()
            {
                let _ = finish_for_stream
                    .state
                    .db
                    .release_gateway_reservation(&finish_for_stream.reservation)
                    .await;
            }
            if let Some(hit) = &finish_for_stream.decision.affinity_hit
                && hit.rule.switch_on_success
            {
                let _ = remember_affinity(
                    &finish_for_stream.state.db,
                    &finish_for_stream.state.affinity_cache,
                    hit,
                    finish_for_stream.decision.channel.id,
                ).await;
            }
        }
    };

    let mut response = Response::new(Body::from_stream(output));
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("text/event-stream"),
    );
    Ok(FinishOutcome::Response(response))
}

async fn enqueue_ledger(ctx: LedgerContext<'_>, usage: Usage, status: &str) -> AppResult<()> {
    let settings = ctx.state.db.runtime_settings().await?;
    let surge_multiplier = ctx.surge_multiplier;
    let discount = fire_sale_discount(&ctx.decision.channel);
    let settlement = settle(
        &usage,
        &ctx.price,
        surge_multiplier,
        discount,
        ctx.decision.channel.limits.provider_share,
        &settings,
    );
    let event = LedgerEvent {
        request_id: ctx.request_id.to_string(),
        user_id: ctx.auth.user.id,
        api_key_id: ctx.api_key.id,
        channel_id: ctx.decision.channel.id,
        provider_user_id: ctx.decision.channel.owner_user_id,
        model: ctx.request.model.clone(),
        tokenizer: crate::tokenizer::estimate_request_tokens(ctx.request).tokenizer,
        usage,
        price: ctx.price,
        surge_multiplier,
        fire_sale_discount: discount,
        total_points: settlement.total_points,
        provider_points: settlement.provider_points,
        status: status.to_string(),
        formula_note: settlement.formula_note,
        reservation: ctx.reservation.clone(),
    };
    ctx.state
        .ledger_tx
        .send(event)
        .await
        .map_err(|err| AppError::Anyhow(anyhow::anyhow!(err.to_string())))?;
    Ok(())
}

pub async fn surge_multiplier(state: &AppState) -> (f64, &'static str) {
    let settings = state.db.runtime_settings().await.unwrap_or_else(|_| {
        RuntimeSettings::from_map(&crate::settings::default_map())
            .expect("built-in runtime settings defaults are valid")
    });
    let channels = state.db.list_route_channels().await.unwrap_or_default();
    surge_multiplier_from_channels(
        &settings,
        &channels,
        state.metrics.tokens_last_hour(),
        Utc::now(),
    )
}

fn surge_multiplier_from_channels(
    settings: &RuntimeSettings,
    channels: &[Channel],
    tokens_last_hour: i64,
    now: DateTime<Utc>,
) -> (f64, &'static str) {
    let hourly_capacity: f64 = channels
        .iter()
        .filter(|channel| channel.enabled && channel.status == "healthy")
        .filter_map(|channel| {
            channel
                .limits
                .windows
                .first()
                .and_then(|window| primary_window_remaining_tokens_per_hour(window, now))
        })
        .sum();
    if hourly_capacity <= f64::EPSILON {
        return (settings.surge_normal_multiplier, "no_capacity");
    }

    // Compare one-hour demand to a one-hour supply rate. Raw remaining inventory is not a
    // time-compatible denominator for the rolling one-hour token window.
    let ratio = tokens_last_hour as f64 / hourly_capacity;
    if ratio < settings.surge_low_threshold {
        (settings.surge_idle_multiplier, "idle")
    } else if ratio > settings.surge_high_threshold {
        (settings.surge_peak_multiplier, "peak")
    } else {
        (settings.surge_normal_multiplier, "normal")
    }
}

fn primary_window_remaining_tokens_per_hour(
    window: &ChannelQuotaWindow,
    now: DateTime<Utc>,
) -> Option<f64> {
    let remaining_tokens = window.limit_tokens - window.used_tokens;
    if remaining_tokens <= 0 {
        return None;
    }
    let end_at = DateTime::parse_from_rfc3339(&window.current_window_end_at)
        .ok()?
        .with_timezone(&Utc);
    let remaining_seconds = end_at.signed_duration_since(now).num_seconds();
    if remaining_seconds <= 0 {
        return None;
    }
    Some(remaining_tokens as f64 * SECONDS_PER_HOUR / remaining_seconds as f64)
}

fn normalized_usage(request: &crate::protocol::TextRequest, usage: Usage) -> Usage {
    if usage.total() > 0 {
        usage
    } else {
        Usage {
            input_tokens: request.estimated_input_tokens(),
            output_tokens: 0,
            cache_tokens: 0,
        }
    }
}

fn merge_usage_from_sse(bytes: &Bytes, usage: &mut Usage) {
    let text = String::from_utf8_lossy(bytes);
    for line in text.lines() {
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data == "[DONE]" || data.is_empty() {
            continue;
        }
        if let Ok(value) = serde_json::from_str::<Value>(data) {
            let parsed = extract_usage(&value);
            if parsed.input_tokens > 0 || parsed.output_tokens > 0 || parsed.cache_tokens > 0 {
                *usage = parsed;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Duration as ChronoDuration, TimeZone};

    use super::*;

    fn default_settings() -> RuntimeSettings {
        RuntimeSettings::from_map(&crate::settings::default_map())
            .expect("built-in runtime settings defaults are valid")
    }

    fn healthy_channel_with_primary_window(
        remaining_tokens: i64,
        window_seconds_left: i64,
    ) -> Channel {
        let now = Utc.with_ymd_and_hms(2026, 5, 19, 12, 0, 0).unwrap();
        Channel {
            id: 1,
            owner_user_id: 1,
            name: "primary".to_string(),
            provider: ProviderKind::OpenAi,
            base_url: "http://example.test".to_string(),
            api_key_secret: "secret".to_string(),
            models: vec!["*".to_string()],
            enabled: true,
            status: "healthy".to_string(),
            health_checked_at: None,
            upstream_latency_ms: None,
            last_error: None,
            limits: crate::models::ChannelLimits {
                windows: vec![ChannelQuotaWindow {
                    id: 1,
                    name: "Minute".to_string(),
                    limit_tokens: 1_000,
                    used_tokens: 1_000 - remaining_tokens,
                    period_unit: "minute".to_string(),
                    period_count: 1,
                    anchor_at: "2026-05-19T12:00:00Z".to_string(),
                    timezone: "UTC".to_string(),
                    current_window_start_at: now.to_rfc3339(),
                    current_window_end_at: (now + ChronoDuration::seconds(window_seconds_left))
                        .to_rfc3339(),
                    sort_order: 0,
                }],
                fire_sale_days_before: 3,
                fire_sale_remaining_pct: 0.25,
                fire_sale_discount: 0.2,
                provider_share: 0.7,
            },
        }
    }

    #[test]
    fn surge_uses_hourly_capacity_not_raw_remaining_inventory() {
        let settings = default_settings();
        let now = Utc.with_ymd_and_hms(2026, 5, 19, 12, 0, 0).unwrap();
        let channel = healthy_channel_with_primary_window(10, 60);

        let (multiplier, state) = surge_multiplier_from_channels(&settings, &[channel], 100, now);

        assert_eq!(state, "idle");
        assert_eq!(multiplier, settings.surge_idle_multiplier);
    }

    #[test]
    fn surge_reports_no_capacity_without_peak_pricing() {
        let settings = default_settings();
        let now = Utc.with_ymd_and_hms(2026, 5, 19, 12, 0, 0).unwrap();

        let (multiplier, state) = surge_multiplier_from_channels(&settings, &[], 0, now);

        assert_eq!(state, "no_capacity");
        assert_eq!(multiplier, settings.surge_normal_multiplier);
    }
}

#[allow(dead_code)]
fn _same_protocol(client: ClientProtocol, provider: ProviderProtocol) -> bool {
    same_wire_protocol(client, provider)
}
