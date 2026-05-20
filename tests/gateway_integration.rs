use std::{net::SocketAddr, time::Duration};

use axum::{
    Json, Router,
    body::Body,
    http::{StatusCode, header},
    response::Response,
    routing::post,
};
use bytes::Bytes;
use serde_json::{Value, json};
use tokenaltar::{
    app::{AppState, build_router},
    config::Config,
    db::{AffinityRuleInput, ChannelInput, ChannelQuotaWindowInput},
    models::ModelPrice,
};
use tower::ServiceExt;

#[tokio::test]
async fn built_in_affinity_presets_cover_model_families() {
    let state = AppState::new(&test_config("sqlite::memory:"))
        .await
        .unwrap();
    let rules = state.db.list_affinity_rules().await.unwrap();

    let gpt = rules
        .iter()
        .find(|rule| rule.name == "gpt prompt cache")
        .unwrap();
    assert_eq!(gpt.model_regex.as_deref(), Some("^gpt-.*$"));
    assert_eq!(gpt.request_path.as_deref(), Some("/v1/responses"));
    assert_eq!(gpt.key_source_type, "json_path");
    assert_eq!(gpt.key_source_path, "prompt_cache_key");
    assert!(gpt.skip_retry_on_failure);
    assert!(gpt.switch_on_success);
    assert!(!gpt.include_model_name);

    let claude = rules
        .iter()
        .find(|rule| rule.name == "claude metadata user")
        .unwrap();
    assert_eq!(claude.model_regex.as_deref(), Some("^claude-.*$"));
    assert_eq!(claude.request_path.as_deref(), Some("/v1/messages"));
    assert_eq!(claude.key_source_path, "metadata.user_id");
    assert_eq!(claude.ttl_seconds, 3600);
    assert!(!claude.include_model_name);

    let gemini_generate = rules
        .iter()
        .find(|rule| rule.name == "gemini cached content")
        .unwrap();
    assert_eq!(gemini_generate.model_regex.as_deref(), Some("^gemini-.*$"));
    assert_eq!(
        gemini_generate.request_path.as_deref(),
        Some("/v1beta/models/:generateContent")
    );
    assert_eq!(gemini_generate.key_source_path, "cachedContent");
    assert!(!gemini_generate.include_model_name);

    let gemini_stream = rules
        .iter()
        .find(|rule| rule.name == "gemini cached content stream")
        .unwrap();
    assert_eq!(
        gemini_stream.request_path.as_deref(),
        Some("/v1beta/models/:streamGenerateContent")
    );
    assert_eq!(gemini_stream.key_source_path, "cachedContent");
    assert!(!gemini_stream.include_model_name);
}

#[tokio::test]
async fn built_in_price_presets_use_one_million_token_unit() {
    let state = AppState::new(&test_config("sqlite::memory:"))
        .await
        .unwrap();
    let admin = state
        .db
        .create_user("pricing-admin@example.com", "password123", "Pricing Admin")
        .await
        .unwrap();
    let prices = state.db.list_prices(&admin).await.unwrap();

    let gpt_55 = prices
        .iter()
        .find(|price| price.model_pattern == r"^gpt-5\.5$")
        .unwrap();
    assert_eq!(gpt_55.input_price_per_1m, 5.0);
    assert_eq!(gpt_55.output_price_per_1m, 30.0);
    assert_eq!(gpt_55.cache_price_per_1m, 0.5);

    let gpt_54 = prices
        .iter()
        .find(|price| price.model_pattern == r"^gpt-5\.4$")
        .unwrap();
    assert_eq!(gpt_54.input_price_per_1m, 2.5);
    assert_eq!(gpt_54.output_price_per_1m, 15.0);
    assert_eq!(gpt_54.cache_price_per_1m, 0.25);

    let opus_47 = prices
        .iter()
        .find(|price| price.model_pattern == r"^claude-opus-4[.-]7(-.+)?$")
        .unwrap();
    assert_eq!(opus_47.input_price_per_1m, 5.0);
    assert_eq!(opus_47.output_price_per_1m, 25.0);
    assert_eq!(opus_47.cache_price_per_1m, 0.5);

    let haiku_45 = prices
        .iter()
        .find(|price| price.model_pattern == r"^claude-haiku-4[.-]5(-.+)?$")
        .unwrap();
    assert_eq!(haiku_45.input_price_per_1m, 1.0);
    assert_eq!(haiku_45.output_price_per_1m, 5.0);
    assert_eq!(haiku_45.cache_price_per_1m, 0.1);

    assert_eq!(
        state
            .db
            .runtime_settings()
            .await
            .unwrap()
            .pricing_unit_tokens,
        1_000_000.0
    );
}

