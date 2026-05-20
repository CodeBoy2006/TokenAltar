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
    required_points_by_channel: &HashMap<i64, f64>,
    runtime: &RuntimeRouterState,
    fire_sale_weight_multiplier: f64,
) -> AppResult<RouteDecision> {
    let healthy = filter_healthy(channels, model, required_points_by_channel, runtime).await;
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

    let channel =
        weighted_choice(&healthy, fire_sale_weight_multiplier).expect("healthy not empty");
    Ok(RouteDecision {
        fire_sale: is_fire_sale(&channel),
        channel,
        affinity_hit,
    })
}

async fn filter_healthy<'a>(
    channels: &'a [Channel],
    model: &str,
    required_points_by_channel: &HashMap<i64, f64>,
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
        let required_points = required_points_by_channel
            .get(&channel.id)
            .copied()
            .unwrap_or(f64::INFINITY);
        if !has_quota_for_estimate(channel, required_points) {
            continue;
        }
        healthy.push(channel);
    }
    healthy
}

fn has_quota_for_estimate(channel: &Channel, required_points: f64) -> bool {
    required_points.is_finite()
        && !channel.limits.windows.is_empty()
        && channel.limits.windows.iter().all(|window| {
            window.limit_points - window.used_points + f64::EPSILON >= required_points
        })
}

fn weighted_choice(channels: &[&Channel], fire_sale_weight_multiplier: f64) -> Option<Channel> {
    let candidates = channels.to_vec();
    if candidates.is_empty() {
        return None;
    }
    let weights = candidates
        .iter()
        .map(|channel| {
            let remaining = channel
                .limits
                .windows
                .first()
                .map(|window| window.limit_points - window.used_points)
                .unwrap_or_default();
            let mut weight = remaining.max(f64::EPSILON);
            if is_fire_sale(channel) {
                weight *= fire_sale_weight_multiplier;
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
    use chrono::{Duration as ChronoDuration, Utc};

    use crate::models::{ChannelLimits, ChannelQuotaWindow, ProviderKind};

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
                health_checked_at: None,
                upstream_latency_ms: None,
                last_error: None,
                limits: limits(100.0),
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
                health_checked_at: None,
                upstream_latency_ms: None,
                last_error: None,
                limits: limits(100.0),
            },
        ];
        let required_points = HashMap::from([(1, 1.0), (2, 1.0)]);
        let decision = choose_channel(&channels, "gpt-test", None, &required_points, &runtime, 5.0)
            .await
            .unwrap();
        assert_eq!(decision.channel.id, 2);
    }

    #[tokio::test]
    async fn skips_channel_without_enough_quota_for_estimate() {
        let runtime = RuntimeRouterState::default();
        let channels = vec![
            Channel {
                id: 1,
                owner_user_id: 1,
                name: "tiny".to_string(),
                provider: ProviderKind::OpenAi,
                base_url: "http://example.test".to_string(),
                api_key_secret: "x".to_string(),
                models: vec!["*".to_string()],
                enabled: true,
                status: "healthy".to_string(),
                health_checked_at: None,
                upstream_latency_ms: None,
                last_error: None,
                limits: limits(0.01),
            },
            Channel {
                id: 2,
                owner_user_id: 1,
                name: "roomy".to_string(),
                provider: ProviderKind::OpenAi,
                base_url: "http://example.test".to_string(),
                api_key_secret: "x".to_string(),
                models: vec!["*".to_string()],
                enabled: true,
                status: "healthy".to_string(),
                health_checked_at: None,
                upstream_latency_ms: None,
                last_error: None,
                limits: limits(100.0),
            },
        ];
        let required_points = HashMap::from([(1, 1.0), (2, 1.0)]);

        let decision = choose_channel(&channels, "gpt-test", None, &required_points, &runtime, 5.0)
            .await
            .unwrap();

        assert_eq!(decision.channel.id, 2);
    }

    #[tokio::test]
    async fn does_not_force_fire_sale_over_higher_weight_regular_channel() {
        let runtime = RuntimeRouterState::default();
        let channels = vec![
            Channel {
                id: 1,
                owner_user_id: 1,
                name: "regular-large".to_string(),
                provider: ProviderKind::OpenAi,
                base_url: "http://example.test".to_string(),
                api_key_secret: "x".to_string(),
                models: vec!["*".to_string()],
                enabled: true,
                status: "healthy".to_string(),
                health_checked_at: None,
                upstream_latency_ms: None,
                last_error: None,
                limits: non_fire_sale_limits(100.0),
            },
            Channel {
                id: 2,
                owner_user_id: 1,
                name: "fire-sale-tiny".to_string(),
                provider: ProviderKind::OpenAi,
                base_url: "http://example.test".to_string(),
                api_key_secret: "x".to_string(),
                models: vec!["*".to_string()],
                enabled: true,
                status: "healthy".to_string(),
                health_checked_at: None,
                upstream_latency_ms: None,
                last_error: None,
                limits: fire_sale_limits(0.01),
            },
        ];
        let required_points = HashMap::from([(1, 0.001), (2, 0.001)]);

        let decision = choose_channel(&channels, "gpt-test", None, &required_points, &runtime, 0.0)
            .await
            .unwrap();

        assert_eq!(decision.channel.id, 1);
    }

    fn limits(remaining: f64) -> ChannelLimits {
        ChannelLimits {
            windows: vec![ChannelQuotaWindow {
                id: 1,
                name: "Primary".to_string(),
                limit_points: remaining,
                used_points: 0.0,
                period_unit: "month".to_string(),
                period_count: 1,
                anchor_at: "2026-05-01T00:00:00".to_string(),
                timezone: "UTC".to_string(),
                current_window_start_at: "2026-05-01T00:00:00Z".to_string(),
                current_window_end_at: "2026-06-01T00:00:00Z".to_string(),
                sort_order: 0,
            }],
            fire_sale_days_before: 3,
            fire_sale_remaining_pct: 0.25,
            fire_sale_discount: 0.2,
        }
    }

    fn non_fire_sale_limits(remaining: f64) -> ChannelLimits {
        let mut limits = limits(remaining);
        limits.fire_sale_days_before = 0;
        limits
    }

    fn fire_sale_limits(remaining: f64) -> ChannelLimits {
        let mut limits = limits(remaining);
        limits.windows[0].current_window_end_at =
            (Utc::now() + ChronoDuration::days(1)).to_rfc3339();
        limits
    }
}
