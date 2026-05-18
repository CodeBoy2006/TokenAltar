use regex::Regex;

use crate::{
    models::{Channel, ModelPrice, Usage},
    tokenizer::estimate_text_tokens,
};

#[derive(Debug, Clone)]
pub struct Settlement {
    pub total_points: f64,
    pub provider_points: f64,
    pub formula_note: String,
}

pub fn select_price(model: &str, prices: &[ModelPrice]) -> ModelPrice {
    scoped_match(model, prices, true)
        .or_else(|| scoped_default(prices, true))
        .or_else(|| scoped_match(model, prices, false))
        .or_else(|| scoped_default(prices, false))
        .unwrap_or(ModelPrice {
            channel_id: None,
            model_pattern: "default".to_string(),
            input_price_per_1k: 1.0,
            output_price_per_1k: 3.0,
            cache_price_per_1k: 0.2,
        })
}

fn scoped_match(model: &str, prices: &[ModelPrice], channel_scoped: bool) -> Option<ModelPrice> {
    prices
        .iter()
        .filter(|price| price.channel_id.is_some() == channel_scoped)
        .filter(|price| price.model_pattern != "default")
        .find(|price| {
            Regex::new(&price.model_pattern)
                .map(|regex| regex.is_match(model))
                .unwrap_or(false)
        })
        .cloned()
}

fn scoped_default(prices: &[ModelPrice], channel_scoped: bool) -> Option<ModelPrice> {
    prices
        .iter()
        .find(|price| price.channel_id.is_some() == channel_scoped && price.model_pattern == "default")
        .cloned()
}

pub fn settle(
    usage: &Usage,
    price: &ModelPrice,
    surge_multiplier: f64,
    fire_sale_discount: f64,
    provider_share: f64,
) -> Settlement {
    let base = (usage.input_tokens as f64 * price.input_price_per_1k
        + usage.output_tokens as f64 * price.output_price_per_1k
        + usage.cache_tokens as f64 * price.cache_price_per_1k)
        / 1000.0;
    let total_points = round4(base * surge_multiplier * fire_sale_discount);
    let provider_points = round4(total_points * provider_share);
    let formula_note = format!(
        "input {} * {:.4}/1k + cache {} * {:.4}/1k + output {} * {:.4}/1k, surge {:.2}x, fire sale {:.2}x",
        usage.input_tokens,
        price.input_price_per_1k,
        usage.cache_tokens,
        price.cache_price_per_1k,
        usage.output_tokens,
        price.output_price_per_1k,
        surge_multiplier,
        fire_sale_discount
    );
    Settlement {
        total_points,
        provider_points,
        formula_note,
    }
}

pub fn reserve_cost(text: &str, price: &ModelPrice) -> f64 {
    estimate_text_tokens("default", text).tokens as f64 * price.input_price_per_1k / 1000.0
}

pub fn fire_sale_discount(channel: &Channel) -> f64 {
    if is_fire_sale(channel) {
        channel.limits.fire_sale_discount
    } else {
        1.0
    }
}

pub fn is_fire_sale(channel: &Channel) -> bool {
    let remaining = channel.limits.cycle_limit_tokens - channel.limits.used_cycle_tokens;
    if channel.limits.cycle_limit_tokens <= 0 {
        return false;
    }
    let remaining_pct = remaining as f64 / channel.limits.cycle_limit_tokens as f64;
    remaining_pct > channel.limits.fire_sale_remaining_pct
        && channel.limits.fire_sale_days_before > 0
}

fn round4(value: f64) -> f64 {
    (value * 10000.0).round() / 10000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settlement_applies_all_multipliers() {
        let settlement = settle(
            &Usage {
                input_tokens: 1000,
                output_tokens: 1000,
                cache_tokens: 500,
            },
            &ModelPrice {
                channel_id: None,
                model_pattern: "default".to_string(),
                input_price_per_1k: 1.0,
                output_price_per_1k: 3.0,
                cache_price_per_1k: 0.2,
            },
            0.5,
            0.2,
            0.7,
        );
        assert_eq!(settlement.total_points, 0.41);
        assert_eq!(settlement.provider_points, 0.287);
    }

    #[test]
    fn channel_price_scope_precedes_global_scope() {
        let price = select_price(
            "gpt-special",
            &[
                ModelPrice {
                    channel_id: Some(7),
                    model_pattern: "default".to_string(),
                    input_price_per_1k: 9.0,
                    output_price_per_1k: 9.0,
                    cache_price_per_1k: 0.0,
                },
                ModelPrice {
                    channel_id: None,
                    model_pattern: "gpt-special".to_string(),
                    input_price_per_1k: 1.0,
                    output_price_per_1k: 1.0,
                    cache_price_per_1k: 0.0,
                },
            ],
        );
        assert_eq!(price.channel_id, Some(7));
        assert_eq!(price.input_price_per_1k, 9.0);
    }
}
