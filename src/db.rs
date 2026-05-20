use std::{
    collections::{HashMap, HashSet},
    str::FromStr,
    time::Duration,
};

use chrono::{DateTime, Datelike, Local, NaiveDate, NaiveDateTime, TimeZone, Timelike, Utc};
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
        AffinityRule, ApiKeyRecord, Channel, ChannelHealthWindow, ChannelLimits,
        ChannelQuotaWindow, GatewayReservation, LedgerEvent, ModelPrice, PublicChannel, User,
        json_array_to_strings,
    },
    settings::{RuntimeSettings, SETTING_DEFAULTS, validate_setting_value},
};

const CHANNEL_HEALTH_WINDOW_SECONDS: i64 = 30 * 60;
const CHANNEL_HEALTH_WINDOW_COUNT: usize = 48;
const CHANNEL_HEALTH_RETENTION_DAYS: i64 = 7;

const MANAGED_USER_SELECT: &str = r#"
    SELECT u.id, u.email, u.role, u.display_name, u.points_balance,
           u.anonymous_leaderboard, u.enabled, u.disabled_at, u.created_at, u.updated_at,
           COALESCE((SELECT COUNT(*) FROM api_keys k WHERE k.user_id = u.id AND k.deleted_at IS NULL), 0) AS api_key_count,
           COALESCE((SELECT COUNT(*) FROM channels c WHERE c.owner_user_id = u.id AND c.deleted_at IS NULL), 0) AS channel_count,
           COALESCE((SELECT SUM(l.total_points) FROM ledger_entries l WHERE l.user_id = u.id), 0.0) AS total_spent_points,
           COALESCE((SELECT SUM(l.provider_points) FROM ledger_entries l WHERE l.provider_user_id = u.id), 0.0) AS total_provider_points
    FROM users u
    ORDER BY u.id DESC
"#;

const MANAGED_USER_SELECT_BY_ID: &str = r#"
    SELECT u.id, u.email, u.role, u.display_name, u.points_balance,
           u.anonymous_leaderboard, u.enabled, u.disabled_at, u.created_at, u.updated_at,
           COALESCE((SELECT COUNT(*) FROM api_keys k WHERE k.user_id = u.id AND k.deleted_at IS NULL), 0) AS api_key_count,
           COALESCE((SELECT COUNT(*) FROM channels c WHERE c.owner_user_id = u.id AND c.deleted_at IS NULL), 0) AS channel_count,
           COALESCE((SELECT SUM(l.total_points) FROM ledger_entries l WHERE l.user_id = u.id), 0.0) AS total_spent_points,
           COALESCE((SELECT SUM(l.provider_points) FROM ledger_entries l WHERE l.provider_user_id = u.id), 0.0) AS total_provider_points
    FROM users u
    WHERE u.id = ?
"#;

#[derive(Clone)]
pub struct Database {
    pub pool: SqlitePool,
}

#[derive(Debug, Clone)]
pub struct ChannelHealthEventInput<'a> {
    pub channel_id: i64,
    pub request_id: Option<&'a str>,
    pub status: &'a str,
    pub http_status: Option<i64>,
    pub ttft_ms: Option<i64>,
    pub total_latency_ms: Option<i64>,
    pub error: Option<&'a str>,
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

