use axum::{
    Json,
    extract::{Path, State},
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    auth::{ConsoleAuth, require_admin, verify_password},
    db::{AffinityRuleInput, ChannelInput, SettingUpdate},
    error::{AppError, AppResult},
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
    pub spend_limit_points: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub struct EnabledRequest {
    pub enabled: bool,
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

pub async fn register(
    State(state): State<crate::app::AppState>,
    Json(request): Json<RegisterRequest>,
) -> AppResult<Json<AuthResponse>> {
    if request.password.len() < 8 {
        return Err(AppError::BadRequest(
            "password must be at least 8 characters".to_string(),
        ));
    }
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
    Ok(Json(AuthResponse { token, user }))
}

pub async fn login(
    State(state): State<crate::app::AppState>,
    Json(request): Json<LoginRequest>,
) -> AppResult<Json<AuthResponse>> {
    let Some((user, password_hash)) = state.db.find_user_with_hash(&request.email).await? else {
        return Err(AppError::Unauthorized);
    };
    if !verify_password(&request.password, &password_hash) {
        return Err(AppError::Unauthorized);
    }
    let token = state.db.create_session(user.id).await?;
    Ok(Json(AuthResponse { token, user }))
}

pub async fn me(ConsoleAuth(auth): ConsoleAuth) -> AppResult<Json<User>> {
    Ok(Json(auth.user))
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
    Ok(Json(json!({ "ok": true })))
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

pub async fn create_channel(
    State(state): State<crate::app::AppState>,
    ConsoleAuth(auth): ConsoleAuth,
    Json(request): Json<ChannelInput>,
) -> AppResult<Json<serde_json::Value>> {
    let channel = state.db.upsert_channel(auth.user.id, request).await?;
    Ok(Json(json!(crate::models::PublicChannel::from(channel))))
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
    if let Some(channel_id) = request.channel_id {
        let channel = state.db.get_channel(channel_id).await?;
        if auth.user.role != "admin" && channel.owner_user_id != auth.user.id {
            return Err(AppError::Forbidden);
        }
    } else {
        require_admin(&auth.user)?;
    }
    state.db.upsert_price(&request).await?;
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
    Ok(Json(json!(state.db.list_settings().await?)))
}

pub async fn update_settings(
    State(state): State<crate::app::AppState>,
    ConsoleAuth(auth): ConsoleAuth,
    Json(request): Json<Vec<SettingUpdate>>,
) -> AppResult<Json<serde_json::Value>> {
    require_admin(&auth.user)?;
    state.db.upsert_settings(&request).await?;
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
    Ok(Json(json!({ "phrase": request.phrase })))
}

pub async fn claim_red_packet(
    State(state): State<crate::app::AppState>,
    ConsoleAuth(auth): ConsoleAuth,
    Json(request): Json<ClaimRedPacketRequest>,
) -> AppResult<Json<serde_json::Value>> {
    let points = state
        .db
        .claim_red_packet(auth.user.id, &request.phrase)
        .await?;
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
    ConsoleAuth(_auth): ConsoleAuth,
) -> AppResult<Json<serde_json::Value>> {
    Ok(Json(state.db.leaderboards().await?))
}
