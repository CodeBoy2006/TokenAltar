use std::time::Duration;

use axum::{
    Json,
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use futures_util::StreamExt;
use serde_json::Value;
use uuid::Uuid;

use crate::{
    affinity::{lookup_affinity, remember_affinity},
    app::AppState,
    auth::GatewayAuth,
    error::{AppError, AppResult},
    models::{GatewayContext, LedgerEvent, ProviderKind, Usage},
    pricing::{fire_sale_discount, select_price, settle},
    protocol::{
        ClientProtocol, ProviderProtocol, client_response_body, extract_usage,
        parse_client_request, provider_protocol, same_wire_protocol, translate_stream_chunk,
        upstream_body, upstream_path,
    },
    routing::{RouteDecision, choose_channel},
};

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
    let mut parse_body = raw_body.clone();
    if let Some(model) = endpoint.path_model {
        parse_body["_model"] = Value::String(model);
    }
    if let Some(stream) = endpoint.path_stream {
        parse_body["_stream"] = Value::Bool(stream);
    }
    let request = parse_client_request(endpoint.client_protocol, &parse_body)?;
    let api_key = auth.api_key.clone().ok_or(AppError::Unauthorized)?;
    state.db.refresh_channel_windows().await?;
    let global_prices = state.db.global_price_book().await?;
    let reserve_price = select_price(&request.model, &global_prices);
    let token_estimate = crate::tokenizer::estimate_request_tokens(&request);
    let reserve = token_estimate.tokens as f64 * reserve_price.input_price_per_1k / 1000.0;
    ensure_affordable(&auth.user, &api_key, reserve)?;

    let channels = state.db.list_route_channels().await?;
    let gateway_context = GatewayContext::default();
    let affinity_hit = lookup_affinity(
        &state.db,
        &state.affinity_cache,
        endpoint.request_path,
        &headers,
        &raw_body,
        &request,
        &gateway_context,
    )
    .await?;
    let decision =
        choose_channel(&channels, &request.model, affinity_hit, &state.router_state).await?;
    let price = select_price(
        &request.model,
        &state.db.price_book_for_channel(decision.channel.id).await?,
    );
    let selected_reserve = token_estimate.tokens as f64 * price.input_price_per_1k / 1000.0;
    ensure_affordable(&auth.user, &api_key, selected_reserve)?;

    let request_id = Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string();
    let upstream = send_upstream(
        &state,
        &decision,
        endpoint.client_protocol,
        &raw_body,
        &request,
    )
    .await?;

    if upstream.status() == StatusCode::TOO_MANY_REQUESTS {
        state
            .router_state
            .mark_cooldown(decision.channel.id, Duration::from_secs(30))
            .await;
        let retry = choose_channel(
            &channels,
            &request.model,
            decision.affinity_hit.clone(),
            &state.router_state,
        )
        .await?;
        let retry_price = select_price(
            &request.model,
            &state.db.price_book_for_channel(retry.channel.id).await?,
        );
        let retry_response = send_upstream(
            &state,
            &retry,
            endpoint.client_protocol,
            &raw_body,
            &request,
        )
        .await?;
        return finish_response(
            FinishContext {
                state,
                auth,
                api_key,
                decision: retry,
                request,
                client_protocol: endpoint.client_protocol,
                request_id,
                price: retry_price,
            },
            retry_response,
        )
        .await;
    }

    finish_response(
        FinishContext {
            state,
            auth,
            api_key,
            decision,
            request,
            client_protocol: endpoint.client_protocol,
            request_id,
            price,
        },
        upstream,
    )
    .await
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
    let body = upstream_body(client_protocol, provider_protocol, raw_body, request);
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
}

struct LedgerContext<'a> {
    state: &'a AppState,
    auth: &'a crate::models::AuthContext,
    api_key: &'a crate::models::ApiKeyRecord,
    decision: &'a RouteDecision,
    request: &'a crate::protocol::TextRequest,
    request_id: &'a str,
    price: crate::models::ModelPrice,
}

