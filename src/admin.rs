use axum::{
    Json,
    extract::{Path, State},
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    auth::{ConsoleAuth, require_admin, verify_password},
    db::{AffinityRuleInput, ChannelInput},
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

pub async fn register(
    State(state): State<crate::app::AppState>,
    Json(request): Json<RegisterRequest>,
) -> AppResult<Json<AuthResponse>> {
    if request.password.len() < 8 {
        return Err(AppError::BadRequest("password must be at least 8 characters".to_string()));
    }
    let invite_required = sqlx::query_scalar::<_, String>(
        "SELECT value FROM system_settings WHERE key = 'invite_required'",
    )
    .fetch_optional(&state.db.pool)
    .await?
    .unwrap_or_else(|| "false".to_string());
    if invite_required == "true" && request.invite_code.as_deref() != Some("TOKENALTAR") {
        return Err(AppError::Forbidden);
    }
    let display_name = request
        .display_name
        .unwrap_or_else(|| request.email.split('@').next().unwrap_or("user").to_string());
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
    ConsoleAuth(_auth): ConsoleAuth,
) -> AppResult<Json<serde_json::Value>> {
    Ok(Json(json!(state.db.list_channels().await?)))
}

pub async fn create_channel(
    State(state): State<crate::app::AppState>,
    ConsoleAuth(auth): ConsoleAuth,
    Json(request): Json<ChannelInput>,
) -> AppResult<Json<serde_json::Value>> {
    require_admin(&auth.user)?;
    let channel = state.db.upsert_channel(auth.user.id, request).await?;
    Ok(Json(json!(channel)))
}

pub async fn list_prices(
    State(state): State<crate::app::AppState>,
    ConsoleAuth(_auth): ConsoleAuth,
) -> AppResult<Json<serde_json::Value>> {
    Ok(Json(json!(state.db.list_prices().await?)))
}

pub async fn upsert_price(
    State(state): State<crate::app::AppState>,
    ConsoleAuth(auth): ConsoleAuth,
    Json(request): Json<ModelPrice>,
) -> AppResult<Json<serde_json::Value>> {
    require_admin(&auth.user)?;
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
    let (multiplier, status) = surge_multiplier(&state).await;
    Ok(Json(json!(state.db.dashboard(multiplier, status).await?)))
}
