use std::{num::NonZeroUsize, sync::Arc};

use axum::http::HeaderMap;
use chrono::{DateTime, Utc};
use lru::LruCache;
use regex::Regex;
use serde_json::Value;
use tokio::sync::Mutex;

use crate::{
    db::Database,
    error::AppResult,
    models::{AffinityRule, GatewayContext, value_to_key_fragment},
    protocol::TextRequest,
};

#[derive(Clone)]
pub struct AffinityCache {
    inner: Arc<Mutex<LruCache<String, CachedAffinity>>>,
}

#[derive(Debug, Clone)]
pub struct AffinityHit {
    pub cache_key: String,
    pub rule: AffinityRule,
    pub channel_id: Option<i64>,
}

#[derive(Debug, Clone)]
struct CachedAffinity {
    channel_id: i64,
    expires_at: DateTime<Utc>,
}

impl AffinityCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(LruCache::new(
                NonZeroUsize::new(capacity).expect("capacity > 0"),
            ))),
        }
    }

    pub async fn get(&self, key: &str) -> Option<i64> {
        let mut inner = self.inner.lock().await;
        let Some(entry) = inner.get(key).cloned() else {
            return None;
        };
        if entry.expires_at > Utc::now() {
            Some(entry.channel_id)
        } else {
            inner.pop(key);
            None
        }
    }

    pub async fn put(&self, key: String, channel_id: i64, expires_at: DateTime<Utc>) {
        self.inner.lock().await.put(
            key,
            CachedAffinity {
                channel_id,
                expires_at,
            },
        );
    }
}

pub async fn lookup_affinity(
    db: &Database,
    cache: &AffinityCache,
    request_path: &str,
    headers: &HeaderMap,
    body: &Value,
    request: &TextRequest,
    context: &GatewayContext,
) -> AppResult<Option<AffinityHit>> {
    let rules = db.list_affinity_rules().await?;
    for rule in rules {
        if !rule.enabled || !rule_matches(&rule, request_path, headers, &request.model) {
            continue;
        }
        let Some(value) = extract_value(&rule, headers, body, context) else {
            continue;
        };
        let cache_key = format!(
            "{}:{}:{}:{}",
            rule.name, request.model, rule.group_name, value
        );
        let channel_id = if let Some(channel_id) = cache.get(&cache_key).await {
            Some(channel_id)
        } else if let Some((channel_id, expires_at)) = db.get_affinity_binding(&cache_key).await? {
            if let Ok(expires_at) = DateTime::parse_from_rfc3339(&expires_at) {
                cache
                    .put(cache_key.clone(), channel_id, expires_at.with_timezone(&Utc))
                    .await;
            }
            Some(channel_id)
        } else {
            None
        };
        return Ok(Some(AffinityHit {
            cache_key,
            rule,
            channel_id,
        }));
    }
    Ok(None)
}

pub async fn remember_affinity(
    db: &Database,
    cache: &AffinityCache,
    hit: &AffinityHit,
    channel_id: i64,
) -> AppResult<()> {
    let expires_at = db
        .set_affinity_binding(
            &hit.cache_key,
            hit.rule.id,
            channel_id,
            hit.rule.ttl_seconds,
        )
        .await?;
    if let Ok(expires_at) = DateTime::parse_from_rfc3339(&expires_at) {
        cache
            .put(hit.cache_key.clone(), channel_id, expires_at.with_timezone(&Utc))
            .await;
    }
    Ok(())
}

fn rule_matches(rule: &AffinityRule, request_path: &str, headers: &HeaderMap, model: &str) -> bool {
    if let Some(path) = &rule.request_path
        && path != request_path
    {
        return false;
    }
    if let Some(model_regex) = &rule.model_regex
        && !Regex::new(model_regex)
            .map(|regex| regex.is_match(model))
            .unwrap_or(false)
    {
        return false;
    }
    if let Some(user_agent_regex) = &rule.user_agent_regex {
        let user_agent = headers
            .get(axum::http::header::USER_AGENT)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        if !Regex::new(user_agent_regex)
            .map(|regex| regex.is_match(user_agent))
            .unwrap_or(false)
        {
            return false;
        }
    }
    true
}

fn extract_value(
    rule: &AffinityRule,
    headers: &HeaderMap,
    body: &Value,
    context: &GatewayContext,
) -> Option<String> {
    match rule.key_source_type.as_str() {
        "request_header" => headers
            .get(&rule.key_source_path)
            .and_then(|value| value.to_str().ok())
            .map(ToString::to_string),
        "context" => match rule.key_source_path.as_str() {
            "user_group" => Some(context.user_group.clone()),
            "date" => Some(Utc::now().date_naive().to_string()),
            _ => None,
        },
        "json_path" => {
            simple_json_path(body, &rule.key_source_path).and_then(value_to_key_fragment)
        }
        _ => None,
    }
}

fn simple_json_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = value;
    let normalized = path.trim_start_matches('$').trim_start_matches('.');
    if normalized.is_empty() {
        return Some(current);
    }
    for segment in normalized.split('.') {
        let mut rest = segment;
        loop {
            if let Some((field, tail)) = rest.split_once('[') {
                if !field.is_empty() {
                    current = current.get(field)?;
                }
                let (idx, after) = tail.split_once(']')?;
                let index = idx.parse::<usize>().ok()?;
                current = current.get(index)?;
                rest = after.trim_start_matches('.');
                if rest.is_empty() {
                    break;
                }
            } else {
                current = current.get(rest)?;
                break;
            }
        }
    }
    Some(current)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn extracts_basic_json_path() {
        let value = json!({"messages": [{"content": "hello"}]});
        assert_eq!(
            simple_json_path(&value, "messages[0].content").unwrap(),
            "hello"
        );
    }

    #[tokio::test]
    async fn cache_drops_expired_affinity_entries() {
        let cache = AffinityCache::new(2);
        cache
            .put(
                "expired".to_string(),
                7,
                Utc::now() - chrono::Duration::seconds(1),
            )
            .await;
        assert_eq!(cache.get("expired").await, None);

        cache
            .put(
                "fresh".to_string(),
                9,
                Utc::now() + chrono::Duration::seconds(60),
            )
            .await;
        assert_eq!(cache.get("fresh").await, Some(9));
    }
}