async fn finish_response(
    finish: FinishContext,
    upstream: reqwest::Response,
) -> AppResult<Response> {
    let status = upstream.status();
    if finish.request.stream {
        return finish_streaming_response(finish, upstream).await;
    }

    let provider_protocol = provider_protocol(&finish.decision.channel.provider);
    let value = upstream
        .json::<Value>()
        .await
        .map_err(|err| AppError::Upstream(err.to_string()))?;
    if !status.is_success() {
        return Ok((status, Json(value)).into_response());
    }
    let (body, usage) = client_response_body(finish.client_protocol, provider_protocol, value);
    settle_success(&finish, usage).await?;
    Ok((status, Json(body)).into_response())
}

async fn settle_success(finish: &FinishContext, usage: Usage) -> AppResult<()> {
    enqueue_ledger(
        LedgerContext {
            state: &finish.state,
            auth: &finish.auth,
            api_key: &finish.api_key,
            decision: &finish.decision,
            request: &finish.request,
            request_id: &finish.request_id,
            price: finish.price.clone(),
        },
        normalized_usage(&finish.request, usage),
        "success",
    )
    .await?;
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
) -> AppResult<Response> {
    let status = upstream.status();
    let provider_protocol = provider_protocol(&finish.decision.channel.provider);
    let mut stream = upstream.bytes_stream();
    let mut usage = Usage {
        input_tokens: 0,
        output_tokens: 0,
        cache_tokens: 0,
    };
    let finish_for_stream = finish.clone();

    let output = async_stream::stream! {
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
        let final_usage = normalized_usage(&finish_for_stream.request, usage.clone());
        let _ = enqueue_ledger(
            LedgerContext {
                state: &finish_for_stream.state,
                auth: &finish_for_stream.auth,
                api_key: &finish_for_stream.api_key,
                decision: &finish_for_stream.decision,
                request: &finish_for_stream.request,
                request_id: &finish_for_stream.request_id,
                price: finish_for_stream.price.clone(),
            },
            final_usage,
            if status.is_success() { "success" } else { "upstream_error" },
        ).await;
        if status.is_success()
            && let Some(hit) = &finish_for_stream.decision.affinity_hit
            && hit.rule.switch_on_success
        {
            let _ = remember_affinity(
                &finish_for_stream.state.db,
                &finish_for_stream.state.affinity_cache,
                hit,
                finish_for_stream.decision.channel.id,
            ).await;
        }
    };

    let mut response = Response::new(Body::from_stream(output));
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("text/event-stream"),
    );
    Ok(response)
}

async fn enqueue_ledger(ctx: LedgerContext<'_>, usage: Usage, status: &str) -> AppResult<()> {
    let surge_multiplier = surge_multiplier(ctx.state).await.0;
    let discount = fire_sale_discount(&ctx.decision.channel);
    let settlement = settle(
        &usage,
        &ctx.price,
        surge_multiplier,
        discount,
        ctx.decision.channel.limits.provider_share,
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
    };
    ctx.state
        .ledger_tx
        .send(event)
        .await
        .map_err(|err| AppError::Anyhow(anyhow::anyhow!(err.to_string())))?;
    Ok(())
}

pub async fn surge_multiplier(state: &AppState) -> (f64, &'static str) {
    let channels = state.db.list_route_channels().await.unwrap_or_default();
    let total_available: i64 = channels
        .iter()
        .map(|channel| channel.limits.cycle_limit_tokens - channel.limits.used_cycle_tokens)
        .sum();
    if total_available <= 0 {
        return (1.5, "peak");
    }
    let ratio = state.metrics.tokens_last_hour() as f64 / total_available as f64;
    if ratio < 0.30 {
        (0.5, "idle")
    } else if ratio > 0.80 {
        (1.5, "peak")
    } else {
        (1.0, "normal")
    }
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

#[allow(dead_code)]
fn _same_protocol(client: ClientProtocol, provider: ProviderProtocol) -> bool {
    same_wire_protocol(client, provider)
}
