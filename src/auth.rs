use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier, password_hash::SaltString};
use axum::{
    extract::{FromRef, FromRequestParts},
    http::{StatusCode, request::Parts},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::RngCore;
use sha2::{Digest, Sha256};

use crate::{
    app::AppState,
    error::{AppError, AppResult},
    models::{AuthContext, User},
};

pub fn hash_password(password: &str) -> AppResult<String> {
    let salt = SaltString::encode_b64(&rand::random::<[u8; 16]>())
        .map_err(|err| AppError::BadRequest(format!("password salt failed: {err}")))?;
    let hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|err| AppError::BadRequest(format!("password hash failed: {err}")))?;
    Ok(hash.to_string())
}

pub fn verify_password(password: &str, hash: &str) -> bool {
    let parsed = match PasswordHash::new(hash) {
        Ok(parsed) => parsed,
        Err(_) => return false,
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

pub fn generate_token(prefix: &str) -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    format!("{prefix}-{}", URL_SAFE_NO_PAD.encode(bytes))
}

pub fn hash_token(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    format!("{digest:x}")
}

#[derive(Debug, Clone)]
pub struct ConsoleAuth(pub AuthContext);

#[derive(Debug, Clone)]
pub struct GatewayAuth(pub AuthContext);

async fn auth_from_bearer(state: &AppState, token: &str) -> AppResult<AuthContext> {
    if token.starts_with("sk-") {
        let api_key = state.db.find_api_key(&hash_token(token)).await?;
        if !api_key.enabled {
            return Err(AppError::Unauthorized);
        }
        state.db.mark_api_key_used(api_key.id).await?;
        let user = state.db.get_user(api_key.user_id).await?;
        Ok(AuthContext {
            user,
            api_key: Some(api_key),
        })
    } else if token.starts_with("ta-") {
        let session = state.db.find_session_user(&hash_token(token)).await?;
        Ok(AuthContext {
            user: session,
            api_key: None,
        })
    } else {
        Err(AppError::Unauthorized)
    }
}

impl<S> FromRequestParts<S> for ConsoleAuth
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = (StatusCode, String);

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let state = AppState::from_ref(state);
        let auth = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .ok_or((StatusCode::UNAUTHORIZED, "missing bearer token".to_string()))?;
        auth_from_bearer(&state, auth)
            .await
            .map(ConsoleAuth)
            .map_err(|err| (StatusCode::UNAUTHORIZED, err.to_string()))
    }
}

impl<S> FromRequestParts<S> for GatewayAuth
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = (StatusCode, String);

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let state = AppState::from_ref(state);
        let auth = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .ok_or((StatusCode::UNAUTHORIZED, "missing bearer token".to_string()))?;
        auth_from_bearer(&state, auth)
            .await
            .map(GatewayAuth)
            .map_err(|err| (StatusCode::UNAUTHORIZED, err.to_string()))
    }
}

pub fn require_admin(user: &User) -> AppResult<()> {
    if user.role == "admin" {
        Ok(())
    } else {
        Err(AppError::Forbidden)
    }
}
