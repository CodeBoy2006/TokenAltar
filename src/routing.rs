use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use rand::Rng;
use tokio::sync::Mutex;

use crate::{
    affinity::AffinityHit,
    error::{AppError, AppResult},
    models::Channel,
    pricing::is_fire_sale,
};

#[derive(Clone, Default)]
pub struct RuntimeRouterState {
    cooldowns: Arc<Mutex<HashMap<i64, Instant>>>,
}

#[derive(Debug, Clone)]
pub struct RouteDecision {
    pub channel: Channel,
    pub affinity_hit: Option<AffinityHit>,
    pub fire_sale: bool,
}

impl RuntimeRouterState {
    pub async fn mark_cooldown(&self, channel_id: i64, duration: Duration) {
        self.cooldowns
            .lock()
            .await
            .insert(channel_id, Instant::now() + duration);
    }

    async fn is_cooling(&self, channel_id: i64) -> bool {
        let mut cooldowns = self.cooldowns.lock().await;
        if let Some(until) = cooldowns.get(&channel_id).copied() {
            if until > Instant::now() {
                return true;
            }
            cooldowns.remove(&channel_id);
        }
        false
    }
}

pub async fn choose_channel(
    channels: &[Channel],
    model: &str,
    affinity_hit: Option<AffinityHit>,
    runtime: &RuntimeRouterState,
) -> AppResult<RouteDecision> {
    let healthy = filter_healthy(channels, model, runtime).await;
    if healthy.is_empty() {
        return Err(AppError::BadRequest(
            "no healthy channel for requested model".to_string(),
        ));
    }

    if let Some(hit) = affinity_hit.clone()
        && let Some(channel_id) = hit.channel_id
    {
        if let Some(channel) = healthy.iter().find(|channel| channel.id == channel_id) {
            return Ok(RouteDecision {
                channel: (*channel).clone(),
                affinity_hit: Some(hit),
                fire_sale: is_fire_sale(channel),
            });
        }
        if hit.rule.skip_retry_on_failure {
            return Err(AppError::Upstream(
                "affinity channel unavailable and retry disabled".to_string(),
            ));
        }
    }

    if let Some(channel) = weighted_choice(&healthy, true) {
        return Ok(RouteDecision {
            fire_sale: is_fire_sale(&channel),
            channel,
            affinity_hit,
        });
    }

    let channel = weighted_choice(&healthy, false).expect("healthy not empty");
    Ok(RouteDecision {
        fire_sale: is_fire_sale(&channel),
        channel,
        affinity_hit,
    })
}

async fn filter_healthy<'a>(
    channels: &'a [Channel],
    model: &str,
    runtime: &RuntimeRouterState,
) -> Vec<&'a Channel> {
    let mut healthy = Vec::new();
    for channel in channels {
        if !channel.enabled || channel.status != "healthy" || runtime.is_cooling(channel.id).await {
            continue;
        }
        if !channel.models.is_empty()
            && !channel.models.iter().any(|item| model_matches(item, model))
        {
            continue;
        }
        let remaining_cycle = channel.limits.cycle_limit_tokens - channel.limits.used_cycle_tokens;
        let remaining_day = channel.limits.daily_limit_tokens - channel.limits.used_day_tokens;
        let remaining_hour = channel.limits.hourly_limit_tokens - channel.limits.used_hour_tokens;
        if remaining_cycle <= 0 || remaining_day <= 0 || remaining_hour <= 0 {
            continue;
        }
        healthy.push(channel);
    }
    healthy
}

fn weighted_choice(channels: &[&Channel], fire_sale_only: bool) -> Option<Channel> {
    let candidates = channels
        .iter()
        .copied()
        .filter(|channel| !fire_sale_only || is_fire_sale(channel))
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return None;
    }
    let weights = candidates
        .iter()
        .map(|channel| {
            let remaining = channel.limits.cycle_limit_tokens - channel.limits.used_cycle_tokens;
            let mut weight = remaining.max(1) as f64;
            if is_fire_sale(channel) {
                weight *= 5.0;
            }
            weight
        })
        .collect::<Vec<_>>();
    let total = weights.iter().sum::<f64>();
    let mut point = rand::rng().random_range(0.0..total);
    for (channel, weight) in candidates.iter().zip(weights.iter()) {
        if point <= *weight {
            return Some((*channel).clone());
        }
        point -= weight;
    }
    candidates.last().map(|channel| (*channel).clone())
}

fn model_matches(pattern: &str, model: &str) -> bool {
    pattern == "*" || pattern == model || model.starts_with(pattern.trim_end_matches('*'))
}

#[cfg(test)]
mod tests {
    use crate::models::{ChannelLimits, ProviderKind};

    use super::*;

    #[tokio::test]
    async fn skips_cooling_channel() {
        let runtime = RuntimeRouterState::default();
        runtime.mark_cooldown(1, Duration::from_secs(30)).await;
        let channels = vec![
            Channel {
                id: 1,
                owner_user_id: 1,
                name: "cooling".to_string(),
                provider: ProviderKind::OpenAi,
                base_url: "http://example.test".to_string(),
                api_key_secret: "x".to_string(),
                models: vec!["*".to_string()],
                enabled: true,
                status: "healthy".to_string(),
                limits: limits(100),
            },
            Channel {
                id: 2,
                owner_user_id: 1,
                name: "healthy".to_string(),
                provider: ProviderKind::OpenAi,
                base_url: "http://example.test".to_string(),
                api_key_secret: "x".to_string(),
                models: vec!["*".to_string()],
                enabled: true,
                status: "healthy".to_string(),
                limits: limits(100),
            },
        ];
        let decision = choose_channel(&channels, "gpt-test", None, &runtime)
            .await
            .unwrap();
        assert_eq!(decision.channel.id, 2);
    }

    fn limits(remaining: i64) -> ChannelLimits {
        ChannelLimits {
            cycle_limit_tokens: remaining,
            cycle_reset_day: 1,
            daily_limit_tokens: remaining,
            hourly_limit_tokens: remaining,
            used_cycle_tokens: 0,
            used_day_tokens: 0,
            used_hour_tokens: 0,
            fire_sale_days_before: 3,
            fire_sale_remaining_pct: 0.25,
            fire_sale_discount: 0.2,
            provider_share: 0.7,
        }
    }
}
