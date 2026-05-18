use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: i64,
    pub email: String,
    pub role: String,
    pub display_name: String,
    pub points_balance: f64,
    pub anonymous_leaderboard: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyRecord {
    pub id: i64,
    pub user_id: i64,
    pub name: String,
    pub key_prefix: String,
    pub enabled: bool,
    pub spend_limit_points: Option<f64>,
    pub spent_points: f64,
    pub expires_at: Option<String>,
    pub allowed_models: Vec<String>,
    pub last_used_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    OpenAi,
    Anthropic,
    Gemini,
}

impl ProviderKind {
    pub fn as_db(&self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
            Self::Gemini => "gemini",
        }
    }
}

impl TryFrom<&str> for ProviderKind {
    type Error = anyhow::Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "openai" => Ok(Self::OpenAi),
            "anthropic" => Ok(Self::Anthropic),
            "gemini" => Ok(Self::Gemini),
            _ => anyhow::bail!("unsupported provider: {value}"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Channel {
    pub id: i64,
    pub owner_user_id: i64,
    pub name: String,
    pub provider: ProviderKind,
    pub base_url: String,
    pub api_key_secret: String,
    pub models: Vec<String>,
    pub enabled: bool,
    pub status: String,
    pub health_checked_at: Option<String>,
    pub upstream_latency_ms: Option<i64>,
    pub last_error: Option<String>,
    pub limits: ChannelLimits,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublicChannel {
    pub id: i64,
    pub owner_user_id: i64,
    pub name: String,
    pub provider: ProviderKind,
    pub base_url: String,
    pub models: Vec<String>,
    pub enabled: bool,
    pub status: String,
    pub health_checked_at: Option<String>,
    pub upstream_latency_ms: Option<i64>,
    pub last_error: Option<String>,
    pub limits: ChannelLimits,
}

impl From<Channel> for PublicChannel {
    fn from(channel: Channel) -> Self {
        Self {
            id: channel.id,
            owner_user_id: channel.owner_user_id,
            name: channel.name,
            provider: channel.provider,
            base_url: channel.base_url,
            models: channel.models,
            enabled: channel.enabled,
            status: channel.status,
            health_checked_at: channel.health_checked_at,
            upstream_latency_ms: channel.upstream_latency_ms,
            last_error: channel.last_error,
            limits: channel.limits,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelLimits {
    pub cycle_limit_tokens: i64,
    pub cycle_reset_day: i64,
    pub daily_limit_tokens: i64,
    pub hourly_limit_tokens: i64,
    pub used_cycle_tokens: i64,
    pub used_day_tokens: i64,
    pub used_hour_tokens: i64,
    pub fire_sale_days_before: i64,
    pub fire_sale_remaining_pct: f64,
    pub fire_sale_discount: f64,
    pub provider_share: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelPrice {
    pub channel_id: Option<i64>,
    pub model_pattern: String,
    pub input_price_per_1k: f64,
    pub output_price_per_1k: f64,
    pub cache_price_per_1k: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_tokens: i64,
}

impl Usage {
    pub fn total(&self) -> i64 {
        self.input_tokens + self.output_tokens + self.cache_tokens
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AffinityRule {
    pub id: i64,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LedgerEvent {
    pub request_id: String,
    pub user_id: i64,
    pub api_key_id: i64,
    pub channel_id: i64,
    pub provider_user_id: i64,
    pub model: String,
    pub tokenizer: String,
    pub usage: Usage,
    pub price: ModelPrice,
    pub surge_multiplier: f64,
    pub fire_sale_discount: f64,
    pub total_points: f64,
    pub provider_points: f64,
    pub status: String,
    pub formula_note: String,
    pub reservation: GatewayReservation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayReservation {
    pub user_id: i64,
    pub api_key_id: i64,
    pub channel_id: i64,
    pub points: f64,
    pub tokens: i64,
}

#[derive(Debug, Clone)]
pub struct AuthContext {
    pub user: User,
    pub api_key: Option<ApiKeyRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayContext {
    pub user_group: String,
}

impl Default for GatewayContext {
    fn default() -> Self {
        Self {
            user_group: "default".to_string(),
        }
    }
}

pub fn json_array_to_strings(value: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(value).unwrap_or_default()
}

pub fn value_to_key_fragment(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        other => Some(other.to_string()),
    }
}