#[tokio::test]
async fn openai_responses_gateway_settles_ledger_and_limits() {
    let upstream = spawn_upstream(json!({
        "id": "resp_test",
        "model": "gpt-test",
        "output": [{"type": "message", "content": [{"type": "output_text", "text": "ok"}]}],
        "usage": {"input_tokens": 12, "output_tokens": 4}
    }))
    .await;
    let (state, token) = setup_state(upstream).await;
    let app = build_router(state.clone());

    let response = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/responses")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(axum::body::Body::from(
                    json!({
                        "model": "gpt-test",
                        "input": "hello",
                        "stream": false
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    tokio::time::sleep(Duration::from_millis(100)).await;
    let ledger = state.db.list_ledger(None).await.unwrap();
    assert_eq!(ledger.len(), 1);
    assert_eq!(ledger[0]["input_tokens"], 12);
    assert_eq!(ledger[0]["output_tokens"], 4);
    let channel = state.db.get_channel(1).await.unwrap();
    assert_approx_eq(channel.limits.windows[0].used_points, 0.0001);
}

#[tokio::test]
async fn anthropic_messages_gateway_converts_response_shape() {
    let upstream = spawn_upstream(json!({
        "id": "resp_test",
        "model": "gpt-test",
        "output": [{"type": "message", "content": [{"type": "output_text", "text": "anthropic ok"}]}],
        "usage": {"input_tokens": 8, "output_tokens": 3}
    }))
    .await;
    let (state, token) = setup_state(upstream).await;
    let app = build_router(state);

    let response = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/messages")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(axum::body::Body::from(
                    json!({
                        "model": "gpt-test",
                        "messages": [{"role": "user", "content": "hello"}],
                        "max_tokens": 64,
                        "stream": false
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let value: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(value["type"], "message");
    assert_eq!(value["content"][0]["text"], "anthropic ok");
}

#[tokio::test]
async fn chat_completions_gateway_converts_response_shape_and_records_tokenizer() {
    let upstream = spawn_upstream(json!({
        "id": "resp_test",
        "model": "gpt-4o-mini",
        "output": [{"type": "message", "content": [{"type": "output_text", "text": "chat ok"}]}],
        "usage": {"input_tokens": 9, "output_tokens": 5}
    }))
    .await;
    let (state, token) = setup_state(upstream).await;
    let app = build_router(state.clone());

    let response = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(axum::body::Body::from(
                    json!({
                        "model": "gpt-4o-mini",
                        "messages": [
                            {"role": "system", "content": "short"},
                            {"role": "user", "content": "hello"}
                        ],
                        "stream": false
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let value: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(value["object"], "chat.completion");
    assert_eq!(value["choices"][0]["message"]["content"], "chat ok");

    tokio::time::sleep(Duration::from_millis(100)).await;
    let ledger = state.db.list_ledger(None).await.unwrap();
    assert_eq!(ledger[0]["tokenizer"], "o200k_base");
}

#[tokio::test]
async fn exhausted_channel_is_marked_unavailable() {
    let upstream = spawn_upstream(json!({
        "id": "resp_test",
        "model": "gpt-test",
        "output": [{"type": "message", "content": [{"type": "output_text", "text": "ok"}]}],
        "usage": {"input_tokens": 12, "output_tokens": 4}
    }))
    .await;
    let (state, token) = setup_state(upstream).await;
    sqlx::query("UPDATE channel_quota_windows SET used_points = limit_points WHERE channel_id = 1")
        .execute(&state.db.pool)
        .await
        .unwrap();
    state.db.refresh_channel_windows().await.unwrap();
    let app = build_router(state);
    let response = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/responses")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(axum::body::Body::from(
                    json!({"model": "gpt-test", "input": "hello"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn routing_skips_channel_that_cannot_cover_estimated_reserve() {
    let upstream = spawn_upstream(json!({
        "id": "resp_small_quota_should_not_be_used",
        "model": "gpt-test",
        "output": [{"type": "message", "content": [{"type": "output_text", "text": "wrong"}]}],
        "usage": {"input_tokens": 8, "output_tokens": 2}
    }))
    .await;
    let backup = spawn_upstream(json!({
        "id": "resp_large_quota",
        "model": "gpt-test",
        "output": [{"type": "message", "content": [{"type": "output_text", "text": "backup ok"}]}],
        "usage": {"input_tokens": 8, "output_tokens": 2}
    }))
    .await;
    let (state, token) = setup_state(upstream).await;
    add_test_channel(&state, backup, "openai").await;
    sqlx::query("UPDATE channel_quota_windows SET used_points = limit_points - 0.000001 WHERE channel_id = 1")
        .execute(&state.db.pool)
        .await
        .unwrap();
    let app = build_router(state.clone());

    let response = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/responses")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(axum::body::Body::from(
                    json!({"model": "gpt-test", "input": "hello"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    tokio::time::sleep(Duration::from_millis(100)).await;
    let ledger = state.db.list_ledger(None).await.unwrap();
    assert_eq!(ledger[0]["channel_id"], 2);
}

#[tokio::test]
async fn channel_price_override_is_used_for_settlement() {
    let upstream = spawn_upstream(json!({
        "id": "resp_test",
        "model": "gpt-special",
        "output": [{"type": "message", "content": [{"type": "output_text", "text": "ok"}]}],
        "usage": {"input_tokens": 1000, "output_tokens": 1000}
    }))
    .await;
    let (state, token) = setup_state(upstream).await;
    state
        .db
        .upsert_price(&ModelPrice {
            channel_id: None,
            model_pattern: "gpt-special".to_string(),
            input_price_per_1m: 1_000.0,
            output_price_per_1m: 1_000.0,
            cache_price_per_1m: 0.0,
        })
        .await
        .unwrap();
    state
        .db
        .upsert_price(&ModelPrice {
            channel_id: Some(1),
            model_pattern: "gpt-special".to_string(),
            input_price_per_1m: 10_000.0,
            output_price_per_1m: 20_000.0,
            cache_price_per_1m: 0.0,
        })
        .await
        .unwrap();
    let app = build_router(state.clone());

    let response = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/responses")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(axum::body::Body::from(
                    json!({"model": "gpt-special", "input": "hello"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    tokio::time::sleep(Duration::from_millis(100)).await;
    let row: (f64, f64) = sqlx::query_as(
        "SELECT input_price_per_1m, output_price_per_1m FROM ledger_entries WHERE model = 'gpt-special'",
    )
    .fetch_one(&state.db.pool)
    .await
    .unwrap();
    assert_eq!(row.0, 10_000.0);
    assert_eq!(row.1, 20_000.0);
}

#[tokio::test]
async fn runtime_settings_drive_fallback_price_and_surge_multipliers() {
    let upstream = spawn_upstream(json!({
        "id": "resp_runtime_settings",
        "model": "unknown-price-model",
        "output": [{"type": "message", "content": [{"type": "output_text", "text": "ok"}]}],
        "usage": {"input_tokens": 1000, "output_tokens": 1000}
    }))
    .await;
    let (state, token) = setup_state(upstream).await;
    state
        .db
        .upsert_settings(&[
            tokenaltar::db::SettingUpdate {
                key: "fallback_input_price_per_unit".to_string(),
                value: "7000".to_string(),
            },
            tokenaltar::db::SettingUpdate {
                key: "fallback_output_price_per_unit".to_string(),
                value: "11000".to_string(),
            },
            tokenaltar::db::SettingUpdate {
                key: "fallback_cache_price_per_unit".to_string(),
                value: "0".to_string(),
            },
            tokenaltar::db::SettingUpdate {
                key: "surge_idle_multiplier".to_string(),
                value: "0.25".to_string(),
            },
        ])
        .await
        .unwrap();
    sqlx::query("DELETE FROM model_prices WHERE channel_id IS NULL")
        .execute(&state.db.pool)
        .await
        .unwrap();
    let app = build_router(state.clone());

    let response = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/responses")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(axum::body::Body::from(
                    json!({"model": "unknown-price-model", "input": "hello"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    tokio::time::sleep(Duration::from_millis(100)).await;
    let row: (f64, f64, f64, f64) = sqlx::query_as(
        "SELECT input_price_per_1m, output_price_per_1m, surge_multiplier, total_points FROM ledger_entries WHERE model = 'unknown-price-model'",
    )
    .fetch_one(&state.db.pool)
    .await
    .unwrap();
    assert_eq!(row.0, 7000.0);
    assert_eq!(row.1, 11000.0);
    assert_eq!(row.2, 0.25);
    assert_eq!(row.3, 4.5);
}

#[tokio::test]
async fn runtime_settings_drive_provider_share_settlement() {
    let upstream = spawn_upstream(json!({
        "id": "resp_provider_share",
        "model": "provider-share-model",
        "output": [{"type": "message", "content": [{"type": "output_text", "text": "ok"}]}],
        "usage": {"input_tokens": 1000, "output_tokens": 1000}
    }))
    .await;
    let (state, token) = setup_state(upstream).await;
    state
        .db
        .upsert_settings(&[tokenaltar::db::SettingUpdate {
            key: "default_channel_provider_share".to_string(),
            value: "1.25".to_string(),
        }])
        .await
        .unwrap();
    sqlx::query("UPDATE channel_limits SET provider_share = 1.0 WHERE channel_id = 1")
        .execute(&state.db.pool)
        .await
        .unwrap();
    let app = build_router(state.clone());

    let response = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/responses")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(axum::body::Body::from(
                    json!({"model": "provider-share-model", "input": "hello"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    tokio::time::sleep(Duration::from_millis(100)).await;
    let row: (f64, f64) = sqlx::query_as(
        "SELECT total_points, provider_points FROM ledger_entries WHERE model = 'provider-share-model'",
    )
    .fetch_one(&state.db.pool)
    .await
    .unwrap();
    assert_eq!(row.1, (row.0 * 1.25 * 10_000.0).round() / 10_000.0);
    assert!(row.1 > row.0);
}

#[tokio::test]
async fn routing_max_attempts_is_runtime_configurable() {
    let failing = spawn_status_upstream(StatusCode::BAD_GATEWAY).await;
    let backup = spawn_upstream(json!({
        "id": "resp_should_be_blocked_by_attempt_limit",
        "model": "gpt-test",
        "output": [{"type": "message", "content": [{"type": "output_text", "text": "unexpected"}]}],
        "usage": {"input_tokens": 8, "output_tokens": 3}
    }))
    .await;
    let (state, token) = setup_state(failing).await;
    add_test_channel(&state, backup, "openai").await;
    bind_tenant_affinity(&state, 1, false).await;
    state
        .db
        .upsert_settings(&[tokenaltar::db::SettingUpdate {
            key: "routing_max_attempts".to_string(),
            value: "1".to_string(),
        }])
        .await
        .unwrap();
    let app = build_router(state.clone());

    let response = app.oneshot(retry_request(&token, "alpha")).await.unwrap();

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(state.db.list_ledger(None).await.unwrap().is_empty());
}

#[tokio::test]
async fn openai_responses_to_openai_channel_is_passthrough() {
    let upstream = spawn_echo_upstream().await;
    let (state, token) = setup_state(upstream).await;
    let app = build_router(state.clone());

    let response = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/responses")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(axum::body::Body::from(
                    json!({
                        "model": "gpt-test",
                        "input": [{"role": "user", "content": [{"type": "input_text", "text": "hello"}]}],
                        "metadata": {"passthrough_marker": true},
                        "stream": false
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let value: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(value["output"][0]["content"][0]["text"], "saw-passthrough");
    assert_eq!(value["usage"]["input_tokens"], 11);

    tokio::time::sleep(Duration::from_millis(100)).await;
    let ledger = state.db.list_ledger(None).await.unwrap();
    assert_eq!(ledger[0]["input_tokens"], 11);
    assert_eq!(ledger[0]["output_tokens"], 2);
}

#[tokio::test]
async fn openai_chat_can_route_to_gemini_text_channel() {
    let upstream = spawn_gemini_echo_upstream().await;
    let (state, token) = setup_state_with_provider(upstream, "gemini").await;
    let app = build_router(state.clone());

    let response = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(axum::body::Body::from(
                    json!({
                        "model": "gemini-test",
                        "messages": [{"role": "user", "content": "hello gemini"}],
                        "stream": false
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let value: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(value["object"], "chat.completion");
    assert_eq!(value["choices"][0]["message"]["content"], "gemini ok");

    tokio::time::sleep(Duration::from_millis(100)).await;
    let ledger = state.db.list_ledger(None).await.unwrap();
    assert_eq!(ledger[0]["input_tokens"], 7);
    assert_eq!(ledger[0]["output_tokens"], 3);
}

#[tokio::test]
async fn openai_responses_with_image_routes_to_gemini_inline_data() {
    let image_url = spawn_image_source_upstream().await;
    let upstream = spawn_gemini_image_upstream().await;
    let (state, token) = setup_state_with_provider(upstream, "gemini").await;
    let app = build_router(state.clone());

    let response = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/responses")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(axum::body::Body::from(
                    json!({
                        "model": "gemini-test",
                        "input": [{
                            "role": "user",
                            "content": [
                                {"type": "input_text", "text": "look"},
                                {"type": "input_image", "image_url": image_url}
                            ]
                        }],
                        "stream": false
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let value: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(value["object"], "response");

    tokio::time::sleep(Duration::from_millis(100)).await;
    let ledger = state.db.list_ledger(None).await.unwrap();
    assert_eq!(ledger[0]["input_tokens"], 6);
    assert_eq!(ledger[0]["output_tokens"], 2);
}

#[tokio::test]
async fn gemini_to_gemini_channel_is_passthrough_without_internal_fields() {
    let upstream = spawn_gemini_passthrough_upstream().await;
    let (state, token) = setup_state_with_provider(upstream, "gemini").await;
    let app = build_router(state.clone());

    let response = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1beta/models/gemini-test:generateContent")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(axum::body::Body::from(
                    json!({
                        "contents": [{
                            "role": "user",
                            "parts": [{"text": "direct gemini"}]
                        }],
                        "generationConfig": {"temperature": 0.2}
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let value: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        value["candidates"][0]["content"]["parts"][0]["text"],
        "direct ok"
    );

    tokio::time::sleep(Duration::from_millis(100)).await;
    let ledger = state.db.list_ledger(None).await.unwrap();
    assert_eq!(ledger[0]["input_tokens"], 5);
    assert_eq!(ledger[0]["output_tokens"], 2);
}

#[tokio::test]
async fn gemini_to_gemini_image_passthrough_keeps_file_data() {
    let upstream = spawn_gemini_image_passthrough_upstream().await;
    let (state, token) = setup_state_with_provider(upstream, "gemini").await;
    let app = build_router(state.clone());

    let response = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1beta/models/gemini-test:generateContent")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(axum::body::Body::from(
                    json!({
                        "contents": [{
                            "role": "user",
                            "parts": [{
                                "fileData": {
                                    "fileUri": "https://local.test/direct-gemini.png",
                                    "mimeType": "image/png"
                                }
                            }]
                        }],
                        "generationConfig": {"temperature": 0.2}
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let value: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        value["candidates"][0]["content"]["parts"][0]["text"],
        "direct image ok"
    );
}

#[tokio::test]
async fn affinity_429_retries_backup_channel_and_switches_binding() {
    let failing = spawn_status_upstream(StatusCode::TOO_MANY_REQUESTS).await;
    let backup = spawn_upstream(json!({
        "id": "resp_retry",
        "model": "gpt-test",
        "output": [{"type": "message", "content": [{"type": "output_text", "text": "retry ok"}]}],
        "usage": {"input_tokens": 12, "output_tokens": 4}
    }))
    .await;
    let (state, token) = setup_state(failing).await;
    let first_channel_id = 1;
    let backup_channel_id = add_test_channel(&state, backup, "openai").await;
    let cache_key = bind_tenant_affinity(&state, first_channel_id, false).await;
    let app = build_router(state.clone());

    let response = app.oneshot(retry_request(&token, "alpha")).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    tokio::time::sleep(Duration::from_millis(100)).await;
    let ledger = state.db.list_ledger(None).await.unwrap();
    assert_eq!(ledger.len(), 1);
    assert_eq!(ledger[0]["channel_id"], backup_channel_id);
    assert_eq!(
        state
            .db
            .get_channel(first_channel_id)
            .await
            .unwrap()
            .limits
            .windows[0]
            .used_points,
        0.0
    );
    assert_approx_eq(
        state
            .db
            .get_channel(backup_channel_id)
            .await
            .unwrap()
            .limits
            .windows[0]
            .used_points,
        0.0001,
    );
    let (binding_channel_id, _) = state
        .db
        .get_affinity_binding(&cache_key)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(binding_channel_id, backup_channel_id);
}

#[tokio::test]
async fn affinity_5xx_retries_backup_channel_before_returning_to_client() {
    let failing = spawn_status_upstream(StatusCode::BAD_GATEWAY).await;
    let backup = spawn_upstream(json!({
        "id": "resp_retry_5xx",
        "model": "gpt-test",
        "output": [{"type": "message", "content": [{"type": "output_text", "text": "retry ok"}]}],
        "usage": {"input_tokens": 8, "output_tokens": 3}
    }))
    .await;
    let (state, token) = setup_state(failing).await;
    let backup_channel_id = add_test_channel(&state, backup, "openai").await;
    bind_tenant_affinity(&state, 1, false).await;
    let app = build_router(state.clone());

    let response = app.oneshot(retry_request(&token, "alpha")).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    tokio::time::sleep(Duration::from_millis(100)).await;
    let ledger = state.db.list_ledger(None).await.unwrap();
    assert_eq!(ledger.len(), 1);
    assert_eq!(ledger[0]["channel_id"], backup_channel_id);
    assert_eq!(ledger[0]["input_tokens"], 8);
    assert_eq!(ledger[0]["output_tokens"], 3);
}

#[tokio::test]
async fn semantic_empty_response_retries_backup_channel() {
    let empty = spawn_upstream(json!({
        "id": "resp_empty",
        "model": "gpt-test",
        "output": [{"type": "message", "content": [{"type": "output_text", "text": "   \n\t"}]}],
        "usage": {"input_tokens": 8, "output_tokens": 0}
    }))
    .await;
    let backup = spawn_upstream(json!({
        "id": "resp_non_empty_backup",
        "model": "gpt-test",
        "output": [{"type": "message", "content": [{"type": "output_text", "text": "backup ok"}]}],
        "usage": {"input_tokens": 8, "output_tokens": 3}
    }))
    .await;
    let (state, token) = setup_state(empty).await;
    let backup_channel_id = add_test_channel(&state, backup, "openai").await;
    bind_tenant_affinity(&state, 1, false).await;
    let app = build_router(state.clone());

    let response = app.oneshot(retry_request(&token, "alpha")).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let value: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(value["output"][0]["content"][0]["text"], "backup ok");
    tokio::time::sleep(Duration::from_millis(100)).await;
    let ledger = state.db.list_ledger(None).await.unwrap();
    assert_eq!(ledger.len(), 1);
    assert_eq!(ledger[0]["channel_id"], backup_channel_id);
    assert_eq!(
        state.db.get_channel(1).await.unwrap().limits.windows[0].used_points,
        0.0
    );
    let user = state
        .db
        .find_user_with_hash("admin@example.com")
        .await
        .unwrap()
        .unwrap()
        .0;
    let channels = state.db.list_public_channels(&user).await.unwrap();
    let empty_window = channels
        .iter()
        .find(|channel| channel.id == 1)
        .unwrap()
        .health_windows
        .iter()
        .find(|window| window.empty_count > 0)
        .unwrap();
    assert_eq!(empty_window.status, "empty");
    assert_eq!(empty_window.avg_ttft_ms, None);
    let backup_window = channels
        .iter()
        .find(|channel| channel.id == backup_channel_id)
        .unwrap()
        .health_windows
        .iter()
        .find(|window| window.success_count > 0)
        .unwrap();
    assert_eq!(backup_window.status, "available");
    assert!(backup_window.avg_ttft_ms.is_some());
}

#[tokio::test]
async fn semantic_empty_stream_retries_backup_before_client_done() {
    let empty_stream = spawn_stream_upstream(&[
        ": keepalive\n\n",
        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"   \"}\n\n",
        "data: [DONE]\n\n",
    ])
    .await;
    let backup_stream = spawn_stream_upstream(&[
        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"backup stream ok\"}\n\n",
        "data: {\"usage\":{\"input_tokens\":8,\"output_tokens\":4}}\n\n",
        "data: [DONE]\n\n",
    ])
    .await;
    let (state, token) = setup_state(empty_stream).await;
    let backup_channel_id = add_test_channel(&state, backup_stream, "openai").await;
    bind_tenant_affinity(&state, 1, false).await;
    let app = build_router(state.clone());

    let response = app
        .oneshot(stream_retry_request(&token, "alpha"))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("backup stream ok"));
    assert!(!text.contains("\"delta\":\"   \""));
    tokio::time::sleep(Duration::from_millis(100)).await;
    let ledger = state.db.list_ledger(None).await.unwrap();
    assert_eq!(ledger.len(), 1);
    assert_eq!(ledger[0]["channel_id"], backup_channel_id);
    assert_eq!(ledger[0]["input_tokens"], 8);
    assert_eq!(ledger[0]["output_tokens"], 4);
    assert_eq!(
        state.db.get_channel(1).await.unwrap().limits.windows[0].used_points,
        0.0
    );
}

#[tokio::test]
async fn affinity_skip_retry_on_failure_returns_bound_channel_error() {
    let failing = spawn_status_upstream(StatusCode::BAD_GATEWAY).await;
    let backup = spawn_upstream(json!({
        "id": "resp_should_not_retry",
        "model": "gpt-test",
        "output": [{"type": "message", "content": [{"type": "output_text", "text": "unexpected"}]}],
        "usage": {"input_tokens": 8, "output_tokens": 3}
    }))
    .await;
    let (state, token) = setup_state(failing).await;
    let backup_channel_id = add_test_channel(&state, backup, "openai").await;
    let cache_key = bind_tenant_affinity(&state, 1, true).await;
    let app = build_router(state.clone());

    let response = app.oneshot(retry_request(&token, "alpha")).await.unwrap();

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(state.db.list_ledger(None).await.unwrap().is_empty());
    assert_eq!(
        state.db.get_channel(1).await.unwrap().limits.windows[0].used_points,
        0.0
    );
    assert_eq!(
        state
            .db
            .get_channel(backup_channel_id)
            .await
            .unwrap()
            .limits
            .windows[0]
            .used_points,
        0.0
    );
    let (binding_channel_id, _) = state
        .db
        .get_affinity_binding(&cache_key)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(binding_channel_id, 1);
}

async fn setup_state(upstream: String) -> (AppState, String) {
    setup_state_with_provider(upstream, "openai").await
}

async fn setup_state_with_provider(upstream: String, provider: &str) -> (AppState, String) {
    let config = test_config("sqlite::memory:");
    let state = AppState::new(&config).await.unwrap();
    state
        .db
        .bootstrap_admin("admin@example.com", "password123")
        .await
        .unwrap();
    let user = state
        .db
        .find_user_with_hash("admin@example.com")
        .await
        .unwrap()
        .unwrap()
        .0;
    let (token, _) = state
        .db
        .create_api_key(user.id, "test", Some(1000.0))
        .await
        .unwrap();
    state
        .db
        .upsert_channel(
            user.id,
            ChannelInput {
                name: "test".to_string(),
                provider: provider.to_string(),
                base_url: upstream,
                api_key_secret: "upstream-key".to_string(),
                models: vec!["*".to_string()],
                enabled: true,
                windows: quota_windows(),
                fire_sale_days_before: 3,
                fire_sale_remaining_pct: 0.25,
                fire_sale_discount: 0.2,
            },
        )
        .await
        .unwrap();
    (state, token)
}

async fn add_test_channel(state: &AppState, upstream: String, provider: &str) -> i64 {
    let owner_user_id = state.db.get_channel(1).await.unwrap().owner_user_id;
    state
        .db
        .upsert_channel(
            owner_user_id,
            ChannelInput {
                name: format!("backup-{provider}"),
                provider: provider.to_string(),
                base_url: upstream,
                api_key_secret: "upstream-key".to_string(),
                models: vec!["*".to_string()],
                enabled: true,
                windows: quota_windows(),
                fire_sale_days_before: 3,
                fire_sale_remaining_pct: 0.25,
                fire_sale_discount: 0.2,
            },
        )
        .await
        .unwrap()
        .id
}

async fn bind_tenant_affinity(state: &AppState, channel_id: i64, skip_retry: bool) -> String {
    let rule = state
        .db
        .create_affinity_rule(AffinityRuleInput {
            name: format!("tenant-stick-{channel_id}-{skip_retry}"),
            enabled: true,
            model_regex: None,
            request_path: Some("/v1/responses".to_string()),
            user_agent_regex: None,
            key_source_type: "request_header".to_string(),
            key_source_path: "x-tenant-id".to_string(),
            group_name: "default".to_string(),
            ttl_seconds: 3600,
            skip_retry_on_failure: skip_retry,
            switch_on_success: true,
            include_model_name: true,
        })
        .await
        .unwrap();
    let cache_key = format!("{}:gpt-test:{}:alpha", rule.name, rule.group_name);
    state
        .db
        .set_affinity_binding(&cache_key, rule.id, channel_id, 3600)
        .await
        .unwrap();
    cache_key
}

fn retry_request(token: &str, tenant: &str) -> axum::http::Request<axum::body::Body> {
    axum::http::Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .header("x-tenant-id", tenant)
        .body(axum::body::Body::from(
            json!({
                "model": "gpt-test",
                "input": "hello",
                "stream": false
            })
            .to_string(),
        ))
        .unwrap()
}

fn stream_retry_request(token: &str, tenant: &str) -> axum::http::Request<axum::body::Body> {
    axum::http::Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .header("x-tenant-id", tenant)
        .body(axum::body::Body::from(
            json!({
                "model": "gpt-test",
                "input": "hello",
                "stream": true
            })
            .to_string(),
        ))
        .unwrap()
}

fn quota_windows() -> Vec<ChannelQuotaWindowInput> {
    vec![
        ChannelQuotaWindowInput {
            name: "Monthly".to_string(),
            limit_points: 1000.0,
            period_unit: "month".to_string(),
            period_count: 1,
            anchor_at: "2026-05-01T00:00:00".to_string(),
            timezone: "UTC".to_string(),
        },
        ChannelQuotaWindowInput {
            name: "Daily".to_string(),
            limit_points: 1000.0,
            period_unit: "day".to_string(),
            period_count: 1,
            anchor_at: "2026-05-18T00:00:00".to_string(),
            timezone: "UTC".to_string(),
        },
    ]
}

fn assert_approx_eq(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 1e-9,
        "expected {expected}, got {actual}"
    );
}

fn test_config(database_url: &str) -> Config {
    Config {
        bind: "127.0.0.1:0".parse().unwrap(),
        database_url: database_url.to_string(),
        admin_email: None,
        admin_password: None,
        leaderboard_timezone: None,
    }
}

async fn spawn_upstream(body: Value) -> String {
    async fn handler(Json(body): Json<Value>) -> Json<Value> {
        Json(body)
    }
    let app = Router::new()
        .route(
            "/v1/responses",
            post({
                let body = body.clone();
                move || async move { Json(body.clone()) }
            }),
        )
        .route("/v1/messages", post(handler));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

async fn spawn_status_upstream(status: StatusCode) -> String {
    let app = Router::new().route(
        "/v1/responses",
        post(move || async move {
            (
                status,
                Json(json!({
                    "error": {
                        "message": format!("temporary {status}")
                    }
                })),
            )
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

async fn spawn_stream_upstream(chunks: &[&'static str]) -> String {
    let chunks = chunks.to_vec();
    let app = Router::new().route(
        "/v1/responses",
        post(move || {
            let chunks = chunks.clone();
            async move {
                let stream = async_stream::stream! {
                    for chunk in chunks {
                        yield Ok::<_, std::convert::Infallible>(Bytes::from_static(chunk.as_bytes()));
                    }
                };
                let mut response = Response::new(Body::from_stream(stream));
                response.headers_mut().insert(
                    header::CONTENT_TYPE,
                    header::HeaderValue::from_static("text/event-stream"),
                );
                response
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

async fn spawn_echo_upstream() -> String {
    async fn responses(Json(body): Json<Value>) -> Json<Value> {
        let saw_marker = body
            .get("metadata")
            .and_then(|metadata| metadata.get("passthrough_marker"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        Json(json!({
            "id": "resp_echo",
            "model": body.get("model").cloned().unwrap_or_else(|| json!("unknown")),
            "output": [{
                "type": "message",
                "content": [{"type": "output_text", "text": if saw_marker { "saw-passthrough" } else { "converted" }}]
            }],
            "usage": {"input_tokens": 11, "output_tokens": 2}
        }))
    }
    let app = Router::new().route("/v1/responses", post(responses));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

async fn spawn_gemini_echo_upstream() -> String {
    async fn generate(Json(body): Json<Value>) -> Json<Value> {
        assert_eq!(body["contents"][0]["parts"][0]["text"], "hello gemini");
        Json(json!({
            "candidates": [{
                "content": {
                    "role": "model",
                    "parts": [{"text": "gemini ok"}]
                },
                "finishReason": "STOP"
            }],
            "usageMetadata": {
                "promptTokenCount": 7,
                "candidatesTokenCount": 3,
                "totalTokenCount": 10
            }
        }))
    }
    let app = Router::new().route("/v1beta/models/{model_action}", post(generate));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

async fn spawn_gemini_passthrough_upstream() -> String {
    async fn generate(Json(body): Json<Value>) -> Json<Value> {
        assert!(body.get("_model").is_none());
        assert!(body.get("_stream").is_none());
        assert_eq!(body["contents"][0]["parts"][0]["text"], "direct gemini");
        Json(json!({
            "candidates": [{
                "content": {
                    "role": "model",
                    "parts": [{"text": "direct ok"}]
                },
                "finishReason": "STOP"
            }],
            "usageMetadata": {
                "promptTokenCount": 5,
                "candidatesTokenCount": 2,
                "totalTokenCount": 7
            }
        }))
    }
    let app = Router::new().route("/v1beta/models/{model_action}", post(generate));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

async fn spawn_gemini_image_upstream() -> String {
    async fn generate(Json(body): Json<Value>) -> Json<Value> {
        assert_eq!(body["contents"][0]["parts"][0]["text"], "look");
        assert_eq!(
            body["contents"][0]["parts"][1]["inlineData"]["mimeType"],
            "image/png"
        );
        assert_eq!(
            body["contents"][0]["parts"][1]["inlineData"]["data"],
            "aW1hZ2UtYnl0ZXM="
        );
        Json(json!({
            "candidates": [{
                "content": {
                    "role": "model",
                    "parts": [{"text": "gemini image ok"}]
                },
                "finishReason": "STOP"
            }],
            "usageMetadata": {
                "promptTokenCount": 6,
                "candidatesTokenCount": 2,
                "totalTokenCount": 8
            }
        }))
    }
    let app = Router::new().route("/v1beta/models/{model_action}", post(generate));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

async fn spawn_gemini_image_passthrough_upstream() -> String {
    async fn generate(Json(body): Json<Value>) -> Json<Value> {
        assert_eq!(
            body["contents"][0]["parts"][0]["fileData"]["fileUri"],
            "https://local.test/direct-gemini.png"
        );
        assert_eq!(
            body["contents"][0]["parts"][0]["fileData"]["mimeType"],
            "image/png"
        );
        Json(json!({
            "candidates": [{
                "content": {
                    "role": "model",
                    "parts": [{"text": "direct image ok"}]
                },
                "finishReason": "STOP"
            }],
            "usageMetadata": {
                "promptTokenCount": 5,
                "candidatesTokenCount": 1,
                "totalTokenCount": 6
            }
        }))
    }
    let app = Router::new().route("/v1beta/models/{model_action}", post(generate));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

async fn spawn_image_source_upstream() -> String {
    async fn image() -> axum::http::Response<axum::body::Body> {
        let mut response = axum::http::Response::new(axum::body::Body::from("image-bytes"));
        response.headers_mut().insert(
            axum::http::header::CONTENT_TYPE,
            axum::http::HeaderValue::from_static("image/png"),
        );
        response
    }
    let app = Router::new().route(
        "/tokenaltar-gemini.png",
        axum::routing::get(|| async { image().await }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}/tokenaltar-gemini.png")
}
