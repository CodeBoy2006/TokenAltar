use std::{collections::BTreeSet, time::Instant};

use axum::{
    Json,
    extract::{Path, Query, State},
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    auth::{ConsoleAuth, require_admin, verify_password},
    db::{
        AffinityRuleInput, ApiKeyUpdateInput, ChannelHealthEventInput, ChannelInput,
        ChannelUpdateInput, ManagedUserCreateInput, ManagedUserUpdateInput, PasswordResetInput,
        SettingUpdate,
    },
    error::{AppError, AppResult},
    events::{
        TOPIC_AFFINITY_RULES, TOPIC_API_KEYS, TOPIC_CHANNELS, TOPIC_DASHBOARD, TOPIC_LEADERBOARDS,
        TOPIC_ME, TOPIC_PRICES, TOPIC_REDPACKETS, TOPIC_RUNTIME_SETTINGS, TOPIC_SETTINGS,
        TOPIC_TRANSFERS, TOPIC_USERS, publish_admin_event, publish_channel_owner_event,
        publish_global_event, publish_user_event,
    },
    gateway::surge_multiplier,
    models::{ModelPrice, User},
};

#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    pub email: String,
    pub password: String,
    pub display_name: Option<String>,
    pub invite_code: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct AuthResponse {
    pub token: String,
    pub user: User,
}

