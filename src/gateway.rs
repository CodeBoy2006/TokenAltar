use std::time::Duration;

use axum::{
    Json,
    body::Body,
    extract::State,
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
    pricing::{fire_sale_discount, reserve_cost, select_price, settle},
    protocol::{
        ClientProtocol, extract_usage, general_to_anthropic_messages, general_to_openai_responses,
        parse_anthropic_messages, parse_openai_responses, response_to_anthropic, response_to_openai,
    },
    routing::{RouteDecision, choose_channel},
};

pub async fn openai_responses(
    State(state): State<AppState>,
    GatewayAuth(auth): GatewayAuth,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> AppResult<Response> {
    let request = parse_openai_responses(body.clone())?;
    handle_gateway(
        state,
        auth,
        headers,
        body,
        request,
        ClientProtocol::OpenAiResponses,
        "/v1/responses",
    )
    .await
}

pub async fn anthropic_messages(
    State(state): State<AppState>,
    GatewayAuth(auth): GatewayAuth,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> AppResult<Response> {
    let request = parse_anthropic_messages(body.clone())?;
    handle_gateway(
        state,
        auth,
        headers,
        body,
        request,
        ClientProtocol::AnthropicMessages,
        "/v1/messages",
    )
    .await
}

async fn handle_gateway(
    state: AppState,
    auth: crate::models::AuthContext,
    headers: HeaderMap,
    raw_body: Value,
    request: crate::protocol::GeneralOpenAIRequest,
    client_protocol: ClientProtocol,
    request_path: &str,
) -> AppResult<Response> {
    let api_key = auth.api_key.clone().ok_or(AppError::Unauthorized)?;
    let prices = state.db.list_prices().await?;
    let price = select_price(&request.model, &prices);
    let reserve = reserve_cost(&serde_json::to_string(&raw_body).unwrap_or_default(), &price);
    if auth.user.points_balance < reserve {
        return Err(AppError::BadRequest("insufficient points for estimated input tokens".to_string()));
    }
    if let Some(limit) = api_key.spend_limit_points
        && api_key.spent_points + reserve > limit
    {
        return Err(AppError::BadRequest("api key spend limit would be exceeded".to_string()));
    }

    let channels = state.db.list_channels().await?;
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
    let decision = choose_channel(
        &channels,
        &request.model,
        affinity_hit,
        &state.router_state,
    )
    .await?;

    let request_id = Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string();
    let stream = request.stream;
    let upstream = send_upstream(&state, &decision, &request, stream).await?;

    if upstream.status() == StatusCode::TOO_MANY_REQUESTS {
        state
            .router_state
            .mark_cooldown(decision.channel.id, Duration::from_secs(30))
            .await;
        let retry = choose_channel(&channels, &request.model, decision.affinity_hit.clone(), &state.router_state).await?;
        let retry_response = send_upstream(&state, &retry, &request, stream).await?;
        let finish = FinishContext {
            state,
            auth,
            api_key,
            decision: retry,
            request,
            client_protocol,
            request_id,
            price,
        };
        return finish_response(finish, retry_response)
        .await;
    }

    finish_response(FinishContext {
        state,
        auth,
        api_key,
        decision,
        request,
        client_protocol,
        request_id,
        price,
    }, upstream)
    .await
}

async fn send_upstream(
    state: &AppState,
    decision: &RouteDecision,
    request: &crate::protocol::GeneralOpenAIRequest,
    stream: bool,
) -> AppResult<reqwest::Response> {
    let (path, body) = match decision.channel.provider {
        ProviderKind::OpenAi => (
            "/v1/responses",
            general_to_openai_responses(request, stream),
        ),
        ProviderKind::Anthropic => (
            "/v1/messages",
            general_to_anthropic_messages(request, stream),
        ),
    };
    let url = format!("{}{}", decision.channel.base_url.trim_end_matches('/'), path);
    let mut builder = state
        .http
        .post(url)
        .bearer_auth(&decision.channel.api_key_secret)
        .json(&body);
    if decision.channel.provider == ProviderKind::Anthropic {
        builder = builder
            .header("anthropic-version", "2023-06-01")
            .header("x-api-key", &decision.channel.api_key_secret);
    }
    builder.send().await.map_err(|err| AppError::Upstream(err.to_string()))
}

#[derive(Clone)]
struct FinishContext {
    state: AppState,
    auth: crate::models::AuthContext,
    api_key: crate::models::ApiKeyRecord,
    decision: RouteDecision,
    request: crate::protocol::GeneralOpenAIRequest,
    client_protocol: ClientProtocol,
    request_id: String,
    price: crate::models::ModelPrice,
}

struct LedgerContext<'a> {
    state: &'a AppState,
    auth: &'a crate::models::AuthContext,
    api_key: &'a crate::models::ApiKeyRecord,
    decision: &'a RouteDecision,
    request: &'a crate::protocol::GeneralOpenAIRequest,
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

    let value = upstream
        .json::<Value>()
        .await
        .map_err(|err| AppError::Upstream(err.to_string()))?;
    if !status.is_success() {
        return Ok((status, Json(value)).into_response());
    }
    let (body, usage) = match finish.client_protocol {
        ClientProtocol::OpenAiResponses => response_to_openai(value),
        ClientProtocol::AnthropicMessages => response_to_anthropic(value),
    };
    enqueue_ledger(
        LedgerContext {
            state: &finish.state,
            auth: &finish.auth,
            api_key: &finish.api_key,
            decision: &finish.decision,
            request: &finish.request,
            request_id: &finish.request_id,
            price: finish.price,
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
    Ok((status, Json(body)).into_response())
}

async fn finish_streaming_response(
    finish: FinishContext,
    upstream: reqwest::Response,
) -> AppResult<Response> {
    let status = upstream.status();
    let mut stream = upstream.bytes_stream();
    let mut usage = Usage {
        input_tokens: 0,
        output_tokens: 0,
        cache_tokens: 0,
    };
    let mut buffer = Vec::<u8>::new();

    let finish_for_stream = finish.clone();

    let output = async_stream::stream! {
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(bytes) => {
                    merge_usage_from_sse(&bytes, &mut usage);
                    buffer.extend_from_slice(&bytes);
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
        let _ = buffer;
        let _ = &finish_for_stream.client_protocol;
    };

    let mut response = Response::new(Body::from_stream(output));
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("text/event-stream"),
    );
    Ok(response)
}

async fn enqueue_ledger(
    ctx: LedgerContext<'_>,
    usage: Usage,
    status: &str,
) -> AppResult<()> {
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
        tokenizer: "tokenaltar-local-estimator".to_string(),
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
    let channels = state.db.list_channels().await.unwrap_or_default();
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

fn normalized_usage(request: &crate::protocol::GeneralOpenAIRequest, usage: Usage) -> Usage {
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
