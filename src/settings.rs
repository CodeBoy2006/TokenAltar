use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::{
    error::{AppError, AppResult},
    models::ModelPrice,
};

pub const DEFAULT_CHANNEL_WINDOWS_JSON: &str = r#"[{"name":"Monthly","limit_points":5,"period_unit":"month","period_count":1,"timezone":"UTC"},{"name":"Daily","limit_points":1,"period_unit":"day","period_count":1,"timezone":"UTC"},{"name":"Hourly","limit_points":0.25,"period_unit":"hour","period_count":1,"timezone":"UTC"}]"#;

pub const SETTING_DEFAULTS: &[(&str, &str)] = &[
    ("invite_required", "false"),
    ("invite_code_default", "TOKENALTAR"),
    ("initial_admin_points", "1000000"),
    ("initial_user_points", "1000"),
    ("pricing_unit_tokens", "1000000"),
    ("settlement_round_digits", "4"),
    ("fallback_input_price_per_unit", "5.0"),
    ("fallback_output_price_per_unit", "30.0"),
    ("fallback_cache_price_per_unit", "0.5"),
    ("surge_low_threshold", "0.30"),
    ("surge_high_threshold", "0.80"),
    ("surge_idle_multiplier", "0.5"),
    ("surge_normal_multiplier", "1.0"),
    ("surge_peak_multiplier", "1.5"),
    ("routing_max_attempts", "8"),
    ("routing_retry_cooldown_seconds", "30"),
    ("routing_fire_sale_weight_multiplier", "5.0"),
    ("ledger_queue_capacity", "4096"),
    ("affinity_cache_capacity", "4096"),
    ("default_api_key_spend_limit_points", "1000"),
    ("default_channel_name", "OpenAI Pool"),
    ("default_channel_provider", "openai"),
    ("default_channel_base_url", "https://api.openai.com"),
    ("default_channel_models", "gpt-*,gpt-4o*"),
    ("default_channel_windows_json", DEFAULT_CHANNEL_WINDOWS_JSON),
    ("default_channel_fire_sale_days_before", "3"),
    ("default_channel_fire_sale_remaining_pct", "0.25"),
    ("default_channel_fire_sale_discount", "0.2"),
    ("default_channel_provider_share", "0.7"),
];

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeSettings {
    pub invite_required: bool,
    pub invite_code_default: String,
    pub initial_admin_points: f64,
    pub initial_user_points: f64,
    pub pricing_unit_tokens: f64,
    pub settlement_round_digits: u32,
    pub fallback_input_price_per_unit: f64,
    pub fallback_output_price_per_unit: f64,
    pub fallback_cache_price_per_unit: f64,
    pub surge_low_threshold: f64,
    pub surge_high_threshold: f64,
    pub surge_idle_multiplier: f64,
    pub surge_normal_multiplier: f64,
    pub surge_peak_multiplier: f64,
    pub routing_max_attempts: usize,
    pub routing_retry_cooldown_seconds: u64,
    pub routing_fire_sale_weight_multiplier: f64,
    pub ledger_queue_capacity: usize,
    pub affinity_cache_capacity: usize,
    pub default_api_key_spend_limit_points: f64,
    pub default_channel_name: String,
    pub default_channel_provider: String,
    pub default_channel_base_url: String,
    pub default_channel_models: String,
    pub default_channel_windows: Vec<DefaultChannelWindow>,
    pub default_channel_windows_json: String,
    pub default_channel_fire_sale_days_before: i64,
    pub default_channel_fire_sale_remaining_pct: f64,
    pub default_channel_fire_sale_discount: f64,
    pub default_channel_provider_share: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DefaultChannelWindow {
    pub name: String,
    pub limit_points: f64,
    pub period_unit: String,
    pub period_count: i64,
    pub timezone: String,
}

impl RuntimeSettings {
    pub fn from_map(values: &HashMap<String, String>) -> AppResult<Self> {
        let default_channel_windows_json = get_string(values, "default_channel_windows_json");
        let default_channel_windows = parse_default_channel_windows(&default_channel_windows_json)?;
        let settings = Self {
            invite_required: parse_bool_setting(
                "invite_required",
                &get_string(values, "invite_required"),
            )?,
            invite_code_default: get_string(values, "invite_code_default"),
            initial_admin_points: get_non_negative_f64(values, "initial_admin_points")?,
            initial_user_points: get_non_negative_f64(values, "initial_user_points")?,
            pricing_unit_tokens: get_positive_f64(values, "pricing_unit_tokens")?,
            settlement_round_digits: get_u32_in_range(values, "settlement_round_digits", 0, 8)?,
            fallback_input_price_per_unit: get_non_negative_f64(
                values,
                "fallback_input_price_per_unit",
            )?,
            fallback_output_price_per_unit: get_non_negative_f64(
                values,
                "fallback_output_price_per_unit",
            )?,
            fallback_cache_price_per_unit: get_non_negative_f64(
                values,
                "fallback_cache_price_per_unit",
            )?,
            surge_low_threshold: get_ratio(values, "surge_low_threshold")?,
            surge_high_threshold: get_ratio(values, "surge_high_threshold")?,
            surge_idle_multiplier: get_non_negative_f64(values, "surge_idle_multiplier")?,
            surge_normal_multiplier: get_non_negative_f64(values, "surge_normal_multiplier")?,
            surge_peak_multiplier: get_non_negative_f64(values, "surge_peak_multiplier")?,
            routing_max_attempts: get_usize_in_range(values, "routing_max_attempts", 1, 100)?,
            routing_retry_cooldown_seconds: get_u64_in_range(
                values,
                "routing_retry_cooldown_seconds",
                0,
                86400,
            )?,
            routing_fire_sale_weight_multiplier: get_non_negative_f64(
                values,
                "routing_fire_sale_weight_multiplier",
            )?,
            ledger_queue_capacity: get_usize_in_range(
                values,
                "ledger_queue_capacity",
                1,
                1_000_000,
            )?,
            affinity_cache_capacity: get_usize_in_range(
                values,
                "affinity_cache_capacity",
                1,
                1_000_000,
            )?,
            default_api_key_spend_limit_points: get_non_negative_f64(
                values,
                "default_api_key_spend_limit_points",
            )?,
            default_channel_name: get_string(values, "default_channel_name"),
            default_channel_provider: get_string(values, "default_channel_provider"),
            default_channel_base_url: get_string(values, "default_channel_base_url"),
            default_channel_models: get_string(values, "default_channel_models"),
            default_channel_windows,
            default_channel_windows_json,
            default_channel_fire_sale_days_before: get_i64_in_range(
                values,
                "default_channel_fire_sale_days_before",
                0,
                3650,
            )?,
            default_channel_fire_sale_remaining_pct: get_ratio(
                values,
                "default_channel_fire_sale_remaining_pct",
            )?,
            default_channel_fire_sale_discount: get_ratio(
                values,
                "default_channel_fire_sale_discount",
            )?,
            default_channel_provider_share: get_non_negative_f64(
                values,
                "default_channel_provider_share",
            )?,
        };
        settings.validate_cross_fields()?;
        Ok(settings)
    }

    pub fn fallback_price(&self) -> ModelPrice {
        ModelPrice {
            channel_id: None,
            model_pattern: "default".to_string(),
            input_price_per_1m: self.fallback_input_price_per_unit,
            output_price_per_1m: self.fallback_output_price_per_unit,
            cache_price_per_1m: self.fallback_cache_price_per_unit,
        }
    }

    fn validate_cross_fields(&self) -> AppResult<()> {
        if self.surge_low_threshold >= self.surge_high_threshold {
            return Err(AppError::BadRequest(
                "surge_low_threshold must be lower than surge_high_threshold".to_string(),
            ));
        }
        if (self.pricing_unit_tokens - 1_000_000.0).abs() > f64::EPSILON {
            return Err(AppError::BadRequest(
                "pricing_unit_tokens is fixed at 1000000".to_string(),
            ));
        }
        validate_provider(&self.default_channel_provider)?;
        validate_default_windows(&self.default_channel_windows)?;
        Ok(())
    }
}

pub fn setting_default(key: &str) -> Option<&'static str> {
    SETTING_DEFAULTS
        .iter()
        .find_map(|(candidate, value)| (*candidate == key).then_some(*value))
}

