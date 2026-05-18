use std::{str::FromStr, time::Duration};

use chrono::{DateTime, Datelike, Local, TimeZone, Utc};
use chrono_tz::Tz;
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{
    Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};

use crate::{
    auth::{generate_token, hash_password, hash_token},
    error::{AppError, AppResult},
    models::{
        AffinityRule, ApiKeyRecord, Channel, ChannelLimits, LedgerEvent, ModelPrice, PublicChannel,
        User, json_array_to_strings,
    },
};

#[derive(Clone)]
pub struct Database {
    pub pool: SqlitePool,
}

#[derive(Debug, Clone, Serialize)]
pub struct DashboardSummary {
    pub users: i64,
    pub channels: i64,
    pub enabled_channels: i64,
    pub available_tokens: i64,
    pub spent_points_today: f64,
    pub surge_multiplier: f64,
    pub surge_state: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaderboardPeriod {
    Day,
    Month,
}

impl LeaderboardPeriod {
    fn as_str(self) -> &'static str {
        match self {
            Self::Day => "day",
            Self::Month => "month",
        }
    }
}

impl TryFrom<&str> for LeaderboardPeriod {
    type Error = AppError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "day" => Ok(Self::Day),
            "month" => Ok(Self::Month),
            other => Err(AppError::BadRequest(format!(
                "unsupported leaderboard period: {other}"
            ))),
        }
    }
}

impl Database {
    pub async fn connect(database_url: &str) -> AppResult<Self> {
        let options = SqliteConnectOptions::from_str(database_url)?
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(8)
            .acquire_timeout(Duration::from_secs(5))
            .connect_with(options)
            .await?;
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .map_err(|err| AppError::Anyhow(anyhow::anyhow!(err)))?;
        Ok(Self { pool })
    }