#[derive(Debug, Clone, Serialize)]
pub struct ManagedUser {
    pub id: i64,
    pub email: String,
    pub role: String,
    pub display_name: String,
    pub points_balance: f64,
    pub anonymous_leaderboard: bool,
    pub enabled: bool,
    pub disabled_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub api_key_count: i64,
    pub channel_count: i64,
    pub total_spent_points: f64,
    pub total_provider_points: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ManagedUserCreateInput {
    pub email: String,
    pub password: String,
    pub role: String,
    pub display_name: Option<String>,
    pub points_balance: Option<f64>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ManagedUserUpdateInput {
    pub email: String,
    pub role: String,
    pub display_name: String,
    pub points_balance: f64,
    pub enabled: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PasswordResetInput {
    pub password: String,
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
        let db = Self { pool };
        db.ensure_setting_defaults().await?;
        Ok(db)
    }

    pub async fn ensure_setting_defaults(&self) -> AppResult<()> {
        let mut tx = self.pool.begin().await?;
        for (key, value) in SETTING_DEFAULTS {
            sqlx::query("INSERT OR IGNORE INTO system_settings(key, value) VALUES (?, ?)")
                .bind(key)
                .bind(value)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn runtime_settings(&self) -> AppResult<RuntimeSettings> {
        let rows = sqlx::query("SELECT key, value FROM system_settings")
            .fetch_all(&self.pool)
            .await?;
        let values = rows
            .iter()
            .map(|row| (row.get::<String, _>("key"), row.get::<String, _>("value")))
            .collect::<HashMap<_, _>>();
        RuntimeSettings::from_map(&values)
    }

    pub async fn bootstrap_admin(&self, email: &str, password: &str) -> AppResult<()> {
        let existing: Option<(i64,)> = sqlx::query_as("SELECT id FROM users WHERE email = ?")
            .bind(email)
            .fetch_optional(&self.pool)
            .await?;
        if existing.is_some() {
            return Ok(());
        }
        let settings = self.runtime_settings().await?;
        let password_hash = hash_password(password)?;
        sqlx::query(
            "INSERT INTO users(email, password_hash, role, display_name, points_balance) VALUES (?, ?, 'admin', 'Admin', ?)",
        )
        .bind(email)
        .bind(password_hash)
        .bind(settings.initial_admin_points)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn create_managed_user(
        &self,
        input: ManagedUserCreateInput,
    ) -> AppResult<ManagedUser> {
        validate_email(&input.email)?;
        validate_password(&input.password)?;
        validate_role(&input.role)?;
        let display_name = match input.display_name.as_deref().map(str::trim) {
            Some(name) if !name.is_empty() => name.to_string(),
            _ => default_display_name(&input.email),
        };
        validate_display_name(&display_name)?;
        let points_balance = match input.points_balance {
            Some(points) => {
                validate_points_balance(points)?;
                points
            }
            None => {
                let settings = self.runtime_settings().await?;
                if input.role == "admin" {
                    settings.initial_admin_points
                } else {
                    settings.initial_user_points
                }
            }
        };
        let enabled = input.enabled.unwrap_or(true);
        let password_hash = hash_password(&input.password)?;
        let result = sqlx::query(
            r#"
            INSERT INTO users(
              email, password_hash, role, display_name, points_balance, enabled, disabled_at
            ) VALUES (?, ?, ?, ?, ?, ?, CASE WHEN ? = 1 THEN NULL ELSE datetime('now') END)
            "#,
        )
        .bind(input.email.trim())
        .bind(password_hash)
        .bind(&input.role)
        .bind(display_name)
        .bind(points_balance)
        .bind(if enabled { 1 } else { 0 })
        .bind(if enabled { 1 } else { 0 })
        .execute(&self.pool)
        .await?;
        self.get_managed_user(result.last_insert_rowid()).await
    }

    pub async fn create_user(
        &self,
        email: &str,
        password: &str,
        display_name: &str,
    ) -> AppResult<User> {
        validate_email(email)?;
        validate_password(password)?;
        validate_display_name(display_name)?;
        let settings = self.runtime_settings().await?;
        let password_hash = hash_password(password)?;
        let result = sqlx::query(
            "INSERT INTO users(email, password_hash, role, display_name, points_balance) VALUES (?, ?, 'user', ?, ?)",
        )
        .bind(email.trim())
        .bind(password_hash)
        .bind(display_name.trim())
        .bind(settings.initial_user_points)
        .execute(&self.pool)
        .await?;
        self.get_user(result.last_insert_rowid()).await
    }

    pub async fn find_user_with_hash(&self, email: &str) -> AppResult<Option<(User, String)>> {
        let row = sqlx::query(
            "SELECT id, email, password_hash, role, display_name, points_balance, anonymous_leaderboard, enabled FROM users WHERE email = ?",
        )
        .bind(email)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|row| (user_from_row(&row), row.get("password_hash"))))
    }

    pub async fn get_user(&self, id: i64) -> AppResult<User> {
        let row = sqlx::query(
            "SELECT id, email, role, display_name, points_balance, anonymous_leaderboard, enabled FROM users WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(AppError::NotFound)?;
        Ok(user_from_row(&row))
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
        let user = self.get_user(row.get("user_id")).await?;
        if user.enabled {
            Ok(user)
        } else {
            Err(AppError::Unauthorized)
        }
    }

    pub async fn list_managed_users(&self) -> AppResult<Vec<ManagedUser>> {
        let rows = sqlx::query(MANAGED_USER_SELECT)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.iter().map(managed_user_from_row).collect())
    }

    pub async fn get_managed_user(&self, id: i64) -> AppResult<ManagedUser> {
        let row = sqlx::query(MANAGED_USER_SELECT_BY_ID)
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or(AppError::NotFound)?;
        Ok(managed_user_from_row(&row))
    }

    pub async fn update_managed_user(
        &self,
        actor_id: i64,
        id: i64,
        input: ManagedUserUpdateInput,
    ) -> AppResult<ManagedUser> {
        validate_email(&input.email)?;
        validate_role(&input.role)?;
        validate_display_name(&input.display_name)?;
        validate_points_balance(input.points_balance)?;
        let current = self.get_user(id).await?;
        if current.role == "admin"
            && input.role != "admin"
            && self.enabled_admin_count().await? <= 1
        {
            return Err(AppError::BadRequest(
                "cannot demote the last enabled admin".to_string(),
            ));
        }
        if current.role == "admin"
            && current.enabled
            && !input.enabled
            && self.enabled_admin_count().await? <= 1
        {
            return Err(AppError::BadRequest(
                "cannot disable the last enabled admin".to_string(),
            ));
        }
        if actor_id == id && (!input.enabled || input.role != "admin") {
            return Err(AppError::BadRequest(
                "cannot remove your own admin access".to_string(),
            ));
        }
        let result = sqlx::query(
            r#"
            UPDATE users
            SET email = ?, role = ?, display_name = ?, points_balance = ?, enabled = ?,
                disabled_at = CASE
                  WHEN ? = 1 THEN NULL
                  WHEN enabled = 0 THEN disabled_at
                  ELSE datetime('now')
                END,
                updated_at = datetime('now')
            WHERE id = ?
            "#,
        )
        .bind(input.email.trim())
        .bind(&input.role)
        .bind(input.display_name.trim())
        .bind(input.points_balance)
        .bind(if input.enabled { 1 } else { 0 })
        .bind(if input.enabled { 1 } else { 0 })
        .bind(id)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            return Err(AppError::NotFound);
        }
        if !input.enabled {
            self.disable_user_resources(id).await?;
        }
        self.get_managed_user(id).await
    }

    pub async fn set_user_enabled(
        &self,
        actor_id: i64,
        id: i64,
        enabled: bool,
    ) -> AppResult<ManagedUser> {
        let current = self.get_user(id).await?;
        if !enabled
            && current.role == "admin"
            && current.enabled
            && self.enabled_admin_count().await? <= 1
        {
            return Err(AppError::BadRequest(
                "cannot disable the last enabled admin".to_string(),
            ));
        }
        if actor_id == id && !enabled {
            return Err(AppError::BadRequest(
                "cannot disable your own account".to_string(),
            ));
        }
        let result = sqlx::query(
            r#"
            UPDATE users
            SET enabled = ?,
                disabled_at = CASE
                  WHEN ? = 1 THEN NULL
                  WHEN enabled = 0 THEN disabled_at
                  ELSE datetime('now')
                END,
                updated_at = datetime('now')
            WHERE id = ?
            "#,
        )
        .bind(if enabled { 1 } else { 0 })
        .bind(if enabled { 1 } else { 0 })
        .bind(id)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            return Err(AppError::NotFound);
        }
        if !enabled {
            self.disable_user_resources(id).await?;
        }
        self.get_managed_user(id).await
    }

    pub async fn reset_user_password(&self, id: i64, password: &str) -> AppResult<()> {
        validate_password(password)?;
        let password_hash = hash_password(password)?;
        let mut tx = self.pool.begin().await?;
        let result = sqlx::query(
            "UPDATE users SET password_hash = ?, updated_at = datetime('now') WHERE id = ?",
        )
        .bind(password_hash)
        .bind(id)
        .execute(&mut *tx)
        .await?;
        if result.rows_affected() == 0 {
            return Err(AppError::NotFound);
        }
        sqlx::query("DELETE FROM sessions WHERE user_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    async fn enabled_admin_count(&self) -> AppResult<i64> {
        Ok(
            sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE role = 'admin' AND enabled = 1")
                .fetch_one(&self.pool)
                .await?,
        )
    }

    async fn disable_user_resources(&self, user_id: i64) -> AppResult<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM sessions WHERE user_id = ?")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "UPDATE api_keys SET enabled = 0, updated_at = datetime('now') WHERE user_id = ? AND deleted_at IS NULL",
        )
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE channels SET enabled = 0, status = 'manual_disabled', updated_at = datetime('now') WHERE owner_user_id = ? AND deleted_at IS NULL",
        )
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn create_api_key(
        &self,
        user_id: i64,
        name: &str,
        spend_limit_points: Option<f64>,
    ) -> AppResult<(String, ApiKeyRecord)> {
        validate_api_key_name(name)?;
        validate_spend_limit(spend_limit_points)?;
        let token = generate_token("sk");
        let key_prefix = token.chars().take(12).collect::<String>();
        let mut tx = self.pool.begin().await?;
        let result = sqlx::query(
            "INSERT INTO api_keys(user_id, name, key_prefix, key_hash, spend_limit_points, allowed_models_json, updated_at) VALUES (?, ?, ?, ?, ?, '[]', datetime('now'))",
        )
        .bind(user_id)
        .bind(name.trim())
        .bind(&key_prefix)
        .bind(hash_token(&token))
        .bind(spend_limit_points)
        .execute(&mut *tx)
        .await?;
        let api_key_id = result.last_insert_rowid();
        sqlx::query(
            r#"
            INSERT OR IGNORE INTO api_key_channels(api_key_id, channel_id)
            SELECT ?, id
            FROM channels
            WHERE deleted_at IS NULL
            "#,
        )
        .bind(api_key_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        let record = self.get_api_key(api_key_id).await?;
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
        let mut record = api_key_from_row(&row);
        record.allowed_channel_ids = self.list_api_key_channel_ids(id).await?;
        Ok(record)
    }

    pub async fn list_api_keys(&self, user_id: i64) -> AppResult<Vec<ApiKeyRecord>> {
        let rows = sqlx::query(
            "SELECT id, user_id, name, key_prefix, enabled, spend_limit_points, spent_points, expires_at, allowed_models_json, last_used_at FROM api_keys WHERE user_id = ? AND deleted_at IS NULL ORDER BY id DESC",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        let mut records = rows.iter().map(api_key_from_row).collect::<Vec<_>>();
        self.attach_api_key_channel_ids(&mut records).await?;
        Ok(records)
    }

    async fn list_api_key_channel_ids(&self, api_key_id: i64) -> AppResult<Vec<i64>> {
        let rows = sqlx::query(
            r#"
            SELECT akc.channel_id
            FROM api_key_channels akc
            JOIN channels c ON c.id = akc.channel_id
            WHERE akc.api_key_id = ? AND c.deleted_at IS NULL
            ORDER BY c.id DESC
            "#,
        )
        .bind(api_key_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.iter().map(|row| row.get("channel_id")).collect())
    }

    async fn attach_api_key_channel_ids(&self, records: &mut [ApiKeyRecord]) -> AppResult<()> {
        let key_ids = records.iter().map(|record| record.id).collect::<Vec<_>>();
        if key_ids.is_empty() {
            return Ok(());
        }
        let placeholders = std::iter::repeat_n("?", key_ids.len())
            .collect::<Vec<_>>()
            .join(",");
        let query = format!(
            r#"
            SELECT akc.api_key_id, akc.channel_id
            FROM api_key_channels akc
            JOIN channels c ON c.id = akc.channel_id
            WHERE akc.api_key_id IN ({placeholders}) AND c.deleted_at IS NULL
            ORDER BY c.id DESC
            "#
        );
        let mut query = sqlx::query(&query);
        for key_id in &key_ids {
            query = query.bind(key_id);
        }
        let rows = query.fetch_all(&self.pool).await?;
        let mut ids_by_key = key_ids
            .iter()
            .map(|id| (*id, Vec::new()))
            .collect::<HashMap<_, _>>();
        for row in rows {
            ids_by_key
                .entry(row.get("api_key_id"))
                .or_default()
                .push(row.get("channel_id"));
        }
        for record in records {
            record.allowed_channel_ids = ids_by_key.remove(&record.id).unwrap_or_default();
        }
        Ok(())
    }

    async fn replace_api_key_channels(
        &self,
        user_id: i64,
        api_key_id: i64,
        channel_ids: &[i64],
    ) -> AppResult<()> {
        validate_channel_selection(channel_ids)?;
        let normalized = unique_positive_ids(channel_ids);
        if normalized.len() != channel_ids.len() {
            return Err(AppError::BadRequest(
                "allowed channel ids must be unique positive integers".to_string(),
            ));
        }
        let mut tx = self.pool.begin().await?;
        let key_exists = sqlx::query(
            "SELECT 1 FROM api_keys WHERE id = ? AND user_id = ? AND deleted_at IS NULL",
        )
        .bind(api_key_id)
        .bind(user_id)
        .fetch_optional(&mut *tx)
        .await?
        .is_some();
        if !key_exists {
            return Err(AppError::NotFound);
        }
        if !normalized.is_empty() {
            let placeholders = std::iter::repeat_n("?", normalized.len())
                .collect::<Vec<_>>()
                .join(",");
            let query = format!(
                "SELECT id FROM channels WHERE id IN ({placeholders}) AND deleted_at IS NULL"
            );
            let mut query = sqlx::query(&query);
            for id in &normalized {
                query = query.bind(id);
            }
            let rows = query.fetch_all(&mut *tx).await?;
            if rows.len() != normalized.len() {
                return Err(AppError::BadRequest(
                    "allowed channels must be visible active channels".to_string(),
                ));
            }
        }
        sqlx::query("DELETE FROM api_key_channels WHERE api_key_id = ?")
            .bind(api_key_id)
            .execute(&mut *tx)
            .await?;
        for channel_id in normalized {
            sqlx::query("INSERT INTO api_key_channels(api_key_id, channel_id) VALUES (?, ?)")
                .bind(api_key_id)
                .bind(channel_id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
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
        self.replace_api_key_channels(user_id, id, &input.allowed_channel_ids)
            .await?;
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
            WHERE key_hash = ? AND enabled = 1 AND deleted_at IS NULL
              AND (expires_at IS NULL OR expires_at > datetime('now'))
            "#,
        )
        .bind(key_hash)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(AppError::Unauthorized)?;
        let mut record = api_key_from_row(&row);
        record.allowed_channel_ids = self.list_api_key_channel_ids(record.id).await?;
        Ok(record)
    }

    pub async fn mark_api_key_used(&self, id: i64) -> AppResult<()> {
        sqlx::query("UPDATE api_keys SET last_used_at = datetime('now') WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn reserve_gateway_request(
        &self,
        user_id: i64,
        api_key_id: i64,
        channel_id: i64,
        tokens: i64,
        points: f64,
    ) -> AppResult<GatewayReservation> {
        if tokens <= 0 || points < 0.0 || !points.is_finite() {
            return Err(AppError::BadRequest(
                "invalid gateway reservation".to_string(),
            ));
        }
        let mut tx = self.pool.begin().await?;
        let user_debit = sqlx::query(
            "UPDATE users SET points_balance = points_balance - ? WHERE id = ? AND enabled = 1 AND points_balance >= ?",
        )
        .bind(points)
        .bind(user_id)
        .bind(points)
        .execute(&mut *tx)
        .await?;
        if user_debit.rows_affected() == 0 {
            return Err(AppError::BadRequest(
                "insufficient points for estimated input tokens".to_string(),
            ));
        }

        let key_debit = sqlx::query(
            r#"
            UPDATE api_keys
            SET spent_points = spent_points + ?, updated_at = datetime('now')
            WHERE id = ? AND user_id = ? AND enabled = 1 AND deleted_at IS NULL
              AND (expires_at IS NULL OR expires_at > datetime('now'))
              AND (spend_limit_points IS NULL OR spent_points + ? <= spend_limit_points)
            "#,
        )
        .bind(points)
        .bind(api_key_id)
        .bind(user_id)
        .bind(points)
        .execute(&mut *tx)
        .await?;
        if key_debit.rows_affected() == 0 {
            return Err(AppError::BadRequest(
                "api key spend limit would be exceeded".to_string(),
            ));
        }

        let quota_window_count: (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*)
            FROM channel_quota_windows
            WHERE channel_id = ?
              AND used_tokens + ? <= limit_tokens
            "#,
        )
        .bind(channel_id)
        .bind(tokens)
        .fetch_one(&mut *tx)
        .await?;
        let expected_window_count: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM channel_quota_windows WHERE channel_id = ?")
                .bind(channel_id)
                .fetch_one(&mut *tx)
                .await?;
        if expected_window_count.0 == 0 || quota_window_count.0 != expected_window_count.0 {
            return Err(AppError::BadRequest(
                "channel token quota no longer has enough room for the estimate".to_string(),
            ));
        }
        sqlx::query(
            r#"
            UPDATE channel_quota_windows
            SET used_tokens = used_tokens + ?, updated_at = datetime('now')
            WHERE channel_id = ?
            "#,
        )
        .bind(tokens)
        .bind(channel_id)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(GatewayReservation {
            user_id,
            api_key_id,
            channel_id,
            points,
            tokens,
        })
    }

    pub async fn release_gateway_reservation(
        &self,
        reservation: &GatewayReservation,
    ) -> AppResult<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("UPDATE users SET points_balance = points_balance + ? WHERE id = ?")
            .bind(reservation.points)
            .bind(reservation.user_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE api_keys SET spent_points = MAX(0, spent_points - ?) WHERE id = ?")
            .bind(reservation.points)
            .bind(reservation.api_key_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            r#"
            UPDATE channel_quota_windows
            SET used_tokens = MAX(0, used_tokens - ?), updated_at = datetime('now')
            WHERE channel_id = ?
            "#,
        )
        .bind(reservation.tokens)
        .bind(reservation.channel_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn list_route_channels(&self) -> AppResult<Vec<Channel>> {
        let rows = sqlx::query(
            r#"
            SELECT c.id, c.owner_user_id, c.name, c.provider, c.base_url, c.api_key_secret, c.models_json,
                   c.enabled, c.status, c.health_checked_at, c.upstream_latency_ms, c.last_error,
                   l.fire_sale_days_before, l.fire_sale_remaining_pct, l.fire_sale_discount, l.provider_share
            FROM channels c JOIN channel_limits l ON c.id = l.channel_id
            WHERE c.deleted_at IS NULL
            ORDER BY c.id DESC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;
        let windows = self.windows_by_channel().await?;
        rows.iter()
            .map(|row| channel_from_row(row, &windows))
            .collect()
    }

    pub async fn list_route_channels_for_api_key(
        &self,
        api_key_id: i64,
    ) -> AppResult<Vec<Channel>> {
        let rows = sqlx::query(
            r#"
            SELECT c.id, c.owner_user_id, c.name, c.provider, c.base_url, c.api_key_secret, c.models_json,
                   c.enabled, c.status, c.health_checked_at, c.upstream_latency_ms, c.last_error,
                   l.fire_sale_days_before, l.fire_sale_remaining_pct, l.fire_sale_discount, l.provider_share
            FROM channels c
            JOIN api_key_channels akc ON akc.channel_id = c.id
            JOIN channel_limits l ON c.id = l.channel_id
            WHERE akc.api_key_id = ? AND c.deleted_at IS NULL
            ORDER BY c.id DESC
            "#,
        )
        .bind(api_key_id)
        .fetch_all(&self.pool)
        .await?;
        let windows = self.windows_by_channel().await?;
        rows.iter()
            .map(|row| channel_from_row(row, &windows))
            .collect()
    }

    async fn windows_by_channel(&self) -> AppResult<HashMap<i64, Vec<ChannelQuotaWindow>>> {
        let rows = sqlx::query(
            r#"
            SELECT id, channel_id, name, limit_tokens, used_tokens, period_unit, period_count,
                   anchor_at, timezone, current_window_start_at, current_window_end_at, sort_order
            FROM channel_quota_windows
            ORDER BY channel_id, sort_order, id
            "#,
        )
        .fetch_all(&self.pool)
        .await?;
        let mut windows: HashMap<i64, Vec<ChannelQuotaWindow>> = HashMap::new();
        for row in rows {
            windows
                .entry(row.get("channel_id"))
                .or_default()
                .push(ChannelQuotaWindow {
                    id: row.get("id"),
                    name: row.get("name"),
                    limit_tokens: row.get("limit_tokens"),
                    used_tokens: row.get("used_tokens"),
                    period_unit: row.get("period_unit"),
                    period_count: row.get("period_count"),
                    anchor_at: row.get("anchor_at"),
                    timezone: row.get("timezone"),
                    current_window_start_at: row.get("current_window_start_at"),
                    current_window_end_at: row.get("current_window_end_at"),
                    sort_order: row.get("sort_order"),
                });
        }
        Ok(windows)
    }

    async fn attach_channel_health_windows(&self, channels: &mut [PublicChannel]) -> AppResult<()> {
        let now = Utc::now();
        let channel_ids = channels
            .iter()
            .map(|channel| channel.id)
            .collect::<Vec<_>>();
        let mut windows_by_channel = channel_ids
            .iter()
            .map(|id| (*id, empty_channel_health_windows(now)))
            .collect::<HashMap<_, _>>();
        if channel_ids.is_empty() {
            return Ok(());
        }

        let retention = format!(
            "-{} seconds",
            CHANNEL_HEALTH_WINDOW_SECONDS * CHANNEL_HEALTH_WINDOW_COUNT as i64
        );
        let rows = sqlx::query(
            r#"
            SELECT channel_id, status, ttft_ms, created_at
            FROM channel_health_events
            WHERE created_at >= datetime('now', ?)
            ORDER BY channel_id, created_at
            "#,
        )
        .bind(retention)
        .fetch_all(&self.pool)
        .await?;
        for row in rows {
            let channel_id = row.get::<i64, _>("channel_id");
            let Some(windows) = windows_by_channel.get_mut(&channel_id) else {
                continue;
            };
            let created_at = row.get::<String, _>("created_at");
            let Some(created_at) = parse_sqlite_utc_datetime_opt(&created_at) else {
                continue;
            };
            let Some(bucket) = windows
                .iter_mut()
                .find(|bucket| created_at >= bucket.start_at && created_at < bucket.end_at)
            else {
                continue;
            };
            bucket.record(row.get::<String, _>("status"), row.get("ttft_ms"));
        }
        for channel in channels {
            let windows = windows_by_channel
                .remove(&channel.id)
                .unwrap_or_else(|| empty_channel_health_windows(now));
            channel.health_windows = windows
                .into_iter()
                .map(ChannelHealthAccumulator::finish)
                .collect();
        }
        Ok(())
    }

    pub async fn list_public_channels(&self, user: &User) -> AppResult<Vec<PublicChannel>> {
        let rows = if user.role == "admin" {
            sqlx::query(
                r#"
                SELECT c.id, c.owner_user_id, c.name, c.provider, c.base_url, c.api_key_secret, c.models_json,
                       c.enabled, c.status, c.health_checked_at, c.upstream_latency_ms, c.last_error,
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
        let windows = self.windows_by_channel().await?;
        let mut channels = rows
            .iter()
            .map(|row| channel_from_row(row, &windows))
            .map(|result| result.map(PublicChannel::from))
            .collect::<AppResult<Vec<_>>>()?;
        self.attach_channel_health_windows(&mut channels).await?;
        Ok(channels)
    }

    pub async fn list_public_route_channels(&self) -> AppResult<Vec<PublicChannel>> {
        let rows = sqlx::query(
            r#"
            SELECT c.id, c.owner_user_id, c.name, c.provider, c.base_url, c.api_key_secret, c.models_json,
                   c.enabled, c.status, c.health_checked_at, c.upstream_latency_ms, c.last_error,
                   l.fire_sale_days_before, l.fire_sale_remaining_pct, l.fire_sale_discount, l.provider_share
            FROM channels c JOIN channel_limits l ON c.id = l.channel_id
            WHERE c.deleted_at IS NULL
            ORDER BY c.id DESC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;
        let windows = self.windows_by_channel().await?;
        let mut channels = rows
            .iter()
            .map(|row| channel_from_row(row, &windows))
            .map(|result| result.map(PublicChannel::from))
            .collect::<AppResult<Vec<_>>>()?;
        self.attach_channel_health_windows(&mut channels).await?;
        Ok(channels)
    }

    pub async fn upsert_channel(
        &self,
        owner_user_id: i64,
        input: ChannelInput,
    ) -> AppResult<Channel> {
        validate_channel_input(&input, true)?;
        let mut tx = self.pool.begin().await?;
        let previous_channel_count =
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM channels WHERE deleted_at IS NULL")
                .fetch_one(&mut *tx)
                .await?;
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
              channel_id, fire_sale_days_before, fire_sale_remaining_pct, fire_sale_discount, provider_share
            ) VALUES (?, ?, ?, ?, ?)
            "#,
        )
        .bind(channel_id)
        .bind(input.fire_sale_days_before)
        .bind(input.fire_sale_remaining_pct)
        .bind(input.fire_sale_discount)
        .bind(input.provider_share)
        .execute(&mut *tx)
        .await?;
        upsert_quota_windows(&mut tx, channel_id, &input.windows, false).await?;
        sqlx::query(
            r#"
            INSERT OR IGNORE INTO api_key_channels(api_key_id, channel_id)
            SELECT k.id, ?
            FROM api_keys k
            WHERE k.deleted_at IS NULL
              AND (
                SELECT COUNT(*)
                FROM api_key_channels akc
                JOIN channels c ON c.id = akc.channel_id
                WHERE akc.api_key_id = k.id AND c.deleted_at IS NULL
              ) = ?
            "#,
        )
        .bind(channel_id)
        .bind(previous_channel_count)
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
                   l.fire_sale_days_before, l.fire_sale_remaining_pct, l.fire_sale_discount, l.provider_share
            FROM channels c JOIN channel_limits l ON c.id = l.channel_id
            WHERE c.id = ? AND c.deleted_at IS NULL
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(AppError::NotFound)?;
        let windows = self.windows_by_channel().await?;
        channel_from_row(&row, &windows)
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
            SET fire_sale_days_before = ?, fire_sale_remaining_pct = ?, fire_sale_discount = ?,
                provider_share = ?, updated_at = datetime('now')
            WHERE channel_id = ?
            "#,
        )
        .bind(input.fire_sale_days_before)
        .bind(input.fire_sale_remaining_pct)
        .bind(input.fire_sale_discount)
        .bind(input.provider_share)
        .bind(id)
        .execute(&mut *tx)
        .await?;
        upsert_quota_windows(&mut tx, id, &input.windows, true).await?;
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
            windows: existing
                .limits
                .windows
                .iter()
                .map(|window| ChannelQuotaWindowInput {
                    name: window.name.clone(),
                    limit_tokens: window.limit_tokens,
                    period_unit: window.period_unit.clone(),
                    period_count: window.period_count,
                    anchor_at: window.anchor_at.clone(),
                    timezone: window.timezone.clone(),
                })
                .collect(),
            fire_sale_days_before: existing.limits.fire_sale_days_before,
            fire_sale_remaining_pct: existing.limits.fire_sale_remaining_pct,
            fire_sale_discount: existing.limits.fire_sale_discount,
            provider_share: existing.limits.provider_share,
        };
        let clone = self.upsert_channel(existing.owner_user_id, input).await?;
        if !reset_usage {
            for (index, window) in existing.limits.windows.iter().enumerate() {
                sqlx::query(
                    r#"
                    UPDATE channel_quota_windows
                    SET used_tokens = ?, current_window_start_at = ?, current_window_end_at = ?,
                        updated_at = datetime('now')
                    WHERE channel_id = ? AND sort_order = ?
                    "#,
                )
                .bind(window.used_tokens)
                .bind(&window.current_window_start_at)
                .bind(&window.current_window_end_at)
                .bind(clone.id)
                .bind(index as i64)
                .execute(&self.pool)
                .await?;
            }
        }
        self.get_channel(clone.id).await.map(PublicChannel::from)
    }

    pub async fn record_channel_health(
        &self,
        channel_id: i64,
        latency_ms: i64,
        last_error: Option<&str>,
    ) -> AppResult<()> {
        self.record_channel_health_event(ChannelHealthEventInput {
            channel_id,
            request_id: None,
            status: if last_error.is_some() {
                "degraded"
            } else {
                "available"
            },
            http_status: None,
            ttft_ms: if last_error.is_some() {
                None
            } else {
                Some(latency_ms)
            },
            total_latency_ms: Some(latency_ms),
            error: last_error,
        })
        .await
    }

    pub async fn record_channel_health_event(
        &self,
        event: ChannelHealthEventInput<'_>,
    ) -> AppResult<()> {
        validate_channel_health_status(event.status)?;
        let mut tx = self.pool.begin().await?;
        let result = sqlx::query(
            r#"
            UPDATE channels
            SET health_checked_at = datetime('now'), upstream_latency_ms = ?, last_error = ?,
                updated_at = datetime('now')
            WHERE id = ? AND deleted_at IS NULL
            "#,
        )
        .bind(event.ttft_ms.or(event.total_latency_ms))
        .bind(if event.status == "available" {
            None
        } else {
            event.error
        })
        .bind(event.channel_id)
        .execute(&mut *tx)
        .await?;
        if result.rows_affected() == 0 {
            return Err(AppError::NotFound);
        }
        sqlx::query(
            r#"
            INSERT INTO channel_health_events(
              channel_id, request_id, status, http_status, ttft_ms, total_latency_ms, error
            ) VALUES (?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(event.channel_id)
        .bind(event.request_id)
        .bind(event.status)
        .bind(event.http_status)
        .bind(event.ttft_ms)
        .bind(event.total_latency_ms)
        .bind(event.error)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            r#"
            DELETE FROM channel_health_events
            WHERE channel_id = ? AND created_at < datetime('now', ?)
            "#,
        )
        .bind(event.channel_id)
        .bind(format!("-{CHANNEL_HEALTH_RETENTION_DAYS} days"))
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
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
        let rows = sqlx::query(
            r#"
            SELECT id, period_unit, period_count, anchor_at, timezone,
                   current_window_start_at, current_window_end_at
            FROM channel_quota_windows
            "#,
        )
        .fetch_all(&self.pool)
        .await?;
        for row in rows {
            let window_id: i64 = row.get("id");
            let current_end = parse_utc_rfc3339(&row.get::<String, _>("current_window_end_at"))?;
            if now < current_end {
                continue;
            }
            let definition = QuotaWindowDefinition {
                period_unit: row.get("period_unit"),
                period_count: row.get("period_count"),
                anchor_at: row.get("anchor_at"),
                timezone: row.get("timezone"),
            };
            let (start_at, end_at) = compute_window_bounds(&definition, now)?;
            sqlx::query(
                r#"
                UPDATE channel_quota_windows
                SET used_tokens = 0, current_window_start_at = ?, current_window_end_at = ?,
                    updated_at = datetime('now')
                WHERE id = ?
                "#,
            )
            .bind(start_at.to_rfc3339())
            .bind(end_at.to_rfc3339())
            .bind(window_id)
            .execute(&self.pool)
            .await?;
        }
        sqlx::query(
            r#"
            UPDATE channels
            SET status = CASE
                WHEN deleted_at IS NOT NULL THEN status
                WHEN enabled = 0 THEN status
                WHEN EXISTS (
                    SELECT 1 FROM channel_quota_windows
                    WHERE channel_id = channels.id AND used_tokens >= limit_tokens
                ) THEN 'cooling'
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

    pub async fn apply_ledger_event(&self, event: &LedgerEvent) -> AppResult<bool> {
        let mut tx = self.pool.begin().await?;
        let inserted = sqlx::query(
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
        if inserted.rows_affected() == 0 {
            tx.commit().await?;
            return Ok(false);
        }

        let point_delta = event.total_points - event.reservation.points;
        sqlx::query("UPDATE users SET points_balance = points_balance - ? WHERE id = ?")
            .bind(point_delta)
            .bind(event.user_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE users SET points_balance = points_balance + ? WHERE id = ?")
            .bind(event.provider_points)
            .bind(event.provider_user_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE api_keys SET spent_points = MAX(0, spent_points + ?) WHERE id = ?")
            .bind(point_delta)
            .bind(event.api_key_id)
            .execute(&mut *tx)
            .await?;
        let token_delta = event.usage.total() - event.reservation.tokens;
        sqlx::query(
            r#"
            UPDATE channel_quota_windows
            SET used_tokens = MAX(0, used_tokens + ?), updated_at = datetime('now')
            WHERE channel_id = ?
            "#,
        )
        .bind(token_delta)
        .bind(event.channel_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(true)
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
            SELECT COALESCE(SUM(w.limit_tokens - w.used_tokens), 0)
            FROM channel_quota_windows w
            JOIN channels c ON c.id = w.channel_id
            WHERE c.deleted_at IS NULL
              AND w.sort_order = (
                  SELECT MIN(w2.sort_order)
                  FROM channel_quota_windows w2
                  WHERE w2.channel_id = w.channel_id
              )
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
        for setting in settings {
            validate_setting_value(&setting.key, &setting.value)?;
        }
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
        let from_exists: Option<i64> = sqlx::query_scalar("SELECT id FROM users WHERE id = ?")
            .bind(from_user_id)
            .fetch_optional(&mut *tx)
            .await?;
        if from_exists.is_none() {
            return Err(AppError::NotFound);
        }
        let exists: Option<i64> = sqlx::query_scalar("SELECT id FROM users WHERE id = ?")
            .bind(to_user_id)
            .fetch_optional(&mut *tx)
            .await?;
        if exists.is_none() {
            return Err(AppError::NotFound);
        }
        let debit = sqlx::query(
            "UPDATE users SET points_balance = points_balance - ? WHERE id = ? AND points_balance >= ?",
        )
            .bind(points)
            .bind(from_user_id)
            .bind(points)
            .execute(&mut *tx)
            .await?;
        if debit.rows_affected() == 0 {
            return Err(AppError::BadRequest("insufficient points".to_string()));
        }
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
    pub windows: Vec<ChannelQuotaWindowInput>,
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
    pub windows: Vec<ChannelQuotaWindowInput>,
    pub fire_sale_days_before: i64,
    pub fire_sale_remaining_pct: f64,
    pub fire_sale_discount: f64,
    pub provider_share: f64,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ChannelQuotaWindowInput {
    pub name: String,
    pub limit_tokens: i64,
    pub period_unit: String,
    pub period_count: i64,
    pub anchor_at: String,
    pub timezone: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ApiKeyUpdateInput {
    pub name: String,
    pub enabled: bool,
    pub spend_limit_points: Option<f64>,
    pub expires_at: Option<String>,
    pub allowed_models: Vec<String>,
    pub allowed_channel_ids: Vec<i64>,
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
        allowed_channel_ids: Vec::new(),
        last_used_at: row.get("last_used_at"),
    }
}

fn user_from_row(row: &sqlx::sqlite::SqliteRow) -> User {
    User {
        id: row.get("id"),
        email: row.get("email"),
        role: row.get("role"),
        display_name: row.get("display_name"),
        points_balance: row.get("points_balance"),
        anonymous_leaderboard: row.get::<i64, _>("anonymous_leaderboard") != 0,
        enabled: row.get::<i64, _>("enabled") != 0,
    }
}

fn managed_user_from_row(row: &sqlx::sqlite::SqliteRow) -> ManagedUser {
    ManagedUser {
        id: row.get("id"),
        email: row.get("email"),
        role: row.get("role"),
        display_name: row.get("display_name"),
        points_balance: row.get("points_balance"),
        anonymous_leaderboard: row.get::<i64, _>("anonymous_leaderboard") != 0,
        enabled: row.get::<i64, _>("enabled") != 0,
        disabled_at: row.get("disabled_at"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        api_key_count: row.get("api_key_count"),
        channel_count: row.get("channel_count"),
        total_spent_points: row.get("total_spent_points"),
        total_provider_points: row.get("total_provider_points"),
    }
}

fn channel_from_row(
    row: &sqlx::sqlite::SqliteRow,
    windows_by_channel: &HashMap<i64, Vec<ChannelQuotaWindow>>,
) -> AppResult<Channel> {
    let channel_id = row.get("id");
    Ok(Channel {
        id: channel_id,
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
            windows: windows_by_channel
                .get(&channel_id)
                .cloned()
                .unwrap_or_default(),
            fire_sale_days_before: row.get("fire_sale_days_before"),
            fire_sale_remaining_pct: row.get("fire_sale_remaining_pct"),
            fire_sale_discount: row.get("fire_sale_discount"),
            provider_share: row.get("provider_share"),
        },
    })
}

struct ChannelHealthAccumulator {
    start_at: DateTime<Utc>,
    end_at: DateTime<Utc>,
    sample_count: i64,
    success_count: i64,
    empty_count: i64,
    degraded_count: i64,
    down_count: i64,
    ttft_sum_ms: i64,
    ttft_count: i64,
}

impl ChannelHealthAccumulator {
    fn record(&mut self, status: String, ttft_ms: Option<i64>) {
        self.sample_count += 1;
        match status.as_str() {
            "available" => {
                self.success_count += 1;
                if let Some(ttft_ms) = ttft_ms {
                    self.ttft_sum_ms += ttft_ms;
                    self.ttft_count += 1;
                }
            }
            "empty" => self.empty_count += 1,
            "degraded" => self.degraded_count += 1,
            "down" => self.down_count += 1,
            _ => self.degraded_count += 1,
        }
    }

    fn finish(self) -> ChannelHealthWindow {
        let status = if self.sample_count == 0 {
            "unknown"
        } else if self.down_count > 0 {
            "down"
        } else if self.empty_count > 0 {
            "empty"
        } else if self.degraded_count > 0 {
            "degraded"
        } else {
            "available"
        };
        ChannelHealthWindow {
            window_start_at: self.start_at.to_rfc3339(),
            window_end_at: self.end_at.to_rfc3339(),
            status: status.to_string(),
            sample_count: self.sample_count,
            success_count: self.success_count,
            empty_count: self.empty_count,
            degraded_count: self.degraded_count,
            down_count: self.down_count,
            avg_ttft_ms: if self.ttft_count > 0 {
                Some((self.ttft_sum_ms as f64 / self.ttft_count as f64).round() as i64)
            } else {
                None
            },
        }
    }
}

fn empty_channel_health_windows(now: DateTime<Utc>) -> Vec<ChannelHealthAccumulator> {
    let end_epoch =
        ((now.timestamp() / CHANNEL_HEALTH_WINDOW_SECONDS) + 1) * CHANNEL_HEALTH_WINDOW_SECONDS;
    let first_start =
        end_epoch - CHANNEL_HEALTH_WINDOW_SECONDS * CHANNEL_HEALTH_WINDOW_COUNT as i64;
    (0..CHANNEL_HEALTH_WINDOW_COUNT)
        .map(|index| {
            let start_epoch = first_start + index as i64 * CHANNEL_HEALTH_WINDOW_SECONDS;
            let end_epoch = start_epoch + CHANNEL_HEALTH_WINDOW_SECONDS;
            ChannelHealthAccumulator {
                start_at: Utc
                    .timestamp_opt(start_epoch, 0)
                    .single()
                    .expect("health window start timestamp is valid"),
                end_at: Utc
                    .timestamp_opt(end_epoch, 0)
                    .single()
                    .expect("health window end timestamp is valid"),
                sample_count: 0,
                success_count: 0,
                empty_count: 0,
                degraded_count: 0,
                down_count: 0,
                ttft_sum_ms: 0,
                ttft_count: 0,
            }
        })
        .collect()
}

fn parse_sqlite_utc_datetime_opt(value: &str) -> Option<DateTime<Utc>> {
    NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S")
        .ok()
        .map(|naive| Utc.from_utc_datetime(&naive))
        .or_else(|| {
            DateTime::parse_from_rfc3339(value)
                .ok()
                .map(|datetime| datetime.with_timezone(&Utc))
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

fn validate_email(email: &str) -> AppResult<()> {
    let email = email.trim();
    if email.len() < 3 || email.len() > 254 || !email.contains('@') {
        return Err(AppError::BadRequest(
            "email must be a valid address".to_string(),
        ));
    }
    Ok(())
}

fn validate_password(password: &str) -> AppResult<()> {
    if password.len() < 8 {
        return Err(AppError::BadRequest(
            "password must be at least 8 characters".to_string(),
        ));
    }
    Ok(())
}

fn validate_role(role: &str) -> AppResult<()> {
    if matches!(role, "admin" | "user") {
        Ok(())
    } else {
        Err(AppError::BadRequest(
            "role must be admin or user".to_string(),
        ))
    }
}

fn validate_display_name(display_name: &str) -> AppResult<()> {
    if display_name.trim().is_empty() || display_name.chars().count() > 80 {
        return Err(AppError::BadRequest(
            "display name must be 1-80 characters".to_string(),
        ));
    }
    Ok(())
}

fn validate_points_balance(points: f64) -> AppResult<()> {
    if !points.is_finite() || points < 0.0 {
        return Err(AppError::BadRequest(
            "points balance must be a non-negative finite number".to_string(),
        ));
    }
    Ok(())
}

fn default_display_name(email: &str) -> String {
    email.split('@').next().unwrap_or("user").trim().to_string()
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

fn validate_channel_selection(ids: &[i64]) -> AppResult<()> {
    if ids.len() > 500 {
        return Err(AppError::BadRequest(
            "an api key can select at most 500 channels".to_string(),
        ));
    }
    if ids.iter().any(|id| *id <= 0) {
        return Err(AppError::BadRequest(
            "allowed channel ids must be positive integers".to_string(),
        ));
    }
    Ok(())
}

fn unique_positive_ids(ids: &[i64]) -> Vec<i64> {
    let mut seen = HashSet::new();
    let mut normalized = Vec::new();
    for id in ids {
        if *id > 0 && seen.insert(*id) {
            normalized.push(*id);
        }
    }
    normalized
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
        &input.windows,
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
        &input.windows,
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
    windows: &[ChannelQuotaWindowInput],
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
    validate_quota_windows(windows)?;
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

fn validate_quota_windows(windows: &[ChannelQuotaWindowInput]) -> AppResult<()> {
    if windows.is_empty() {
        return Err(AppError::BadRequest(
            "channel must define at least one quota window".to_string(),
        ));
    }
    for window in windows {
        let name = window.name.trim();
        if name.is_empty() || name.chars().count() > 80 {
            return Err(AppError::BadRequest(
                "quota window name must be 1-80 characters".to_string(),
            ));
        }
        if window.limit_tokens <= 0 || window.period_count <= 0 {
            return Err(AppError::BadRequest(
                "quota window limit and period count must be positive".to_string(),
            ));
        }
        let definition = QuotaWindowDefinition {
            period_unit: window.period_unit.clone(),
            period_count: window.period_count,
            anchor_at: window.anchor_at.clone(),
            timezone: window.timezone.clone(),
        };
        let _ = compute_window_bounds(&definition, Utc::now())?;
    }
    Ok(())
}

fn validate_channel_health_status(status: &str) -> AppResult<()> {
    match status {
        "available" | "empty" | "degraded" | "down" => Ok(()),
        other => Err(AppError::BadRequest(format!(
            "unsupported channel health status: {other}"
        ))),
    }
}

struct QuotaWindowDefinition {
    period_unit: String,
    period_count: i64,
    anchor_at: String,
    timezone: String,
}

async fn upsert_quota_windows(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    channel_id: i64,
    windows: &[ChannelQuotaWindowInput],
    replace_existing: bool,
) -> AppResult<()> {
    if replace_existing {
        sqlx::query("DELETE FROM channel_quota_windows WHERE channel_id = ?")
            .bind(channel_id)
            .execute(&mut **tx)
            .await?;
    }
    let now = Utc::now();
    for (index, window) in windows.iter().enumerate() {
        let definition = QuotaWindowDefinition {
            period_unit: window.period_unit.trim().to_ascii_lowercase(),
            period_count: window.period_count,
            anchor_at: window.anchor_at.trim().to_string(),
            timezone: window.timezone.trim().to_string(),
        };
        let (start_at, end_at) = compute_window_bounds(&definition, now)?;
        sqlx::query(
            r#"
            INSERT INTO channel_quota_windows(
              channel_id, name, limit_tokens, used_tokens, period_unit, period_count,
              anchor_at, timezone, current_window_start_at, current_window_end_at, sort_order
            ) VALUES (?, ?, ?, 0, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(channel_id)
        .bind(window.name.trim())
        .bind(window.limit_tokens)
        .bind(&definition.period_unit)
        .bind(definition.period_count)
        .bind(&definition.anchor_at)
        .bind(&definition.timezone)
        .bind(start_at.to_rfc3339())
        .bind(end_at.to_rfc3339())
        .bind(index as i64)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

fn compute_window_bounds(
    definition: &QuotaWindowDefinition,
    now: DateTime<Utc>,
) -> AppResult<(DateTime<Utc>, DateTime<Utc>)> {
    if definition.period_count <= 0 {
        return Err(AppError::BadRequest(
            "quota window period count must be positive".to_string(),
        ));
    }
    let tz = parse_timezone(&definition.timezone)?;
    let anchor = parse_local_anchor(&definition.anchor_at, tz)?;
    let local_now = now.with_timezone(&tz);
    let start = match definition.period_unit.as_str() {
        "minute" => fixed_window_start(
            anchor,
            local_now,
            chrono::Duration::minutes(definition.period_count),
        ),
        "hour" => fixed_window_start(
            anchor,
            local_now,
            chrono::Duration::hours(definition.period_count),
        ),
        "day" => fixed_window_start(
            anchor,
            local_now,
            chrono::Duration::days(definition.period_count),
        ),
        "week" => fixed_window_start(
            anchor,
            local_now,
            chrono::Duration::weeks(definition.period_count),
        ),
        "month" => month_window_start(anchor, local_now, definition.period_count)?,
        "year" => month_window_start(anchor, local_now, definition.period_count * 12)?,
        other => {
            return Err(AppError::BadRequest(format!(
                "unsupported quota window period unit: {other}"
            )));
        }
    };
    let end = advance_window(start, &definition.period_unit, definition.period_count)?;
    Ok((start.with_timezone(&Utc), end.with_timezone(&Utc)))
}

fn parse_timezone(timezone: &str) -> AppResult<Tz> {
    timezone
        .parse()
        .map_err(|_| AppError::BadRequest(format!("invalid quota window timezone: {timezone}")))
}

fn parse_local_anchor(anchor_at: &str, timezone: Tz) -> AppResult<DateTime<Tz>> {
    let normalized = anchor_at.trim();
    if let Ok(utc_anchor) = DateTime::parse_from_rfc3339(normalized) {
        return Ok(utc_anchor.with_timezone(&timezone));
    }
    let naive = NaiveDateTime::parse_from_str(normalized, "%Y-%m-%dT%H:%M:%S")
        .or_else(|_| NaiveDateTime::parse_from_str(normalized, "%Y-%m-%d %H:%M:%S"))
        .or_else(|_| {
            NaiveDate::parse_from_str(normalized, "%Y-%m-%d")
                .map(|date| date.and_hms_opt(0, 0, 0).expect("midnight is valid"))
        })
        .map_err(|_| {
            AppError::BadRequest(
                "quota window anchor_at must be an RFC3339 or local YYYY-MM-DDTHH:MM:SS timestamp"
                    .to_string(),
            )
        })?;
    resolve_local_datetime(timezone, naive)
}

fn resolve_local_datetime(timezone: Tz, naive: NaiveDateTime) -> AppResult<DateTime<Tz>> {
    timezone
        .from_local_datetime(&naive)
        .earliest()
        .ok_or_else(|| {
            AppError::BadRequest(
                "quota window anchor falls into a nonexistent local time".to_string(),
            )
        })
}

fn fixed_window_start(
    anchor: DateTime<Tz>,
    now: DateTime<Tz>,
    duration: chrono::Duration,
) -> DateTime<Tz> {
    if now < anchor {
        return anchor;
    }
    let elapsed = now.signed_duration_since(anchor);
    let duration_seconds = duration.num_seconds().max(1);
    let periods = elapsed.num_seconds().div_euclid(duration_seconds);
    anchor + chrono::Duration::seconds(periods * duration_seconds)
}

fn month_window_start(
    anchor: DateTime<Tz>,
    now: DateTime<Tz>,
    period_months: i64,
) -> AppResult<DateTime<Tz>> {
    if period_months <= 0 {
        return Err(AppError::BadRequest(
            "quota window period count must be positive".to_string(),
        ));
    }
    if now < anchor {
        return Ok(anchor);
    }
    let elapsed_months =
        (now.year() - anchor.year()) as i64 * 12 + now.month() as i64 - anchor.month() as i64;
    let mut periods = elapsed_months.div_euclid(period_months).max(0);
    let mut start = add_months_clamped(anchor, periods * period_months)?;
    while advance_window(start, "month", period_months)? <= now {
        periods += 1;
        start = add_months_clamped(anchor, periods * period_months)?;
    }
    while start > now && periods > 0 {
        periods -= 1;
        start = add_months_clamped(anchor, periods * period_months)?;
    }
    Ok(start)
}

fn advance_window(
    start: DateTime<Tz>,
    period_unit: &str,
    period_count: i64,
) -> AppResult<DateTime<Tz>> {
    match period_unit {
        "minute" => Ok(start + chrono::Duration::minutes(period_count)),
        "hour" => Ok(start + chrono::Duration::hours(period_count)),
        "day" => Ok(start + chrono::Duration::days(period_count)),
        "week" => Ok(start + chrono::Duration::weeks(period_count)),
        "month" => add_months_clamped(start, period_count),
        "year" => add_months_clamped(start, period_count * 12),
        other => Err(AppError::BadRequest(format!(
            "unsupported quota window period unit: {other}"
        ))),
    }
}

fn add_months_clamped(start: DateTime<Tz>, months: i64) -> AppResult<DateTime<Tz>> {
    let month0 = start.month0() as i64 + months;
    let year = start.year() + month0.div_euclid(12) as i32;
    let month = month0.rem_euclid(12) as u32 + 1;
    let max_day = days_in_month(year, month);
    let day = start.day().min(max_day);
    let naive = NaiveDate::from_ymd_opt(year, month, day)
        .and_then(|date| {
            date.and_hms_nano_opt(
                start.hour(),
                start.minute(),
                start.second(),
                start.nanosecond(),
            )
        })
        .ok_or_else(|| AppError::BadRequest("invalid quota window boundary".to_string()))?;
    resolve_local_datetime(start.timezone(), naive)
}

fn days_in_month(year: i32, month: u32) -> u32 {
    let (next_year, next_month) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    let first_next = NaiveDate::from_ymd_opt(next_year, next_month, 1).expect("valid month");
    first_next.pred_opt().expect("previous day").day()
}

fn parse_utc_rfc3339(value: &str) -> AppResult<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|_| AppError::BadRequest("invalid stored quota window boundary".to_string()))
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