pub fn validate_setting_value(key: &str, value: &str) -> AppResult<()> {
    if setting_default(key).is_none() {
        return Ok(());
    }
    let mut values = default_map();
    values.insert(key.to_string(), value.to_string());
    RuntimeSettings::from_map(&values).map(|_| ())
}

pub fn default_map() -> HashMap<String, String> {
    SETTING_DEFAULTS
        .iter()
        .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
        .collect()
}

fn get_string(values: &HashMap<String, String>, key: &str) -> String {
    values
        .get(key)
        .cloned()
        .or_else(|| setting_default(key).map(ToString::to_string))
        .unwrap_or_default()
}

fn get_non_negative_f64(values: &HashMap<String, String>, key: &str) -> AppResult<f64> {
    let value = parse_f64_setting(key, &get_string(values, key))?;
    if value < 0.0 {
        return Err(AppError::BadRequest(format!("{key} must be non-negative")));
    }
    Ok(value)
}

fn get_positive_f64(values: &HashMap<String, String>, key: &str) -> AppResult<f64> {
    let value = parse_f64_setting(key, &get_string(values, key))?;
    if value <= 0.0 {
        return Err(AppError::BadRequest(format!("{key} must be positive")));
    }
    Ok(value)
}

fn get_ratio(values: &HashMap<String, String>, key: &str) -> AppResult<f64> {
    let value = parse_f64_setting(key, &get_string(values, key))?;
    if !(0.0..=1.0).contains(&value) {
        return Err(AppError::BadRequest(format!(
            "{key} must be a finite ratio between 0 and 1"
        )));
    }
    Ok(value)
}