#[derive(Debug, Deserialize)]
pub struct CreateApiKeyRequest {
    pub name: String,
    pub enabled: Option<bool>,
    pub spend_limit_points: Option<f64>,
    pub allowed_models: Option<Vec<String>>,
    pub allowed_channel_ids: Option<Vec<i64>>,
    pub expires_at: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct EnabledRequest {
    pub enabled: bool,
}

#[derive(Debug, Deserialize)]
pub struct BatchIdsRequest {
    pub ids: Vec<i64>,
}

#[derive(Debug, Deserialize)]
pub struct ChannelBatchEnabledRequest {
    pub ids: Vec<i64>,
    pub enabled: bool,
}

#[derive(Debug, Deserialize)]
pub struct CopyChannelRequest {
    pub suffix: Option<String>,
    pub reset_usage: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct TransferRequest {
    pub to_user_id: i64,
    pub points: f64,
    pub memo: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RedPacketRequest {
    pub phrase: String,
    pub total_points: f64,
    pub total_parts: i64,
    pub mode: String,
}

#[derive(Debug, Deserialize)]
pub struct ClaimRedPacketRequest {
    pub phrase: String,
}

#[derive(Debug, Deserialize)]
pub struct LeaderboardQuery {
    pub period: Option<String>,
}

pub async fn register(
    State(state): State<crate::app::AppState>,
    Json(request): Json<RegisterRequest>,
) -> AppResult<Json<AuthResponse>> {
    let invite_required = sqlx::query_scalar::<_, String>(
        "SELECT value FROM system_settings WHERE key = 'invite_required'",
    )
    .fetch_optional(&state.db.pool)
    .await?
    .unwrap_or_else(|| "false".to_string());
    if invite_required == "true" {
        let Some(code) = request.invite_code.as_deref() else {
            return Err(AppError::Forbidden);
        };
        let accepted = state.db.consume_invite_code(code).await?;
        if !accepted {
            return Err(AppError::Forbidden);
        }
    }
    let display_name = request.display_name.unwrap_or_else(|| {
        request
            .email
            .split('@')
            .next()
            .unwrap_or("user")
            .to_string()
    });
    let user = state
        .db
        .create_user(&request.email, &request.password, &display_name)
        .await?;
    let token = state.db.create_session(user.id).await?;
    publish_admin_event(&state, [TOPIC_USERS, TOPIC_DASHBOARD]);
    Ok(Json(AuthResponse { token, user }))
}

pub async fn login(
    State(state): State<crate::app::AppState>,
    Json(request): Json<LoginRequest>,
) -> AppResult<Json<AuthResponse>> {
    let Some((user, password_hash)) = state.db.find_user_with_hash(&request.email).await? else {
        return Err(AppError::Unauthorized);
    };
    if !user.enabled {
        return Err(AppError::Unauthorized);
    }
    if !verify_password(&request.password, &password_hash) {
        return Err(AppError::Unauthorized);
    }
    let token = state.db.create_session(user.id).await?;
    Ok(Json(AuthResponse { token, user }))
}

pub async fn me(ConsoleAuth(auth): ConsoleAuth) -> AppResult<Json<User>> {
    Ok(Json(auth.user))
}

pub async fn list_users(
    State(state): State<crate::app::AppState>,
    ConsoleAuth(auth): ConsoleAuth,
) -> AppResult<Json<serde_json::Value>> {
    require_admin(&auth.user)?;
    Ok(Json(json!(state.db.list_managed_users().await?)))
}

pub async fn create_user(
    State(state): State<crate::app::AppState>,
    ConsoleAuth(auth): ConsoleAuth,
    Json(request): Json<ManagedUserCreateInput>,
) -> AppResult<Json<serde_json::Value>> {
    require_admin(&auth.user)?;
    let user = state.db.create_managed_user(request).await?;
    publish_admin_event(&state, [TOPIC_USERS, TOPIC_DASHBOARD]);
    Ok(Json(json!(user)))
}

pub async fn update_user(
    State(state): State<crate::app::AppState>,
    ConsoleAuth(auth): ConsoleAuth,
    Path(id): Path<i64>,
    Json(request): Json<ManagedUserUpdateInput>,
) -> AppResult<Json<serde_json::Value>> {
    require_admin(&auth.user)?;
    let user = state
        .db
        .update_managed_user(auth.user.id, id, request)
        .await?;
    publish_admin_event(&state, [TOPIC_USERS, TOPIC_DASHBOARD, TOPIC_CHANNELS]);
    publish_user_event(
        &state,
        user.id,
        [TOPIC_ME, TOPIC_DASHBOARD, TOPIC_API_KEYS, TOPIC_CHANNELS],
    );
    Ok(Json(json!(user)))
}

pub async fn set_user_enabled(
    State(state): State<crate::app::AppState>,
    ConsoleAuth(auth): ConsoleAuth,
    Path(id): Path<i64>,
    Json(request): Json<EnabledRequest>,
) -> AppResult<Json<serde_json::Value>> {
    require_admin(&auth.user)?;
    let user = state
        .db
        .set_user_enabled(auth.user.id, id, request.enabled)
        .await?;
    publish_admin_event(&state, [TOPIC_USERS, TOPIC_DASHBOARD, TOPIC_CHANNELS]);
    publish_user_event(
        &state,
        user.id,
        [TOPIC_ME, TOPIC_DASHBOARD, TOPIC_API_KEYS, TOPIC_CHANNELS],
    );
    Ok(Json(json!(user)))
}

pub async fn reset_user_password(
    State(state): State<crate::app::AppState>,
    ConsoleAuth(auth): ConsoleAuth,
    Path(id): Path<i64>,
    Json(request): Json<PasswordResetInput>,
) -> AppResult<Json<serde_json::Value>> {
    require_admin(&auth.user)?;
    state.db.reset_user_password(id, &request.password).await?;
    Ok(Json(json!({ "ok": true })))
}

pub async fn create_api_key(
    State(state): State<crate::app::AppState>,
    ConsoleAuth(auth): ConsoleAuth,
    Json(request): Json<CreateApiKeyRequest>,
) -> AppResult<Json<serde_json::Value>> {
    let (token, record) = state
        .db
        .create_api_key(auth.user.id, &request.name, request.spend_limit_points)
        .await?;
    let record = if request.enabled.is_some()
        || request.allowed_models.is_some()
        || request.allowed_channel_ids.is_some()
        || request.expires_at.is_some()
    {
        let update = ApiKeyUpdateInput {
            name: record.name.clone(),
            enabled: request.enabled.unwrap_or(record.enabled),
            spend_limit_points: record.spend_limit_points,
            expires_at: request.expires_at,
            allowed_models: request.allowed_models.unwrap_or_default(),
            allowed_channel_ids: request
                .allowed_channel_ids
                .unwrap_or_else(|| record.allowed_channel_ids.clone()),
        };
        state
            .db
            .update_api_key(auth.user.id, record.id, update)
            .await?
    } else {
        record
    };
    publish_user_event(&state, auth.user.id, [TOPIC_ME, TOPIC_API_KEYS]);
    publish_admin_event(&state, [TOPIC_USERS, TOPIC_DASHBOARD, TOPIC_API_KEYS]);
    Ok(Json(json!({ "token": token, "record": record })))
}

pub async fn list_api_keys(
    State(state): State<crate::app::AppState>,
    ConsoleAuth(auth): ConsoleAuth,
) -> AppResult<Json<serde_json::Value>> {
    let keys = state.db.list_api_keys(auth.user.id).await?;
    Ok(Json(json!(keys)))
}

pub async fn set_api_key_enabled(
    State(state): State<crate::app::AppState>,
    ConsoleAuth(auth): ConsoleAuth,
    Path(id): Path<i64>,
    Json(request): Json<EnabledRequest>,
) -> AppResult<Json<serde_json::Value>> {
    state
        .db
        .set_api_key_enabled(auth.user.id, id, request.enabled)
        .await?;
    publish_user_event(&state, auth.user.id, [TOPIC_ME, TOPIC_API_KEYS]);
    Ok(Json(json!({ "ok": true })))
}

pub async fn update_api_key(
    State(state): State<crate::app::AppState>,
    ConsoleAuth(auth): ConsoleAuth,
    Path(id): Path<i64>,
    Json(request): Json<ApiKeyUpdateInput>,
) -> AppResult<Json<serde_json::Value>> {
    let record = state.db.update_api_key(auth.user.id, id, request).await?;
    publish_user_event(&state, auth.user.id, [TOPIC_ME, TOPIC_API_KEYS]);
    Ok(Json(json!(record)))
}

pub async fn rotate_api_key(
    State(state): State<crate::app::AppState>,
    ConsoleAuth(auth): ConsoleAuth,
    Path(id): Path<i64>,
) -> AppResult<Json<serde_json::Value>> {
    let (token, record) = state.db.rotate_api_key(auth.user.id, id).await?;
    publish_user_event(&state, auth.user.id, [TOPIC_ME, TOPIC_API_KEYS]);
    Ok(Json(json!({ "token": token, "record": record })))
}

pub async fn delete_api_key(
    State(state): State<crate::app::AppState>,
    ConsoleAuth(auth): ConsoleAuth,
    Path(id): Path<i64>,
) -> AppResult<Json<serde_json::Value>> {
    state.db.delete_api_key(auth.user.id, id).await?;
    publish_user_event(&state, auth.user.id, [TOPIC_ME, TOPIC_API_KEYS]);
    Ok(Json(json!({ "ok": true })))
}

pub async fn batch_delete_api_keys(
    State(state): State<crate::app::AppState>,
    ConsoleAuth(auth): ConsoleAuth,
    Json(request): Json<BatchIdsRequest>,
) -> AppResult<Json<serde_json::Value>> {
    let count = state
        .db
        .batch_delete_api_keys(auth.user.id, &request.ids)
        .await?;
    publish_user_event(&state, auth.user.id, [TOPIC_ME, TOPIC_API_KEYS]);
    Ok(Json(json!({ "deleted": count })))
}

pub async fn list_channels(
    State(state): State<crate::app::AppState>,
    ConsoleAuth(auth): ConsoleAuth,
) -> AppResult<Json<serde_json::Value>> {
    state.db.refresh_channel_windows().await?;
    Ok(Json(json!(
        state.db.list_public_channels(&auth.user).await?
    )))
}

pub async fn list_route_channel_options(
    State(state): State<crate::app::AppState>,
    ConsoleAuth(_auth): ConsoleAuth,
) -> AppResult<Json<serde_json::Value>> {
    state.db.refresh_channel_windows().await?;
    Ok(Json(json!(state.db.list_public_route_channels().await?)))
}

pub async fn create_channel(
    State(state): State<crate::app::AppState>,
    ConsoleAuth(auth): ConsoleAuth,
    Json(request): Json<ChannelInput>,
) -> AppResult<Json<serde_json::Value>> {
    let channel = state.db.upsert_channel(auth.user.id, request).await?;
    publish_channel_owner_event(
        &state,
        channel.owner_user_id,
        [TOPIC_ME, TOPIC_DASHBOARD, TOPIC_CHANNELS, TOPIC_PRICES],
    );
    Ok(Json(json!(crate::models::PublicChannel::from(channel))))
}

pub async fn update_channel(
    State(state): State<crate::app::AppState>,
    ConsoleAuth(auth): ConsoleAuth,
    Path(id): Path<i64>,
    Json(request): Json<ChannelUpdateInput>,
) -> AppResult<Json<serde_json::Value>> {
    let channel = state.db.update_channel(&auth.user, id, request).await?;
    publish_channel_owner_event(
        &state,
        channel.owner_user_id,
        [TOPIC_ME, TOPIC_DASHBOARD, TOPIC_CHANNELS, TOPIC_PRICES],
    );
    Ok(Json(json!(channel)))
}

pub async fn set_channel_enabled(
    State(state): State<crate::app::AppState>,
    ConsoleAuth(auth): ConsoleAuth,
    Path(id): Path<i64>,
    Json(request): Json<EnabledRequest>,
) -> AppResult<Json<serde_json::Value>> {
    let channel = state
        .db
        .set_channel_enabled(&auth.user, id, request.enabled)
        .await?;
    publish_channel_owner_event(
        &state,
        channel.owner_user_id,
        [TOPIC_ME, TOPIC_DASHBOARD, TOPIC_CHANNELS],
    );
    Ok(Json(json!(channel)))
}

pub async fn delete_channel(
    State(state): State<crate::app::AppState>,
    ConsoleAuth(auth): ConsoleAuth,
    Path(id): Path<i64>,
) -> AppResult<Json<serde_json::Value>> {
    let owner_user_id = state.db.get_channel(id).await?.owner_user_id;
    state.db.delete_channel(&auth.user, id).await?;
    publish_channel_owner_event(
        &state,
        owner_user_id,
        [TOPIC_ME, TOPIC_DASHBOARD, TOPIC_CHANNELS],
    );
    Ok(Json(json!({ "ok": true })))
}

pub async fn batch_set_channels_enabled(
    State(state): State<crate::app::AppState>,
    ConsoleAuth(auth): ConsoleAuth,
    Json(request): Json<ChannelBatchEnabledRequest>,
) -> AppResult<Json<serde_json::Value>> {
    let owner_user_ids = channel_owner_ids(&state, &request.ids).await?;
    let count = state
        .db
        .batch_set_channels_enabled(&auth.user, &request.ids, request.enabled)
        .await?;
    for owner_user_id in owner_user_ids {
        publish_channel_owner_event(
            &state,
            owner_user_id,
            [TOPIC_ME, TOPIC_DASHBOARD, TOPIC_CHANNELS],
        );
    }
    Ok(Json(json!({ "updated": count })))
}

pub async fn copy_channel(
    State(state): State<crate::app::AppState>,
    ConsoleAuth(auth): ConsoleAuth,
    Path(id): Path<i64>,
    Json(request): Json<CopyChannelRequest>,
) -> AppResult<Json<serde_json::Value>> {
    let suffix = request.suffix.as_deref().unwrap_or(" copy");
    let channel = state
        .db
        .copy_channel(&auth.user, id, suffix, request.reset_usage.unwrap_or(true))
        .await?;
    publish_channel_owner_event(
        &state,
        channel.owner_user_id,
        [TOPIC_ME, TOPIC_DASHBOARD, TOPIC_CHANNELS, TOPIC_PRICES],
    );
    Ok(Json(json!(channel)))
}

pub async fn test_channel(
    State(state): State<crate::app::AppState>,
    ConsoleAuth(auth): ConsoleAuth,
    Path(id): Path<i64>,
) -> AppResult<Json<serde_json::Value>> {
    let channel = state.db.get_channel(id).await?;
    if auth.user.role != "admin" && channel.owner_user_id != auth.user.id {
        return Err(AppError::Forbidden);
    }
    let url = match channel.provider {
        crate::models::ProviderKind::Gemini => {
            format!("{}/v1beta/models", channel.base_url.trim_end_matches('/'))
        }
        _ => format!("{}/v1/models", channel.base_url.trim_end_matches('/')),
    };
    let started = Instant::now();
    let result = state
        .http
        .get(url)
        .headers(provider_test_headers(
            &channel.provider,
            &channel.api_key_secret,
        ))
        .send()
        .await;
    let latency_ms = started.elapsed().as_millis().min(i64::MAX as u128) as i64;
    let (ok, message) = match result {
        Ok(response) if response.status().is_success() => {
            (true, format!("HTTP {}", response.status()))
        }
        Ok(response) => (false, format!("HTTP {}", response.status())),
        Err(err) => (false, err.to_string()),
    };
    state
        .db
        .record_channel_health_event(ChannelHealthEventInput {
            channel_id: id,
            request_id: None,
            status: if ok { "available" } else { "down" },
            http_status: None,
            ttft_ms: if ok { Some(latency_ms) } else { None },
            total_latency_ms: Some(latency_ms),
            error: if ok { None } else { Some(message.as_str()) },
        })
        .await?;
    publish_channel_owner_event(
        &state,
        channel.owner_user_id,
        [TOPIC_ME, TOPIC_DASHBOARD, TOPIC_CHANNELS],
    );
    Ok(Json(json!({
        "ok": ok,
        "latency_ms": latency_ms,
        "message": message,
    })))
}

pub async fn list_prices(
    State(state): State<crate::app::AppState>,
    ConsoleAuth(auth): ConsoleAuth,
) -> AppResult<Json<serde_json::Value>> {
    Ok(Json(json!(state.db.list_prices(&auth.user).await?)))
}

pub async fn upsert_price(
    State(state): State<crate::app::AppState>,
    ConsoleAuth(auth): ConsoleAuth,
    Json(request): Json<ModelPrice>,
) -> AppResult<Json<serde_json::Value>> {
    let owner_user_id = if let Some(channel_id) = request.channel_id {
        let channel = state.db.get_channel(channel_id).await?;
        if auth.user.role != "admin" && channel.owner_user_id != auth.user.id {
            return Err(AppError::Forbidden);
        }
        Some(channel.owner_user_id)
    } else {
        require_admin(&auth.user)?;
        None
    };
    state.db.upsert_price(&request).await?;
    if let Some(owner_user_id) = owner_user_id {
        publish_channel_owner_event(&state, owner_user_id, [TOPIC_PRICES]);
    } else {
        publish_global_event(&state, [TOPIC_PRICES]);
    }
    Ok(Json(json!({ "ok": true })))
}

pub async fn list_affinity_rules(
    State(state): State<crate::app::AppState>,
    ConsoleAuth(_auth): ConsoleAuth,
) -> AppResult<Json<serde_json::Value>> {
    Ok(Json(json!(state.db.list_affinity_rules().await?)))
}

pub async fn create_affinity_rule(
    State(state): State<crate::app::AppState>,
    ConsoleAuth(auth): ConsoleAuth,
    Json(request): Json<AffinityRuleInput>,
) -> AppResult<Json<serde_json::Value>> {
    require_admin(&auth.user)?;
    let rule = state.db.create_affinity_rule(request).await?;
    publish_admin_event(&state, [TOPIC_AFFINITY_RULES]);
    Ok(Json(json!(rule)))
}

pub async fn list_ledger(
    State(state): State<crate::app::AppState>,
    ConsoleAuth(auth): ConsoleAuth,
) -> AppResult<Json<serde_json::Value>> {
    let user_filter = if auth.user.role == "admin" {
        None
    } else {
        Some(auth.user.id)
    };
    Ok(Json(json!(state.db.list_ledger(user_filter).await?)))
}

pub async fn dashboard(
    State(state): State<crate::app::AppState>,
    ConsoleAuth(_auth): ConsoleAuth,
) -> AppResult<Json<serde_json::Value>> {
    state.db.refresh_channel_windows().await?;
    let (multiplier, status) = surge_multiplier(&state).await;
    Ok(Json(json!(state.db.dashboard(multiplier, status).await?)))
}

pub async fn get_settings(
    State(state): State<crate::app::AppState>,
    ConsoleAuth(auth): ConsoleAuth,
) -> AppResult<Json<serde_json::Value>> {
    require_admin(&auth.user)?;
    let settings = state.db.list_settings().await?;
    let runtime = state.db.runtime_settings().await?;
    Ok(Json(json!({
        "settings": settings,
        "runtime": runtime,
    })))
}

pub async fn runtime_settings(
    State(state): State<crate::app::AppState>,
    ConsoleAuth(_auth): ConsoleAuth,
) -> AppResult<Json<serde_json::Value>> {
    Ok(Json(json!(state.db.runtime_settings().await?)))
}

pub async fn update_settings(
    State(state): State<crate::app::AppState>,
    ConsoleAuth(auth): ConsoleAuth,
    Json(request): Json<Vec<SettingUpdate>>,
) -> AppResult<Json<serde_json::Value>> {
    require_admin(&auth.user)?;
    state.db.upsert_settings(&request).await?;
    publish_global_event(
        &state,
        [
            TOPIC_RUNTIME_SETTINGS,
            TOPIC_DASHBOARD,
            TOPIC_CHANNELS,
            TOPIC_PRICES,
        ],
    );
    publish_admin_event(&state, [TOPIC_SETTINGS]);
    Ok(Json(json!({ "ok": true })))
}

pub async fn set_anonymous_leaderboard(
    State(state): State<crate::app::AppState>,
    ConsoleAuth(auth): ConsoleAuth,
    Json(request): Json<EnabledRequest>,
) -> AppResult<Json<serde_json::Value>> {
    let user = state
        .db
        .set_anonymous_leaderboard(auth.user.id, request.enabled)
        .await?;
    publish_user_event(&state, auth.user.id, [TOPIC_ME]);
    publish_global_event(&state, [TOPIC_LEADERBOARDS]);
    Ok(Json(json!(user)))
}

pub async fn transfer_points(
    State(state): State<crate::app::AppState>,
    ConsoleAuth(auth): ConsoleAuth,
    Json(request): Json<TransferRequest>,
) -> AppResult<Json<serde_json::Value>> {
    state
        .db
        .transfer_points(
            auth.user.id,
            request.to_user_id,
            request.points,
            request.memo.as_deref(),
        )
        .await?;
    publish_user_event(
        &state,
        auth.user.id,
        [TOPIC_ME, TOPIC_DASHBOARD, TOPIC_TRANSFERS],
    );
    publish_user_event(
        &state,
        request.to_user_id,
        [TOPIC_ME, TOPIC_DASHBOARD, TOPIC_TRANSFERS],
    );
    publish_admin_event(&state, [TOPIC_USERS, TOPIC_DASHBOARD]);
    Ok(Json(json!({ "ok": true })))
}

pub async fn list_transfers(
    State(state): State<crate::app::AppState>,
    ConsoleAuth(auth): ConsoleAuth,
) -> AppResult<Json<serde_json::Value>> {
    Ok(Json(json!(state.db.list_transfers(auth.user.id).await?)))
}

pub async fn create_red_packet(
    State(state): State<crate::app::AppState>,
    ConsoleAuth(auth): ConsoleAuth,
    Json(request): Json<RedPacketRequest>,
) -> AppResult<Json<serde_json::Value>> {
    state
        .db
        .create_red_packet(
            auth.user.id,
            &request.phrase,
            request.total_points,
            request.total_parts,
            &request.mode,
        )
        .await?;
    publish_user_event(
        &state,
        auth.user.id,
        [TOPIC_ME, TOPIC_DASHBOARD, TOPIC_REDPACKETS],
    );
    publish_admin_event(&state, [TOPIC_USERS, TOPIC_DASHBOARD]);
    Ok(Json(json!({ "phrase": request.phrase })))
}

pub async fn claim_red_packet(
    State(state): State<crate::app::AppState>,
    ConsoleAuth(auth): ConsoleAuth,
    Json(request): Json<ClaimRedPacketRequest>,
) -> AppResult<Json<serde_json::Value>> {
    let creator_user_id = red_packet_creator_user_id(&state, &request.phrase).await?;
    let points = state
        .db
        .claim_red_packet(auth.user.id, &request.phrase)
        .await?;
    publish_user_event(&state, auth.user.id, [TOPIC_ME, TOPIC_DASHBOARD]);
    if let Some(creator_user_id) = creator_user_id
        && creator_user_id != auth.user.id
    {
        publish_user_event(&state, creator_user_id, [TOPIC_DASHBOARD, TOPIC_REDPACKETS]);
    }
    publish_admin_event(&state, [TOPIC_USERS, TOPIC_DASHBOARD]);
    Ok(Json(json!({ "points": points })))
}

pub async fn list_red_packets(
    State(state): State<crate::app::AppState>,
    ConsoleAuth(auth): ConsoleAuth,
) -> AppResult<Json<serde_json::Value>> {
    Ok(Json(json!(state.db.list_red_packets(auth.user.id).await?)))
}

pub async fn leaderboards(
    State(state): State<crate::app::AppState>,
    Query(query): Query<LeaderboardQuery>,
    ConsoleAuth(_auth): ConsoleAuth,
) -> AppResult<Json<serde_json::Value>> {
    let period =
        crate::db::LeaderboardPeriod::try_from(query.period.as_deref().unwrap_or("month"))?;
    Ok(Json(
        state
            .db
            .leaderboards(period, state.leaderboard_timezone.as_deref())
            .await?,
    ))
}

async fn channel_owner_ids(state: &crate::app::AppState, ids: &[i64]) -> AppResult<Vec<i64>> {
    let mut owner_user_ids = BTreeSet::new();
    for id in ids {
        owner_user_ids.insert(state.db.get_channel(*id).await?.owner_user_id);
    }
    Ok(owner_user_ids.into_iter().collect())
}

async fn red_packet_creator_user_id(
    state: &crate::app::AppState,
    phrase: &str,
) -> AppResult<Option<i64>> {
    Ok(
        sqlx::query_scalar("SELECT creator_user_id FROM red_packets WHERE phrase = ?")
            .bind(phrase)
            .fetch_optional(&state.db.pool)
            .await?,
    )
}

fn provider_test_headers(
    provider: &crate::models::ProviderKind,
    api_key: &str,
) -> axum::http::HeaderMap {
    let mut headers = axum::http::HeaderMap::new();
    match provider {
        crate::models::ProviderKind::OpenAi => {
            if let Ok(value) = axum::http::HeaderValue::from_str(&format!("Bearer {api_key}")) {
                headers.insert(axum::http::header::AUTHORIZATION, value);
            }
        }
        crate::models::ProviderKind::Anthropic => {
            if let Ok(value) = axum::http::HeaderValue::from_str(api_key) {
                headers.insert("x-api-key", value);
            }
            headers.insert(
                "anthropic-version",
                axum::http::HeaderValue::from_static("2023-06-01"),
            );
        }
        crate::models::ProviderKind::Gemini => {
            if let Ok(value) = axum::http::HeaderValue::from_str(api_key) {
                headers.insert("x-goog-api-key", value);
            }
        }
    }
    headers
}