    pub async fn bootstrap_admin(&self, email: &str, password: &str) -> AppResult<()> {
        let existing: Option<(i64,)> = sqlx::query_as("SELECT id FROM users WHERE email = ?")
            .bind(email)
            .fetch_optional(&self.pool)
            .await?;
        if existing.is_some() {
            return Ok(());
        }
        let password_hash = hash_password(password)?;
        sqlx::query(
            "INSERT INTO users(email, password_hash, role, display_name, points_balance) VALUES (?, ?, 'admin', 'Admin', 1000000)",
        )
        .bind(email)
        .bind(password_hash)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn create_user(
        &self,
        email: &str,
        password: &str,
        display_name: &str,
    ) -> AppResult<User> {
        let password_hash = hash_password(password)?;
        let result = sqlx::query(
            "INSERT INTO users(email, password_hash, role, display_name, points_balance) VALUES (?, ?, 'user', ?, 1000)",
        )
        .bind(email)
        .bind(password_hash)
        .bind(display_name)
        .execute(&self.pool)
        .await?;
        self.get_user(result.last_insert_rowid()).await
    }

    pub async fn find_user_with_hash(&self, email: &str) -> AppResult<Option<(User, String)>> {
        let row = sqlx::query(
            "SELECT id, email, password_hash, role, display_name, points_balance, anonymous_leaderboard FROM users WHERE email = ?",
        )
        .bind(email)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|row| {
            (
                User {
                    id: row.get("id"),
                    email: row.get("email"),
                    role: row.get("role"),
                    display_name: row.get("display_name"),
                    points_balance: row.get("points_balance"),
                    anonymous_leaderboard: row.get::<i64, _>("anonymous_leaderboard") != 0,
                },
                row.get("password_hash"),
            )
        }))
    }

    pub async fn get_user(&self, id: i64) -> AppResult<User> {
        let row = sqlx::query(
            "SELECT id, email, role, display_name, points_balance, anonymous_leaderboard FROM users WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(AppError::NotFound)?;
        Ok(User {
            id: row.get("id"),
            email: row.get("email"),
            role: row.get("role"),
            display_name: row.get("display_name"),
            points_balance: row.get("points_balance"),
            anonymous_leaderboard: row.get::<i64, _>("anonymous_leaderboard") != 0,
        })
    }

    pub async fn create_session(&self, user_id: i64) -> AppResult<String> {
        let token = generate_token("ta");
        let expires_at = (Utc::now() + chrono::Duration::days(30)).to_rfc3339();
        sqlx::query("INSERT INTO sessions(token_hash, user_id, expires_at) VALUES (?, ?, ?)")
            .bind(hash_token(&token))
            .bind(user_id)
            .bind(expires_at)
            .execute(&self.pool)
            .await?;
        Ok(token)
    }

    pub async fn consume_invite_code(&self, code: &str) -> AppResult<bool> {
        let mut tx = self.pool.begin().await?;
        let row =
            sqlx::query("SELECT enabled, max_uses, used_count FROM invite_codes WHERE code = ?")
                .bind(code)
                .fetch_optional(&mut *tx)
                .await?;
        let accepted = if let Some(row) = row {
            let enabled = row.get::<i64, _>("enabled") != 0;
            let max_uses: Option<i64> = row.get("max_uses");
            let used_count: i64 = row.get("used_count");
            enabled && max_uses.is_none_or(|max| used_count < max)
        } else {
            let default_code = sqlx::query_scalar::<_, String>(
                "SELECT value FROM system_settings WHERE key = 'invite_code_default'",
            )
            .fetch_optional(&mut *tx)
            .await?
            .unwrap_or_else(|| "TOKENALTAR".to_string());
            code == default_code
        };
        if accepted {
            sqlx::query("UPDATE invite_codes SET used_count = used_count + 1 WHERE code = ?")
                .bind(code)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(accepted)
    }

    pub async fn find_session_user(&self, token_hash: &str) -> AppResult<User> {
        let row = sqlx::query(
            "SELECT user_id FROM sessions WHERE token_hash = ? AND expires_at > datetime('now')",
        )
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(AppError::Unauthorized)?;
        self.get_user(row.get("user_id")).await
    }

    pub async fn create_api_key(
        &self,
        user_id: i64,
        name: &str,
        spend_limit_points: Option<f64>,
    ) -> AppResult<(String, ApiKeyRecord)> {
        let token = generate_token("sk");
        let key_prefix = token.chars().take(12).collect::<String>();
        let result = sqlx::query(
            "INSERT INTO api_keys(user_id, name, key_prefix, key_hash, spend_limit_points, allowed_models_json, updated_at) VALUES (?, ?, ?, ?, ?, '[]', datetime('now'))",
        )
        .bind(user_id)
        .bind(name)
        .bind(&key_prefix)
        .bind(hash_token(&token))
        .bind(spend_limit_points)
        .execute(&self.pool)
        .await?;
        let record = self.get_api_key(result.last_insert_rowid()).await?;
        Ok((token, record))
    }

    pub async fn get_api_key(&self, id: i64) -> AppResult<ApiKeyRecord> {
        let row = sqlx::query(
            "SELECT id, user_id, name, key_prefix, enabled, spend_limit_points, spent_points, expires_at, allowed_models_json, last_used_at FROM api_keys WHERE id = ? AND deleted_at IS NULL",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(AppError::NotFound)?;
        Ok(api_key_from_row(&row))
    }

    pub async fn list_api_keys(&self, user_id: i64) -> AppResult<Vec<ApiKeyRecord>> {
        let rows = sqlx::query(
            "SELECT id, user_id, name, key_prefix, enabled, spend_limit_points, spent_points, expires_at, allowed_models_json, last_used_at FROM api_keys WHERE user_id = ? AND deleted_at IS NULL ORDER BY id DESC",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.iter().map(api_key_from_row).collect())
    }

    pub async fn set_api_key_enabled(&self, user_id: i64, id: i64, enabled: bool) -> AppResult<()> {
        let result = sqlx::query(
            "UPDATE api_keys SET enabled = ?, updated_at = datetime('now') WHERE id = ? AND user_id = ? AND deleted_at IS NULL",
        )
            .bind(if enabled { 1 } else { 0 })
            .bind(id)
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        if result.rows_affected() == 0 {
            Err(AppError::NotFound)
        } else {
            Ok(())
        }
    }

    pub async fn update_api_key(
        &self,
        user_id: i64,
        id: i64,
        input: ApiKeyUpdateInput,
    ) -> AppResult<ApiKeyRecord> {
        validate_api_key_name(&input.name)?;
        validate_spend_limit(input.spend_limit_points)?;
        let allowed_models_json = normalize_models_json(&input.allowed_models)?;
        let result = sqlx::query(
            r#"
            UPDATE api_keys
            SET name = ?, enabled = ?, spend_limit_points = ?, expires_at = ?,
                allowed_models_json = ?, updated_at = datetime('now')
            WHERE id = ? AND user_id = ? AND deleted_at IS NULL
            "#,
        )
        .bind(input.name.trim())
        .bind(if input.enabled { 1 } else { 0 })
        .bind(input.spend_limit_points)
        .bind(normalize_optional_text(input.expires_at.as_deref()))
        .bind(allowed_models_json)
        .bind(id)
        .bind(user_id)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            return Err(AppError::NotFound);
        }
        self.get_api_key(id).await
    }

    pub async fn rotate_api_key(&self, user_id: i64, id: i64) -> AppResult<(String, ApiKeyRecord)> {
        let token = generate_token("sk");
        let key_prefix = token.chars().take(12).collect::<String>();
        let result = sqlx::query(
            r#"
            UPDATE api_keys
            SET key_prefix = ?, key_hash = ?, enabled = 1, updated_at = datetime('now')
            WHERE id = ? AND user_id = ? AND deleted_at IS NULL
            "#,
        )
        .bind(&key_prefix)
        .bind(hash_token(&token))
        .bind(id)
        .bind(user_id)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            return Err(AppError::NotFound);
        }
        Ok((token, self.get_api_key(id).await?))
    }

    pub async fn delete_api_key(&self, user_id: i64, id: i64) -> AppResult<()> {
        let result = sqlx::query(
            "UPDATE api_keys SET enabled = 0, deleted_at = datetime('now'), updated_at = datetime('now') WHERE id = ? AND user_id = ? AND deleted_at IS NULL",
        )
        .bind(id)
        .bind(user_id)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            Err(AppError::NotFound)
        } else {
            Ok(())
        }
    }

    pub async fn batch_delete_api_keys(&self, user_id: i64, ids: &[i64]) -> AppResult<u64> {
        validate_batch_ids(ids)?;
        let mut tx = self.pool.begin().await?;
        let mut count = 0;
        for id in ids {
            let result = sqlx::query(
                "UPDATE api_keys SET enabled = 0, deleted_at = datetime('now'), updated_at = datetime('now') WHERE id = ? AND user_id = ? AND deleted_at IS NULL",
            )
            .bind(id)
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
            count += result.rows_affected();
        }
        tx.commit().await?;
        Ok(count)
    }

    pub async fn find_api_key(&self, key_hash: &str) -> AppResult<ApiKeyRecord> {
        let row = sqlx::query(
            r#"
            SELECT id, user_id, name, key_prefix, enabled, spend_limit_points, spent_points,
                   expires_at, allowed_models_json, last_used_at
            FROM api_keys
            WHERE key_hash = ? AND deleted_at IS NULL
              AND (expires_at IS NULL OR expires_at > datetime('now'))
            "#,
        )
        .bind(key_hash)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(AppError::Unauthorized)?;
        Ok(api_key_from_row(&row))
    }

    pub async fn mark_api_key_used(&self, id: i64) -> AppResult<()> {
        sqlx::query("UPDATE api_keys SET last_used_at = datetime('now') WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn list_route_channels(&self) -> AppResult<Vec<Channel>> {
        let rows = sqlx::query(
            r#"
            SELECT c.id, c.owner_user_id, c.name, c.provider, c.base_url, c.api_key_secret, c.models_json,
                   c.enabled, c.status, c.health_checked_at, c.upstream_latency_ms, c.last_error,
                   l.cycle_limit_tokens, l.cycle_reset_day, l.daily_limit_tokens, l.hourly_limit_tokens,
                   l.used_cycle_tokens, l.used_day_tokens, l.used_hour_tokens,
                   l.fire_sale_days_before, l.fire_sale_remaining_pct, l.fire_sale_discount, l.provider_share
            FROM channels c JOIN channel_limits l ON c.id = l.channel_id
            WHERE c.deleted_at IS NULL
            ORDER BY c.id DESC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(channel_from_row).collect()
    }

    pub async fn list_public_channels(&self, user: &User) -> AppResult<Vec<PublicChannel>> {
        let rows = if user.role == "admin" {
            sqlx::query(
                r#"
                SELECT c.id, c.owner_user_id, c.name, c.provider, c.base_url, c.api_key_secret, c.models_json,
                       c.enabled, c.status, c.health_checked_at, c.upstream_latency_ms, c.last_error,
                       l.cycle_limit_tokens, l.cycle_reset_day, l.daily_limit_tokens, l.hourly_limit_tokens,
                       l.used_cycle_tokens, l.used_day_tokens, l.used_hour_tokens,
                       l.fire_sale_days_before, l.fire_sale_remaining_pct, l.fire_sale_discount, l.provider_share
                FROM channels c JOIN channel_limits l ON c.id = l.channel_id
                WHERE c.deleted_at IS NULL
                ORDER BY c.id DESC
                "#,
            )
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query(
                r#"
                SELECT c.id, c.owner_user_id, c.name, c.provider, c.base_url, c.api_key_secret, c.models_json,
                       c.enabled, c.status, c.health_checked_at, c.upstream_latency_ms, c.last_error,
                       l.cycle_limit_tokens, l.cycle_reset_day, l.daily_limit_tokens, l.hourly_limit_tokens,
                       l.used_cycle_tokens, l.used_day_tokens, l.used_hour_tokens,
                       l.fire_sale_days_before, l.fire_sale_remaining_pct, l.fire_sale_discount, l.provider_share
                FROM channels c JOIN channel_limits l ON c.id = l.channel_id
                WHERE c.owner_user_id = ? AND c.deleted_at IS NULL
                ORDER BY c.id DESC
                "#,
            )
            .bind(user.id)
            .fetch_all(&self.pool)
            .await?
        };
        rows.iter()
            .map(channel_from_row)
            .map(|result| result.map(PublicChannel::from))
            .collect()
    }

    pub async fn upsert_channel(
        &self,
        owner_user_id: i64,
        input: ChannelInput,
    ) -> AppResult<Channel> {
        validate_channel_input(&input, true)?;
        let mut tx = self.pool.begin().await?;
        let models_json = serde_json::to_string(&input.models)
            .map_err(|err| AppError::Anyhow(anyhow::anyhow!(err)))?;
        let result = sqlx::query(
            "INSERT INTO channels(owner_user_id, name, provider, base_url, api_key_secret, models_json, enabled) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(owner_user_id)
        .bind(&input.name)
        .bind(&input.provider)
        .bind(&input.base_url)
        .bind(&input.api_key_secret)
        .bind(models_json)
        .bind(if input.enabled { 1 } else { 0 })
        .execute(&mut *tx)
        .await?;
        let channel_id = result.last_insert_rowid();
        sqlx::query(
            r#"
            INSERT INTO channel_limits(
              channel_id, cycle_limit_tokens, cycle_reset_day, daily_limit_tokens, hourly_limit_tokens,
              fire_sale_days_before, fire_sale_remaining_pct, fire_sale_discount, provider_share
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(channel_id)
        .bind(input.cycle_limit_tokens)
        .bind(input.cycle_reset_day)
        .bind(input.daily_limit_tokens)
        .bind(input.hourly_limit_tokens)
        .bind(input.fire_sale_days_before)
        .bind(input.fire_sale_remaining_pct)
        .bind(input.fire_sale_discount)
        .bind(input.provider_share)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        self.get_channel(channel_id).await
    }

    pub async fn get_channel(&self, id: i64) -> AppResult<Channel> {
        let row = sqlx::query(
            r#"
            SELECT c.id, c.owner_user_id, c.name, c.provider, c.base_url, c.api_key_secret, c.models_json,
                   c.enabled, c.status, c.health_checked_at, c.upstream_latency_ms, c.last_error,
                   l.cycle_limit_tokens, l.cycle_reset_day, l.daily_limit_tokens, l.hourly_limit_tokens,
                   l.used_cycle_tokens, l.used_day_tokens, l.used_hour_tokens,
                   l.fire_sale_days_before, l.fire_sale_remaining_pct, l.fire_sale_discount, l.provider_share
            FROM channels c JOIN channel_limits l ON c.id = l.channel_id
            WHERE c.id = ? AND c.deleted_at IS NULL
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(AppError::NotFound)?;
        channel_from_row(&row)
    }

    pub async fn update_channel(
        &self,
        user: &User,
        id: i64,
        input: ChannelUpdateInput,
    ) -> AppResult<PublicChannel> {
        validate_channel_update(&input)?;
        let existing = self.get_channel(id).await?;
        if user.role != "admin" && existing.owner_user_id != user.id {
            return Err(AppError::Forbidden);
        }
        let models_json = normalize_models_json(&input.models)?;
        let api_key = normalize_optional_text(input.api_key_secret.as_deref())
            .unwrap_or(existing.api_key_secret);
        let mut tx = self.pool.begin().await?;
        let result = sqlx::query(
            r#"
            UPDATE channels
            SET name = ?, provider = ?, base_url = ?, api_key_secret = ?, models_json = ?,
                enabled = ?, status = CASE WHEN ? = 1 THEN 'healthy' ELSE status END,
                updated_at = datetime('now')
            WHERE id = ? AND deleted_at IS NULL
            "#,
        )
        .bind(input.name.trim())
        .bind(&input.provider)
        .bind(input.base_url.trim())
        .bind(api_key)
        .bind(models_json)
        .bind(if input.enabled { 1 } else { 0 })
        .bind(if input.enabled { 1 } else { 0 })
        .bind(id)
        .execute(&mut *tx)
        .await?;
        if result.rows_affected() == 0 {
            return Err(AppError::NotFound);
        }
        sqlx::query(
            r#"
            UPDATE channel_limits
            SET cycle_limit_tokens = ?, cycle_reset_day = ?, daily_limit_tokens = ?, hourly_limit_tokens = ?,
                fire_sale_days_before = ?, fire_sale_remaining_pct = ?, fire_sale_discount = ?,
                provider_share = ?, updated_at = datetime('now')
            WHERE channel_id = ?
            "#,
        )
        .bind(input.cycle_limit_tokens)
        .bind(input.cycle_reset_day)
        .bind(input.daily_limit_tokens)
        .bind(input.hourly_limit_tokens)
        .bind(input.fire_sale_days_before)
        .bind(input.fire_sale_remaining_pct)
        .bind(input.fire_sale_discount)
        .bind(input.provider_share)
        .bind(id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        self.get_channel(id).await.map(PublicChannel::from)
    }

    pub async fn set_channel_enabled(
        &self,
        user: &User,
        id: i64,
        enabled: bool,
    ) -> AppResult<PublicChannel> {
        let existing = self.get_channel(id).await?;
        if user.role != "admin" && existing.owner_user_id != user.id {
            return Err(AppError::Forbidden);
        }
        let status = if enabled {
            "healthy"
        } else {
            "manual_disabled"
        };
        let result = sqlx::query(
            "UPDATE channels SET enabled = ?, status = ?, updated_at = datetime('now') WHERE id = ? AND deleted_at IS NULL",
        )
        .bind(if enabled { 1 } else { 0 })
        .bind(status)
        .bind(id)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            return Err(AppError::NotFound);
        }
        self.get_channel(id).await.map(PublicChannel::from)
    }

    pub async fn delete_channel(&self, user: &User, id: i64) -> AppResult<()> {
        let existing = self.get_channel(id).await?;
        if user.role != "admin" && existing.owner_user_id != user.id {
            return Err(AppError::Forbidden);
        }
        let result = sqlx::query(
            "UPDATE channels SET enabled = 0, status = 'deleted', deleted_at = datetime('now'), updated_at = datetime('now') WHERE id = ? AND deleted_at IS NULL",
        )
        .bind(id)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            Err(AppError::NotFound)
        } else {
            Ok(())
        }
    }

    pub async fn batch_set_channels_enabled(
        &self,
        user: &User,
        ids: &[i64],
        enabled: bool,
    ) -> AppResult<u64> {
        validate_batch_ids(ids)?;
        for id in ids {
            let existing = self.get_channel(*id).await?;
            if user.role != "admin" && existing.owner_user_id != user.id {
                return Err(AppError::Forbidden);
            }
        }
        let mut tx = self.pool.begin().await?;
        let mut count = 0;
        let status = if enabled {
            "healthy"
        } else {
            "manual_disabled"
        };
        for id in ids {
            let result = sqlx::query(
                "UPDATE channels SET enabled = ?, status = ?, updated_at = datetime('now') WHERE id = ? AND deleted_at IS NULL",
            )
            .bind(if enabled { 1 } else { 0 })
            .bind(status)
            .bind(id)
            .execute(&mut *tx)
            .await?;
            count += result.rows_affected();
        }
        tx.commit().await?;
        Ok(count)
    }

    pub async fn copy_channel(
        &self,
        user: &User,
        id: i64,
        suffix: &str,
        reset_usage: bool,
    ) -> AppResult<PublicChannel> {
        let existing = self.get_channel(id).await?;
        if user.role != "admin" && existing.owner_user_id != user.id {
            return Err(AppError::Forbidden);
        }
        let input = ChannelInput {
            name: format!("{}{}", existing.name, suffix),
            provider: existing.provider.as_db().to_string(),
            base_url: existing.base_url,
            api_key_secret: existing.api_key_secret,
            models: existing.models,
            enabled: existing.enabled,
            cycle_limit_tokens: existing.limits.cycle_limit_tokens,
            cycle_reset_day: existing.limits.cycle_reset_day,
            daily_limit_tokens: existing.limits.daily_limit_tokens,
            hourly_limit_tokens: existing.limits.hourly_limit_tokens,
            fire_sale_days_before: existing.limits.fire_sale_days_before,
            fire_sale_remaining_pct: existing.limits.fire_sale_remaining_pct,
            fire_sale_discount: existing.limits.fire_sale_discount,
            provider_share: existing.limits.provider_share,
        };
        let clone = self.upsert_channel(existing.owner_user_id, input).await?;
        if !reset_usage {
            sqlx::query(
                r#"
                UPDATE channel_limits
                SET used_cycle_tokens = ?, used_day_tokens = ?, used_hour_tokens = ?
                WHERE channel_id = ?
                "#,
            )
            .bind(existing.limits.used_cycle_tokens)
            .bind(existing.limits.used_day_tokens)
            .bind(existing.limits.used_hour_tokens)
            .bind(clone.id)
            .execute(&self.pool)
            .await?;
        }
        self.get_channel(clone.id).await.map(PublicChannel::from)
    }

    pub async fn record_channel_health(
        &self,
        channel_id: i64,
        latency_ms: i64,
        last_error: Option<&str>,
    ) -> AppResult<()> {
        let result = sqlx::query(
            r#"
            UPDATE channels
            SET health_checked_at = datetime('now'), upstream_latency_ms = ?, last_error = ?,
                updated_at = datetime('now')
            WHERE id = ? AND deleted_at IS NULL
            "#,
        )
        .bind(latency_ms)
        .bind(last_error)
        .bind(channel_id)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            Err(AppError::NotFound)
        } else {
            Ok(())
        }
    }

    pub async fn list_prices(&self, user: &User) -> AppResult<Vec<ModelPrice>> {
        let rows = if user.role == "admin" {
            sqlx::query(
                r#"
                SELECT channel_id, model_pattern, input_price_per_1k, output_price_per_1k, cache_price_per_1k
                FROM model_prices
                ORDER BY channel_id IS NOT NULL, channel_id, id
                "#,
            )
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query(
                r#"
                SELECT p.channel_id, p.model_pattern, p.input_price_per_1k, p.output_price_per_1k, p.cache_price_per_1k
                FROM model_prices p
                LEFT JOIN channels c ON p.channel_id = c.id
                WHERE p.channel_id IS NULL OR c.owner_user_id = ?
                ORDER BY p.channel_id IS NOT NULL, p.channel_id, p.id
                "#,
            )
            .bind(user.id)
            .fetch_all(&self.pool)
            .await?
        };
        Ok(rows
            .iter()
            .map(|row| ModelPrice {
                channel_id: row.get("channel_id"),
                model_pattern: row.get("model_pattern"),
                input_price_per_1k: row.get("input_price_per_1k"),
                output_price_per_1k: row.get("output_price_per_1k"),
                cache_price_per_1k: row.get("cache_price_per_1k"),
            })
            .collect())
    }

    pub async fn price_book_for_channel(&self, channel_id: i64) -> AppResult<Vec<ModelPrice>> {
        let rows = sqlx::query(
            r#"
            SELECT channel_id, model_pattern, input_price_per_1k, output_price_per_1k, cache_price_per_1k
            FROM model_prices
            WHERE channel_id IS NULL OR channel_id = ?
            ORDER BY channel_id IS NULL, id
            "#,
        )
        .bind(channel_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .iter()
            .map(|row| ModelPrice {
                channel_id: row.get("channel_id"),
                model_pattern: row.get("model_pattern"),
                input_price_per_1k: row.get("input_price_per_1k"),
                output_price_per_1k: row.get("output_price_per_1k"),
                cache_price_per_1k: row.get("cache_price_per_1k"),
            })
            .collect())
    }

    pub async fn global_price_book(&self) -> AppResult<Vec<ModelPrice>> {
        let rows = sqlx::query(
            r#"
            SELECT channel_id, model_pattern, input_price_per_1k, output_price_per_1k, cache_price_per_1k
            FROM model_prices
            WHERE channel_id IS NULL
            ORDER BY id
            "#,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .iter()
            .map(|row| ModelPrice {
                channel_id: row.get("channel_id"),
                model_pattern: row.get("model_pattern"),
                input_price_per_1k: row.get("input_price_per_1k"),
                output_price_per_1k: row.get("output_price_per_1k"),
                cache_price_per_1k: row.get("cache_price_per_1k"),
            })
            .collect())
    }

    pub async fn refresh_channel_windows(&self) -> AppResult<()> {
        let now = Utc::now();
        let today = now.date_naive().to_string();
        let hour = now.format("%Y-%m-%dT%H").to_string();
        let day = now.day() as i64;
        sqlx::query(
            r#"
            UPDATE channel_limits
            SET used_day_tokens = 0, last_day_reset_at = ?
            WHERE last_day_reset_at IS NULL OR substr(last_day_reset_at, 1, 10) != ?
            "#,
        )
        .bind(now.to_rfc3339())
        .bind(&today)
        .execute(&self.pool)
        .await?;
        sqlx::query(
            r#"
            UPDATE channel_limits
            SET used_hour_tokens = 0, last_hour_reset_at = ?
            WHERE last_hour_reset_at IS NULL OR substr(last_hour_reset_at, 1, 13) != ?
            "#,
        )
        .bind(now.to_rfc3339())
        .bind(&hour)
        .execute(&self.pool)
        .await?;
        sqlx::query(
            r#"
            UPDATE channel_limits
            SET used_cycle_tokens = 0, last_cycle_reset_at = ?
            WHERE cycle_reset_day = ?
              AND (last_cycle_reset_at IS NULL OR substr(last_cycle_reset_at, 1, 10) != ?)
            "#,
        )
        .bind(now.to_rfc3339())
        .bind(day)
        .bind(&today)
        .execute(&self.pool)
        .await?;
        sqlx::query(
            r#"
            UPDATE channels
            SET status = CASE
                WHEN deleted_at IS NOT NULL THEN status
                WHEN enabled = 0 THEN status
                WHEN (SELECT used_cycle_tokens >= cycle_limit_tokens FROM channel_limits WHERE channel_id = channels.id) THEN 'monthly_exhausted'
                WHEN (SELECT used_day_tokens >= daily_limit_tokens OR used_hour_tokens >= hourly_limit_tokens FROM channel_limits WHERE channel_id = channels.id) THEN 'cooling'
                ELSE 'healthy'
              END,
              updated_at = datetime('now')
            "#,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn upsert_price(&self, price: &ModelPrice) -> AppResult<()> {
        if let Some(channel_id) = price.channel_id {
            let _ = self.get_channel(channel_id).await?;
            sqlx::query(
                r#"
                INSERT INTO model_prices(channel_id, model_pattern, input_price_per_1k, output_price_per_1k, cache_price_per_1k)
                VALUES (?, ?, ?, ?, ?)
                ON CONFLICT(channel_id, model_pattern) DO UPDATE SET
                  input_price_per_1k = excluded.input_price_per_1k,
                  output_price_per_1k = excluded.output_price_per_1k,
                  cache_price_per_1k = excluded.cache_price_per_1k
                "#,
            )
            .bind(channel_id)
            .bind(&price.model_pattern)
            .bind(price.input_price_per_1k)
            .bind(price.output_price_per_1k)
            .bind(price.cache_price_per_1k)
            .execute(&self.pool)
            .await?;
        } else {
            sqlx::query(
                r#"
                INSERT INTO model_prices(channel_id, model_pattern, input_price_per_1k, output_price_per_1k, cache_price_per_1k)
                VALUES (NULL, ?, ?, ?, ?)
                ON CONFLICT(model_pattern) WHERE channel_id IS NULL DO UPDATE SET
                  input_price_per_1k = excluded.input_price_per_1k,
                  output_price_per_1k = excluded.output_price_per_1k,
                  cache_price_per_1k = excluded.cache_price_per_1k
                "#,
            )
            .bind(&price.model_pattern)
            .bind(price.input_price_per_1k)
            .bind(price.output_price_per_1k)
            .bind(price.cache_price_per_1k)
            .execute(&self.pool)
            .await?;
        }
        Ok(())
    }

    pub async fn list_affinity_rules(&self) -> AppResult<Vec<AffinityRule>> {
        let rows = sqlx::query(
            r#"
            SELECT id, name, enabled, model_regex, request_path, user_agent_regex, key_source_type,
                   key_source_path, group_name, ttl_seconds, skip_retry_on_failure, switch_on_success
            FROM affinity_rules ORDER BY id DESC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.iter().map(affinity_rule_from_row).collect())
    }

    pub async fn create_affinity_rule(&self, input: AffinityRuleInput) -> AppResult<AffinityRule> {
        let result = sqlx::query(
            r#"
            INSERT INTO affinity_rules(
              name, enabled, model_regex, request_path, user_agent_regex, key_source_type,
              key_source_path, group_name, ttl_seconds, skip_retry_on_failure, switch_on_success
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&input.name)
        .bind(if input.enabled { 1 } else { 0 })
        .bind(&input.model_regex)
        .bind(&input.request_path)
        .bind(&input.user_agent_regex)
        .bind(&input.key_source_type)
        .bind(&input.key_source_path)
        .bind(&input.group_name)
        .bind(input.ttl_seconds)
        .bind(if input.skip_retry_on_failure { 1 } else { 0 })
        .bind(if input.switch_on_success { 1 } else { 0 })
        .execute(&self.pool)
        .await?;
        let id = result.last_insert_rowid();
        let rules = self.list_affinity_rules().await?;
        rules
            .into_iter()
            .find(|rule| rule.id == id)
            .ok_or(AppError::NotFound)
    }

    pub async fn get_affinity_binding(&self, cache_key: &str) -> AppResult<Option<(i64, String)>> {
        let row = sqlx::query(
            "SELECT channel_id, expires_at FROM affinity_bindings WHERE cache_key = ? AND expires_at > datetime('now')",
        )
        .bind(cache_key)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|row| (row.get("channel_id"), row.get("expires_at"))))
    }

    pub async fn set_affinity_binding(
        &self,
        cache_key: &str,
        rule_id: i64,
        channel_id: i64,
        ttl_seconds: i64,
    ) -> AppResult<String> {
        let expires_at = (Utc::now() + chrono::Duration::seconds(ttl_seconds)).to_rfc3339();
        sqlx::query(
            r#"
            INSERT INTO affinity_bindings(cache_key, rule_id, channel_id, expires_at, updated_at)
            VALUES (?, ?, ?, ?, datetime('now'))
            ON CONFLICT(cache_key) DO UPDATE SET
              rule_id = excluded.rule_id,
              channel_id = excluded.channel_id,
              expires_at = excluded.expires_at,
              updated_at = datetime('now')
            "#,
        )
        .bind(cache_key)
        .bind(rule_id)
        .bind(channel_id)
        .bind(&expires_at)
        .execute(&self.pool)
        .await?;
        Ok(expires_at)
    }

    pub async fn apply_ledger_event(&self, event: &LedgerEvent) -> AppResult<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            r#"
            INSERT OR IGNORE INTO ledger_entries(
              request_id, user_id, api_key_id, channel_id, provider_user_id, model, tokenizer,
              input_tokens, output_tokens, cache_tokens, input_price_per_1k, output_price_per_1k,
              cache_price_per_1k, surge_multiplier, fire_sale_discount, total_points,
              provider_points, status, formula_note
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&event.request_id)
        .bind(event.user_id)
        .bind(event.api_key_id)
        .bind(event.channel_id)
        .bind(event.provider_user_id)
        .bind(&event.model)
        .bind(&event.tokenizer)
        .bind(event.usage.input_tokens)
        .bind(event.usage.output_tokens)
        .bind(event.usage.cache_tokens)
        .bind(event.price.input_price_per_1k)
        .bind(event.price.output_price_per_1k)
        .bind(event.price.cache_price_per_1k)
        .bind(event.surge_multiplier)
        .bind(event.fire_sale_discount)
        .bind(event.total_points)
        .bind(event.provider_points)
        .bind(&event.status)
        .bind(&event.formula_note)
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE users SET points_balance = points_balance - ? WHERE id = ?")
            .bind(event.total_points)
            .bind(event.user_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE users SET points_balance = points_balance + ? WHERE id = ?")
            .bind(event.provider_points)
            .bind(event.provider_user_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE api_keys SET spent_points = spent_points + ? WHERE id = ?")
            .bind(event.total_points)
            .bind(event.api_key_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "UPDATE channel_limits SET used_cycle_tokens = used_cycle_tokens + ?, used_day_tokens = used_day_tokens + ?, used_hour_tokens = used_hour_tokens + ?, updated_at = datetime('now') WHERE channel_id = ?",
        )
        .bind(event.usage.total())
        .bind(event.usage.total())
        .bind(event.usage.total())
        .bind(event.channel_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn list_ledger(&self, user_id: Option<i64>) -> AppResult<Vec<serde_json::Value>> {
        let rows = if let Some(user_id) = user_id {
            sqlx::query("SELECT * FROM ledger_entries WHERE user_id = ? ORDER BY id DESC LIMIT 200")
                .bind(user_id)
                .fetch_all(&self.pool)
                .await?
        } else {
            sqlx::query("SELECT * FROM ledger_entries ORDER BY id DESC LIMIT 200")
                .fetch_all(&self.pool)
                .await?
        };
        Ok(rows
            .iter()
            .map(|row| {
                json!({
                    "id": row.get::<i64, _>("id"),
                    "request_id": row.get::<String, _>("request_id"),
                    "user_id": row.get::<i64, _>("user_id"),
                    "channel_id": row.get::<i64, _>("channel_id"),
                    "model": row.get::<String, _>("model"),
                    "tokenizer": row.get::<String, _>("tokenizer"),
                    "input_tokens": row.get::<i64, _>("input_tokens"),
                    "output_tokens": row.get::<i64, _>("output_tokens"),
                    "cache_tokens": row.get::<i64, _>("cache_tokens"),
                    "total_points": row.get::<f64, _>("total_points"),
                    "provider_points": row.get::<f64, _>("provider_points"),
                    "status": row.get::<String, _>("status"),
                    "formula_note": row.get::<String, _>("formula_note"),
                    "created_at": row.get::<String, _>("created_at"),
                })
            })
            .collect())
    }

    pub async fn dashboard(
        &self,
        surge_multiplier: f64,
        surge_state: &str,
    ) -> AppResult<DashboardSummary> {
        let users: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
            .fetch_one(&self.pool)
            .await?;
        let channels: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM channels WHERE deleted_at IS NULL")
                .fetch_one(&self.pool)
                .await?;
        let enabled_channels: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM channels WHERE enabled = 1 AND deleted_at IS NULL",
        )
        .fetch_one(&self.pool)
        .await?;
        let available_tokens: (i64,) = sqlx::query_as(
            r#"
            SELECT COALESCE(SUM(l.cycle_limit_tokens - l.used_cycle_tokens), 0)
            FROM channel_limits l JOIN channels c ON c.id = l.channel_id
            WHERE c.deleted_at IS NULL
            "#,
        )
        .fetch_one(&self.pool)
        .await?;
        let spent_points_today: (f64,) = sqlx::query_as(
            "SELECT COALESCE(SUM(total_points), 0.0) FROM ledger_entries WHERE created_at >= date('now')",
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(DashboardSummary {
            users: users.0,
            channels: channels.0,
            enabled_channels: enabled_channels.0,
            available_tokens: available_tokens.0,
            spent_points_today: spent_points_today.0,
            surge_multiplier,
            surge_state: surge_state.to_string(),
        })
    }

    pub async fn list_settings(&self) -> AppResult<Vec<SettingRecord>> {
        let rows = sqlx::query("SELECT key, value, updated_at FROM system_settings ORDER BY key")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows
            .iter()
            .map(|row| SettingRecord {
                key: row.get("key"),
                value: row.get("value"),
                updated_at: row.get("updated_at"),
            })
            .collect())
    }

    pub async fn upsert_settings(&self, settings: &[SettingUpdate]) -> AppResult<()> {
        let mut tx = self.pool.begin().await?;
        for setting in settings {
            sqlx::query(
                r#"
                INSERT INTO system_settings(key, value, updated_at)
                VALUES (?, ?, datetime('now'))
                ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = datetime('now')
                "#,
            )
            .bind(&setting.key)
            .bind(&setting.value)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn set_anonymous_leaderboard(&self, user_id: i64, enabled: bool) -> AppResult<User> {
        sqlx::query(
            "UPDATE users SET anonymous_leaderboard = ?, updated_at = datetime('now') WHERE id = ?",
        )
        .bind(if enabled { 1 } else { 0 })
        .bind(user_id)
        .execute(&self.pool)
        .await?;
        self.get_user(user_id).await
    }

    pub async fn transfer_points(
        &self,
        from_user_id: i64,
        to_user_id: i64,
        points: f64,
        memo: Option<&str>,
    ) -> AppResult<()> {
        if from_user_id == to_user_id {
            return Err(AppError::BadRequest(
                "cannot transfer to yourself".to_string(),
            ));
        }
        if points <= 0.0 {
            return Err(AppError::BadRequest(
                "transfer points must be positive".to_string(),
            ));
        }
        let mut tx = self.pool.begin().await?;
        let balance: f64 = sqlx::query_scalar("SELECT points_balance FROM users WHERE id = ?")
            .bind(from_user_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(AppError::NotFound)?;
        let exists: Option<i64> = sqlx::query_scalar("SELECT id FROM users WHERE id = ?")
            .bind(to_user_id)
            .fetch_optional(&mut *tx)
            .await?;
        if exists.is_none() {
            return Err(AppError::NotFound);
        }
        if balance < points {
            return Err(AppError::BadRequest("insufficient points".to_string()));
        }
        sqlx::query("UPDATE users SET points_balance = points_balance - ? WHERE id = ?")
            .bind(points)
            .bind(from_user_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE users SET points_balance = points_balance + ? WHERE id = ?")
            .bind(points)
            .bind(to_user_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "INSERT INTO transfers(from_user_id, to_user_id, points, memo) VALUES (?, ?, ?, ?)",
        )
        .bind(from_user_id)
        .bind(to_user_id)
        .bind(points)
        .bind(memo)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn list_transfers(&self, user_id: i64) -> AppResult<Vec<serde_json::Value>> {
        let rows = sqlx::query(
            r#"
            SELECT t.id, t.from_user_id, t.to_user_id, t.points, t.memo, t.created_at,
                   fu.display_name AS from_name, tu.display_name AS to_name
            FROM transfers t
            JOIN users fu ON fu.id = t.from_user_id
            JOIN users tu ON tu.id = t.to_user_id
            WHERE t.from_user_id = ? OR t.to_user_id = ?
            ORDER BY t.id DESC LIMIT 100
            "#,
        )
        .bind(user_id)
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .iter()
            .map(|row| {
                json!({
                    "id": row.get::<i64, _>("id"),
                    "from_user_id": row.get::<i64, _>("from_user_id"),
                    "to_user_id": row.get::<i64, _>("to_user_id"),
                    "from_name": row.get::<String, _>("from_name"),
                    "to_name": row.get::<String, _>("to_name"),
                    "points": row.get::<f64, _>("points"),
                    "memo": row.get::<Option<String>, _>("memo"),
                    "created_at": row.get::<String, _>("created_at"),
                })
            })
            .collect())
    }

    pub async fn create_red_packet(
        &self,
        creator_user_id: i64,
        phrase: &str,
        total_points: f64,
        total_parts: i64,
        mode: &str,
    ) -> AppResult<()> {
        if phrase.trim().len() < 3 {
            return Err(AppError::BadRequest(
                "phrase must be at least 3 characters".to_string(),
            ));
        }
        if total_points <= 0.0 || total_parts <= 0 {
            return Err(AppError::BadRequest(
                "red packet points and parts must be positive".to_string(),
            ));
        }
        if !matches!(mode, "even" | "lucky") {
            return Err(AppError::BadRequest(
                "mode must be even or lucky".to_string(),
            ));
        }
        let mut tx = self.pool.begin().await?;
        let balance: f64 = sqlx::query_scalar("SELECT points_balance FROM users WHERE id = ?")
            .bind(creator_user_id)
            .fetch_one(&mut *tx)
            .await?;
        if balance < total_points {
            return Err(AppError::BadRequest("insufficient points".to_string()));
        }
        sqlx::query("UPDATE users SET points_balance = points_balance - ? WHERE id = ?")
            .bind(total_points)
            .bind(creator_user_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "INSERT INTO red_packets(creator_user_id, phrase, total_points, remaining_points, total_parts, mode) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(creator_user_id)
        .bind(phrase)
        .bind(total_points)
        .bind(total_points)
        .bind(total_parts)
        .bind(mode)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn claim_red_packet(&self, user_id: i64, phrase: &str) -> AppResult<f64> {
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query(
            "SELECT id, remaining_points, total_parts, claimed_parts, mode FROM red_packets WHERE phrase = ?",
        )
        .bind(phrase)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(AppError::NotFound)?;
        let packet_id: i64 = row.get("id");
        let remaining_points: f64 = row.get("remaining_points");
        let total_parts: i64 = row.get("total_parts");
        let claimed_parts: i64 = row.get("claimed_parts");
        let mode: String = row.get("mode");
        let already: Option<i64> = sqlx::query_scalar(
            "SELECT id FROM red_packet_claims WHERE red_packet_id = ? AND user_id = ?",
        )
        .bind(packet_id)
        .bind(user_id)
        .fetch_optional(&mut *tx)
        .await?;
        if already.is_some() {
            return Err(AppError::BadRequest(
                "red packet already claimed".to_string(),
            ));
        }
        let remaining_parts = total_parts - claimed_parts;
        if remaining_parts <= 0 || remaining_points <= 0.0 {
            return Err(AppError::BadRequest("red packet exhausted".to_string()));
        }
        let points = if remaining_parts == 1 || mode == "even" {
            remaining_points / remaining_parts as f64
        } else {
            let average = remaining_points / remaining_parts as f64;
            let max = (average * 2.0).min(remaining_points - 0.0001);
            rand::rng().random_range(0.0001..max)
        };
        let points = (points * 10000.0).floor() / 10000.0;
        let result = sqlx::query(
            "UPDATE red_packets SET remaining_points = remaining_points - ?, claimed_parts = claimed_parts + 1 WHERE id = ? AND claimed_parts < total_parts AND remaining_points >= ?",
        )
        .bind(points)
        .bind(packet_id)
        .bind(points)
        .execute(&mut *tx)
        .await?;
        if result.rows_affected() == 0 {
            return Err(AppError::BadRequest("red packet exhausted".to_string()));
        }
        sqlx::query(
            "INSERT INTO red_packet_claims(red_packet_id, user_id, points) VALUES (?, ?, ?)",
        )
        .bind(packet_id)
        .bind(user_id)
        .bind(points)
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE users SET points_balance = points_balance + ? WHERE id = ?")
            .bind(points)
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(points)
    }

    pub async fn list_red_packets(&self, user_id: i64) -> AppResult<Vec<serde_json::Value>> {
        let rows = sqlx::query(
            r#"
            SELECT id, phrase, total_points, remaining_points, total_parts, claimed_parts, mode, created_at
            FROM red_packets WHERE creator_user_id = ? ORDER BY id DESC LIMIT 100
            "#,
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .iter()
            .map(|row| {
                json!({
                    "id": row.get::<i64, _>("id"),
                    "phrase": row.get::<String, _>("phrase"),
                    "total_points": row.get::<f64, _>("total_points"),
                    "remaining_points": row.get::<f64, _>("remaining_points"),
                    "total_parts": row.get::<i64, _>("total_parts"),
                    "claimed_parts": row.get::<i64, _>("claimed_parts"),
                    "mode": row.get::<String, _>("mode"),
                    "created_at": row.get::<String, _>("created_at"),
                })
            })
            .collect())
    }

    pub async fn leaderboards(
        &self,
        period: LeaderboardPeriod,
        timezone: Option<&str>,
    ) -> AppResult<serde_json::Value> {
        let window_start = leaderboard_window_start(period, timezone)?;
        let providers = sqlx::query(
            r#"
            SELECT u.id, u.display_name, u.anonymous_leaderboard,
                   COALESCE(SUM(l.input_tokens + l.output_tokens + l.cache_tokens), 0) AS score
            FROM ledger_entries l JOIN users u ON u.id = l.provider_user_id
            WHERE l.created_at >= ? AND l.status = 'success'
            GROUP BY u.id ORDER BY score DESC LIMIT 20
            "#,
        )
        .bind(&window_start)
        .fetch_all(&self.pool)
        .await?;
        let consumers = sqlx::query(
            r#"
            SELECT u.id, u.display_name, u.anonymous_leaderboard,
                   COALESCE(SUM(l.total_points), 0) AS score
            FROM ledger_entries l JOIN users u ON u.id = l.user_id
            WHERE l.created_at >= ? AND l.status = 'success'
            GROUP BY u.id ORDER BY score DESC LIMIT 20
            "#,
        )
        .bind(&window_start)
        .fetch_all(&self.pool)
        .await?;
        Ok(json!({
            "period": period.as_str(),
            "window_start": window_start,
            "timezone": normalized_leaderboard_timezone(timezone),
            "providers": providers.iter().map(leaderboard_row).collect::<Vec<_>>(),
            "consumers": consumers.iter().map(leaderboard_row).collect::<Vec<_>>(),
        }))
    }
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ChannelInput {
    pub name: String,
    pub provider: String,
    pub base_url: String,
    pub api_key_secret: String,
    pub models: Vec<String>,
    pub enabled: bool,
    pub cycle_limit_tokens: i64,
    pub cycle_reset_day: i64,
    pub daily_limit_tokens: i64,
    pub hourly_limit_tokens: i64,
    pub fire_sale_days_before: i64,
    pub fire_sale_remaining_pct: f64,
    pub fire_sale_discount: f64,
    pub provider_share: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChannelUpdateInput {
    pub name: String,
    pub provider: String,
    pub base_url: String,
    pub api_key_secret: Option<String>,
    pub models: Vec<String>,
    pub enabled: bool,
    pub cycle_limit_tokens: i64,
    pub cycle_reset_day: i64,
    pub daily_limit_tokens: i64,
    pub hourly_limit_tokens: i64,
    pub fire_sale_days_before: i64,
    pub fire_sale_remaining_pct: f64,
    pub fire_sale_discount: f64,
    pub provider_share: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ApiKeyUpdateInput {
    pub name: String,
    pub enabled: bool,
    pub spend_limit_points: Option<f64>,
    pub expires_at: Option<String>,
    pub allowed_models: Vec<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct AffinityRuleInput {
    pub name: String,
    pub enabled: bool,
    pub model_regex: Option<String>,
    pub request_path: Option<String>,
    pub user_agent_regex: Option<String>,
    pub key_source_type: String,
    pub key_source_path: String,
    pub group_name: String,
    pub ttl_seconds: i64,
    pub skip_retry_on_failure: bool,
    pub switch_on_success: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SettingRecord {
    pub key: String,
    pub value: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SettingUpdate {
    pub key: String,
    pub value: String,
}

fn api_key_from_row(row: &sqlx::sqlite::SqliteRow) -> ApiKeyRecord {
    ApiKeyRecord {
        id: row.get("id"),
        user_id: row.get("user_id"),
        name: row.get("name"),
        key_prefix: row.get("key_prefix"),
        enabled: row.get::<i64, _>("enabled") != 0,
        spend_limit_points: row.get("spend_limit_points"),
        spent_points: row.get("spent_points"),
        expires_at: row.get("expires_at"),
        allowed_models: row
            .get::<Option<String>, _>("allowed_models_json")
            .as_deref()
            .map(json_array_to_strings)
            .unwrap_or_default(),
        last_used_at: row.get("last_used_at"),
    }
}

fn channel_from_row(row: &sqlx::sqlite::SqliteRow) -> AppResult<Channel> {
    Ok(Channel {
        id: row.get("id"),
        owner_user_id: row.get("owner_user_id"),
        name: row.get("name"),
        provider: crate::models::ProviderKind::try_from(row.get::<String, _>("provider").as_str())?,
        base_url: row.get("base_url"),
        api_key_secret: row.get("api_key_secret"),
        models: json_array_to_strings(&row.get::<String, _>("models_json")),
        enabled: row.get::<i64, _>("enabled") != 0,
        status: row.get("status"),
        health_checked_at: row.get("health_checked_at"),
        upstream_latency_ms: row.get("upstream_latency_ms"),
        last_error: row.get("last_error"),
        limits: ChannelLimits {
            cycle_limit_tokens: row.get("cycle_limit_tokens"),
            cycle_reset_day: row.get("cycle_reset_day"),
            daily_limit_tokens: row.get("daily_limit_tokens"),
            hourly_limit_tokens: row.get("hourly_limit_tokens"),
            used_cycle_tokens: row.get("used_cycle_tokens"),
            used_day_tokens: row.get("used_day_tokens"),
            used_hour_tokens: row.get("used_hour_tokens"),
            fire_sale_days_before: row.get("fire_sale_days_before"),
            fire_sale_remaining_pct: row.get("fire_sale_remaining_pct"),
            fire_sale_discount: row.get("fire_sale_discount"),
            provider_share: row.get("provider_share"),
        },
    })
}

fn affinity_rule_from_row(row: &sqlx::sqlite::SqliteRow) -> AffinityRule {
    AffinityRule {
        id: row.get("id"),
        name: row.get("name"),
        enabled: row.get::<i64, _>("enabled") != 0,
        model_regex: row.get("model_regex"),
        request_path: row.get("request_path"),
        user_agent_regex: row.get("user_agent_regex"),
        key_source_type: row.get("key_source_type"),
        key_source_path: row.get("key_source_path"),
        group_name: row.get("group_name"),
        ttl_seconds: row.get("ttl_seconds"),
        skip_retry_on_failure: row.get::<i64, _>("skip_retry_on_failure") != 0,
        switch_on_success: row.get::<i64, _>("switch_on_success") != 0,
    }
}

fn leaderboard_row(row: &sqlx::sqlite::SqliteRow) -> serde_json::Value {
    let anonymous = row.get::<i64, _>("anonymous_leaderboard") != 0;
    let id: i64 = row.get("id");
    let score = row
        .try_get::<f64, _>("score")
        .unwrap_or_else(|_| row.get::<i64, _>("score") as f64);
    json!({
        "user_id": if anonymous { serde_json::Value::Null } else { json!(id) },
        "name": if anonymous {
            format!("Anonymous #{}", id % 10000)
        } else {
            row.get::<String, _>("display_name")
        },
        "score": score,
    })
}

pub fn now_rfc3339() -> String {
    DateTime::<Utc>::from(std::time::SystemTime::now()).to_rfc3339()
}

fn validate_api_key_name(name: &str) -> AppResult<()> {
    if name.trim().is_empty() || name.chars().count() > 80 {
        return Err(AppError::BadRequest(
            "api key name must be 1-80 characters".to_string(),
        ));
    }
    Ok(())
}

fn validate_spend_limit(spend_limit_points: Option<f64>) -> AppResult<()> {
    if let Some(limit) = spend_limit_points
        && (!limit.is_finite() || limit < 0.0)
    {
        return Err(AppError::BadRequest(
            "api key spend limit must be non-negative".to_string(),
        ));
    }
    Ok(())
}

fn validate_batch_ids(ids: &[i64]) -> AppResult<()> {
    if ids.is_empty() {
        return Err(AppError::BadRequest("ids cannot be empty".to_string()));
    }
    if ids.len() > 100 {
        return Err(AppError::BadRequest(
            "batch operation accepts at most 100 ids".to_string(),
        ));
    }
    if ids.iter().any(|id| *id <= 0) {
        return Err(AppError::BadRequest(
            "ids must be positive integers".to_string(),
        ));
    }
    Ok(())
}

fn normalize_optional_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn normalize_models_json(models: &[String]) -> AppResult<String> {
    let mut normalized = Vec::new();
    for model in models {
        let model = model.trim();
        if model.is_empty() {
            continue;
        }
        if model.len() > 255 {
            return Err(AppError::BadRequest(format!(
                "model pattern too long: {model}"
            )));
        }
        if !normalized.iter().any(|existing: &String| existing == model) {
            normalized.push(model.to_string());
        }
    }
    serde_json::to_string(&normalized).map_err(|err| AppError::Anyhow(anyhow::anyhow!(err)))
}

fn validate_channel_input(input: &ChannelInput, require_key: bool) -> AppResult<()> {
    validate_channel_fields(
        &input.name,
        &input.provider,
        &input.base_url,
        if require_key {
            Some(input.api_key_secret.as_str())
        } else {
            None
        },
        &input.models,
        input.cycle_limit_tokens,
        input.cycle_reset_day,
        input.daily_limit_tokens,
        input.hourly_limit_tokens,
        input.fire_sale_days_before,
        input.fire_sale_remaining_pct,
        input.fire_sale_discount,
        input.provider_share,
    )
}

fn validate_channel_update(input: &ChannelUpdateInput) -> AppResult<()> {
    let api_key_secret = input.api_key_secret.as_deref().and_then(|value| {
        if value.trim().is_empty() {
            None
        } else {
            Some(value)
        }
    });
    validate_channel_fields(
        &input.name,
        &input.provider,
        &input.base_url,
        api_key_secret,
        &input.models,
        input.cycle_limit_tokens,
        input.cycle_reset_day,
        input.daily_limit_tokens,
        input.hourly_limit_tokens,
        input.fire_sale_days_before,
        input.fire_sale_remaining_pct,
        input.fire_sale_discount,
        input.provider_share,
    )
}

#[allow(clippy::too_many_arguments)]
fn validate_channel_fields(
    name: &str,
    provider: &str,
    base_url: &str,
    api_key_secret: Option<&str>,
    models: &[String],
    cycle_limit_tokens: i64,
    cycle_reset_day: i64,
    daily_limit_tokens: i64,
    hourly_limit_tokens: i64,
    fire_sale_days_before: i64,
    fire_sale_remaining_pct: f64,
    fire_sale_discount: f64,
    provider_share: f64,
) -> AppResult<()> {
    if name.trim().is_empty() || name.chars().count() > 120 {
        return Err(AppError::BadRequest(
            "channel name must be 1-120 characters".to_string(),
        ));
    }
    crate::models::ProviderKind::try_from(provider)
        .map_err(|err| AppError::BadRequest(err.to_string()))?;
    let parsed_url = reqwest::Url::parse(base_url.trim()).map_err(|_| {
        AppError::BadRequest("channel base_url must be an absolute URL".to_string())
    })?;
    if !matches!(parsed_url.scheme(), "http" | "https") {
        return Err(AppError::BadRequest(
            "channel base_url must use http or https".to_string(),
        ));
    }
    if let Some(secret) = api_key_secret
        && secret.trim().is_empty()
    {
        return Err(AppError::BadRequest(
            "channel api key cannot be empty".to_string(),
        ));
    }
    let _ = normalize_models_json(models)?;
    if cycle_limit_tokens <= 0 || daily_limit_tokens <= 0 || hourly_limit_tokens <= 0 {
        return Err(AppError::BadRequest(
            "channel token limits must be positive".to_string(),
        ));
    }
    if !(1..=28).contains(&cycle_reset_day) {
        return Err(AppError::BadRequest(
            "cycle reset day must be between 1 and 28".to_string(),
        ));
    }
    if daily_limit_tokens > cycle_limit_tokens || hourly_limit_tokens > daily_limit_tokens {
        return Err(AppError::BadRequest(
            "hourly <= daily <= cycle limits must hold".to_string(),
        ));
    }
    if fire_sale_days_before < 0
        || !fire_sale_remaining_pct.is_finite()
        || !(0.0..=1.0).contains(&fire_sale_remaining_pct)
        || !fire_sale_discount.is_finite()
        || !(0.0..=1.0).contains(&fire_sale_discount)
        || !provider_share.is_finite()
        || !(0.0..=1.0).contains(&provider_share)
    {
        return Err(AppError::BadRequest(
            "channel economy knobs must be finite ratios in range".to_string(),
        ));
    }
    Ok(())
}

fn leaderboard_window_start(
    period: LeaderboardPeriod,
    timezone: Option<&str>,
) -> AppResult<String> {
    let now = Utc::now();
    if let Some(name) = timezone.and_then(non_empty_timezone) {
        let tz: Tz = name
            .parse()
            .map_err(|_| AppError::BadRequest(format!("invalid leaderboard timezone: {name}")))?;
        let local = now.with_timezone(&tz);
        let start_date = match period {
            LeaderboardPeriod::Day => local.date_naive(),
            LeaderboardPeriod::Month => local
                .date_naive()
                .with_day(1)
                .ok_or_else(|| AppError::BadRequest("invalid leaderboard month".to_string()))?,
        };
        let local_start = tz
            .from_local_datetime(
                &start_date
                    .and_hms_opt(0, 0, 0)
                    .ok_or_else(|| AppError::BadRequest("invalid leaderboard day".to_string()))?,
            )
            .earliest()
            .ok_or_else(|| {
                AppError::BadRequest("invalid leaderboard timezone boundary".to_string())
            })?;
        Ok(sqlite_utc_datetime(local_start.with_timezone(&Utc)))
    } else {
        let local = Local::now();
        let start = match period {
            LeaderboardPeriod::Day => local.date_naive(),
            LeaderboardPeriod::Month => local
                .date_naive()
                .with_day(1)
                .ok_or_else(|| AppError::BadRequest("invalid leaderboard month".to_string()))?,
        };
        let local_start = Local
            .from_local_datetime(
                &start
                    .and_hms_opt(0, 0, 0)
                    .ok_or_else(|| AppError::BadRequest("invalid leaderboard day".to_string()))?,
            )
            .earliest()
            .ok_or_else(|| {
                AppError::BadRequest("invalid server local timezone boundary".to_string())
            })?;
        Ok(sqlite_utc_datetime(local_start.with_timezone(&Utc)))
    }
}

fn sqlite_utc_datetime(datetime: DateTime<Utc>) -> String {
    datetime.format("%Y-%m-%d %H:%M:%S").to_string()
}

fn normalized_leaderboard_timezone(timezone: Option<&str>) -> String {
    timezone
        .and_then(non_empty_timezone)
        .map(ToString::to_string)
        .unwrap_or_else(|| "server-local".to_string())
}

fn non_empty_timezone(timezone: &str) -> Option<&str> {
    let trimmed = timezone.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}
