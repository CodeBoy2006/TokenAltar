use std::{str::FromStr, time::Duration};

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::json;
use sqlx::{
    Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};

use crate::{
    auth::{generate_token, hash_password, hash_token},
    error::{AppError, AppResult},
    models::{
        AffinityRule, ApiKeyRecord, Channel, ChannelLimits, LedgerEvent, ModelPrice, User,
        json_array_to_strings,
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

    pub async fn create_user(&self, email: &str, password: &str, display_name: &str) -> AppResult<User> {
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
            "INSERT INTO api_keys(user_id, name, key_prefix, key_hash, spend_limit_points) VALUES (?, ?, ?, ?, ?)",
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
            "SELECT id, user_id, name, key_prefix, enabled, spend_limit_points, spent_points FROM api_keys WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(AppError::NotFound)?;
        Ok(api_key_from_row(&row))
    }

    pub async fn list_api_keys(&self, user_id: i64) -> AppResult<Vec<ApiKeyRecord>> {
        let rows = sqlx::query(
            "SELECT id, user_id, name, key_prefix, enabled, spend_limit_points, spent_points FROM api_keys WHERE user_id = ? ORDER BY id DESC",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.iter().map(api_key_from_row).collect())
    }

    pub async fn set_api_key_enabled(&self, user_id: i64, id: i64, enabled: bool) -> AppResult<()> {
        let result = sqlx::query("UPDATE api_keys SET enabled = ? WHERE id = ? AND user_id = ?")
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

    pub async fn find_api_key(&self, key_hash: &str) -> AppResult<ApiKeyRecord> {
        let row = sqlx::query(
            "SELECT id, user_id, name, key_prefix, enabled, spend_limit_points, spent_points FROM api_keys WHERE key_hash = ?",
        )
        .bind(key_hash)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(AppError::Unauthorized)?;
        Ok(api_key_from_row(&row))
    }

    pub async fn list_channels(&self) -> AppResult<Vec<Channel>> {
        let rows = sqlx::query(
            r#"
            SELECT c.id, c.owner_user_id, c.name, c.provider, c.base_url, c.api_key_secret, c.models_json,
                   c.enabled, c.status,
                   l.cycle_limit_tokens, l.cycle_reset_day, l.daily_limit_tokens, l.hourly_limit_tokens,
                   l.used_cycle_tokens, l.used_day_tokens, l.used_hour_tokens,
                   l.fire_sale_days_before, l.fire_sale_remaining_pct, l.fire_sale_discount, l.provider_share
            FROM channels c JOIN channel_limits l ON c.id = l.channel_id
            ORDER BY c.id DESC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(channel_from_row).collect()
    }

    pub async fn upsert_channel(&self, owner_user_id: i64, input: ChannelInput) -> AppResult<Channel> {
        let mut tx = self.pool.begin().await?;
        let models_json =
            serde_json::to_string(&input.models).map_err(|err| AppError::Anyhow(anyhow::anyhow!(err)))?;
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
                   c.enabled, c.status,
                   l.cycle_limit_tokens, l.cycle_reset_day, l.daily_limit_tokens, l.hourly_limit_tokens,
                   l.used_cycle_tokens, l.used_day_tokens, l.used_hour_tokens,
                   l.fire_sale_days_before, l.fire_sale_remaining_pct, l.fire_sale_discount, l.provider_share
            FROM channels c JOIN channel_limits l ON c.id = l.channel_id
            WHERE c.id = ?
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(AppError::NotFound)?;
        channel_from_row(&row)
    }

    pub async fn list_prices(&self) -> AppResult<Vec<ModelPrice>> {
        let rows = sqlx::query(
            "SELECT model_pattern, input_price_per_1k, output_price_per_1k, cache_price_per_1k FROM model_prices ORDER BY id",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .iter()
            .map(|row| ModelPrice {
                model_pattern: row.get("model_pattern"),
                input_price_per_1k: row.get("input_price_per_1k"),
                output_price_per_1k: row.get("output_price_per_1k"),
                cache_price_per_1k: row.get("cache_price_per_1k"),
            })
            .collect())
    }

    pub async fn upsert_price(&self, price: &ModelPrice) -> AppResult<()> {
        sqlx::query(
            r#"
            INSERT INTO model_prices(model_pattern, input_price_per_1k, output_price_per_1k, cache_price_per_1k)
            VALUES (?, ?, ?, ?)
            ON CONFLICT(model_pattern) DO UPDATE SET
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
        rules.into_iter().find(|rule| rule.id == id).ok_or(AppError::NotFound)
    }

    pub async fn get_affinity_binding(&self, cache_key: &str) -> AppResult<Option<i64>> {
        let row = sqlx::query(
            "SELECT channel_id FROM affinity_bindings WHERE cache_key = ? AND expires_at > datetime('now')",
        )
        .bind(cache_key)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|row| row.get("channel_id")))
    }

    pub async fn set_affinity_binding(
        &self,
        cache_key: &str,
        rule_id: i64,
        channel_id: i64,
        ttl_seconds: i64,
    ) -> AppResult<()> {
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
        .bind(expires_at)
        .execute(&self.pool)
        .await?;
        Ok(())
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
            sqlx::query(
                "SELECT * FROM ledger_entries WHERE user_id = ? ORDER BY id DESC LIMIT 200",
            )
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

    pub async fn dashboard(&self, surge_multiplier: f64, surge_state: &str) -> AppResult<DashboardSummary> {
        let users: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
            .fetch_one(&self.pool)
            .await?;
        let channels: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM channels")
            .fetch_one(&self.pool)
            .await?;
        let enabled_channels: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM channels WHERE enabled = 1")
                .fetch_one(&self.pool)
                .await?;
        let available_tokens: (i64,) = sqlx::query_as(
            "SELECT COALESCE(SUM(cycle_limit_tokens - used_cycle_tokens), 0) FROM channel_limits",
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

fn api_key_from_row(row: &sqlx::sqlite::SqliteRow) -> ApiKeyRecord {
    ApiKeyRecord {
        id: row.get("id"),
        user_id: row.get("user_id"),
        name: row.get("name"),
        key_prefix: row.get("key_prefix"),
        enabled: row.get::<i64, _>("enabled") != 0,
        spend_limit_points: row.get("spend_limit_points"),
        spent_points: row.get("spent_points"),
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

pub fn now_rfc3339() -> String {
    DateTime::<Utc>::from(std::time::SystemTime::now()).to_rfc3339()
}