fn get_i64_in_range(
    values: &HashMap<String, String>,
    key: &str,
    min: i64,
    max: i64,
) -> AppResult<i64> {
    let value = get_string(values, key).parse::<i64>().map_err(|_| {
        AppError::BadRequest(format!("{key} must be an integer between {min} and {max}"))
    })?;
    if value < min || value > max {
        return Err(AppError::BadRequest(format!(
            "{key} must be an integer between {min} and {max}"
        )));
    }
    Ok(value)
}

fn get_u32_in_range(
    values: &HashMap<String, String>,
    key: &str,
    min: u32,
    max: u32,
) -> AppResult<u32> {
    let value = get_string(values, key).parse::<u32>().map_err(|_| {
        AppError::BadRequest(format!("{key} must be an integer between {min} and {max}"))
    })?;
    if value < min || value > max {
        return Err(AppError::BadRequest(format!(
            "{key} must be an integer between {min} and {max}"
        )));
    }
    Ok(value)
}

fn get_u64_in_range(
    values: &HashMap<String, String>,
    key: &str,
    min: u64,
    max: u64,
) -> AppResult<u64> {
    let value = get_string(values, key).parse::<u64>().map_err(|_| {
        AppError::BadRequest(format!("{key} must be an integer between {min} and {max}"))
    })?;
    if value < min || value > max {
        return Err(AppError::BadRequest(format!(
            "{key} must be an integer between {min} and {max}"
        )));
    }
    Ok(value)
}

fn get_usize_in_range(
    values: &HashMap<String, String>,
    key: &str,
    min: usize,
    max: usize,
) -> AppResult<usize> {
    let value = get_string(values, key).parse::<usize>().map_err(|_| {
        AppError::BadRequest(format!("{key} must be an integer between {min} and {max}"))
    })?;
    if value < min || value > max {
        return Err(AppError::BadRequest(format!(
            "{key} must be an integer between {min} and {max}"
        )));
    }
    Ok(value)
}

fn parse_f64_setting(key: &str, value: &str) -> AppResult<f64> {
    let parsed = value
        .trim()
        .parse::<f64>()
        .map_err(|_| AppError::BadRequest(format!("{key} must be a finite number")))?;
    if !parsed.is_finite() {
        return Err(AppError::BadRequest(format!(
            "{key} must be a finite number"
        )));
    }
    Ok(parsed)
}

fn parse_bool_setting(key: &str, value: &str) -> AppResult<bool> {
    match value.trim() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(AppError::BadRequest(format!("{key} must be true or false"))),
    }
}

fn parse_default_channel_windows(value: &str) -> AppResult<Vec<DefaultChannelWindow>> {
    serde_json::from_str(value).map_err(|_| {
        AppError::BadRequest(
            "default_channel_windows_json must be a JSON array of channel window defaults"
                .to_string(),
        )
    })
}

fn validate_provider(provider: &str) -> AppResult<()> {
    crate::models::ProviderKind::try_from(provider)
        .map(|_| ())
        .map_err(|err| AppError::BadRequest(err.to_string()))
}

fn validate_default_windows(windows: &[DefaultChannelWindow]) -> AppResult<()> {
    if windows.is_empty() {
        return Err(AppError::BadRequest(
            "default_channel_windows_json must contain at least one window".to_string(),
        ));
    }
    for window in windows {
        if window.name.trim().is_empty() {
            return Err(AppError::BadRequest(
                "default channel window name cannot be empty".to_string(),
            ));
        }
        if window.limit_points <= 0.0
            || !window.limit_points.is_finite()
            || window.period_count <= 0
        {
            return Err(AppError::BadRequest(
                "default channel window point limits and period counts must be positive"
                    .to_string(),
            ));
        }
        if !matches!(
            window.period_unit.as_str(),
            "minute" | "hour" | "day" | "week" | "month" | "year"
        ) {
            return Err(AppError::BadRequest(format!(
                "unsupported default channel window period unit: {}",
                window.period_unit
            )));
        }
        window.timezone.parse::<chrono_tz::Tz>().map_err(|_| {
            AppError::BadRequest(format!(
                "invalid default channel window timezone: {}",
                window.timezone
            ))
        })?;
    }
    Ok(())
}
