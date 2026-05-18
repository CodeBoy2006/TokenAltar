use axum::{body::Body, http::StatusCode};
use serde_json::{Value, json};
use tokenaltar::{
    app::{AppState, build_router},
    config::Config,
    db::{ChannelInput, LeaderboardPeriod},
};
use tower::ServiceExt;

#[tokio::test]
async fn transfer_moves_points_losslessly() {
    let state = setup_state().await;
    let alice = state
        .db
        .create_user("alice@example.com", "password123", "Alice")
        .await
        .unwrap();
    let bob = state
        .db
        .create_user("bob@example.com", "password123", "Bob")
        .await
        .unwrap();

    state
        .db
        .transfer_points(alice.id, bob.id, 25.5, Some("@TokenAltar PayTo:Bob"))
        .await
        .unwrap();

    let alice_after = state.db.get_user(alice.id).await.unwrap();
    let bob_after = state.db.get_user(bob.id).await.unwrap();
    assert_eq!(alice_after.points_balance, 974.5);
    assert_eq!(bob_after.points_balance, 1025.5);
    assert_eq!(state.db.list_transfers(alice.id).await.unwrap().len(), 1);
}

#[tokio::test]
async fn red_packet_claim_is_single_use_per_user() {
    let state = setup_state().await;
    let creator = state
        .db
        .create_user("creator@example.com", "password123", "Creator")
        .await
        .unwrap();
    let claimer = state
        .db
        .create_user("claimer@example.com", "password123", "Claimer")
        .await
        .unwrap();

    state
        .db
        .create_red_packet(creator.id, "RustIsBest", 30.0, 3, "even")
        .await
        .unwrap();
    let points = state
        .db
        .claim_red_packet(claimer.id, "RustIsBest")
        .await
        .unwrap();
    assert_eq!(points, 10.0);
    let duplicate = state.db.claim_red_packet(claimer.id, "RustIsBest").await;
    assert!(duplicate.is_err());
}

#[tokio::test]
async fn anonymous_leaderboard_masks_user_identity() {
    let state = setup_state().await;
    let user = state
        .db
        .create_user("anon@example.com", "password123", "Secret Name")
        .await
        .unwrap();
    state
        .db
        .set_anonymous_leaderboard(user.id, true)
        .await
        .unwrap();
    sqlx::query(
        r#"
        INSERT INTO ledger_entries(
          request_id, user_id, api_key_id, channel_id, provider_user_id, model, tokenizer,
          input_tokens, output_tokens, cache_tokens, input_price_per_1k, output_price_per_1k,
          cache_price_per_1k, surge_multiplier, fire_sale_discount, total_points,
          provider_points, status, formula_note
        ) VALUES ('req_lb', ?, 1, 1, ?, 'gpt-test', 'test', 10, 5, 0, 1, 3, 0, 1, 1, 1, 1, 'success', 'test')
        "#,
    )
    .bind(user.id)
    .bind(user.id)
    .execute(&state.db.pool)
    .await
    .unwrap();

    let leaderboards = state
        .db
        .leaderboards(LeaderboardPeriod::Month, None)
        .await
        .unwrap();
    assert!(
        leaderboards["providers"][0]["name"]
            .as_str()
            .unwrap()
            .starts_with("Anonymous #")
    );
    assert!(leaderboards["providers"][0]["user_id"].is_null());
}

#[tokio::test]
async fn leaderboards_support_day_period_and_skip_failed_ledger_rows() {
    let state = setup_state().await;
    let user = state
        .db
        .create_user("daily@example.com", "password123", "Daily")
        .await
        .unwrap();
    sqlx::query(
        r#"
        INSERT INTO ledger_entries(
          request_id, user_id, api_key_id, channel_id, provider_user_id, model, tokenizer,
          input_tokens, output_tokens, cache_tokens, input_price_per_1k, output_price_per_1k,
          cache_price_per_1k, surge_multiplier, fire_sale_discount, total_points,
          provider_points, status, formula_note, created_at
        ) VALUES
          ('req_daily_success', ?, 1, 1, ?, 'gpt-test', 'test', 10, 5, 0, 1, 3, 0, 1, 1, 2, 1, 'success', 'ok', datetime('now')),
          ('req_daily_error', ?, 1, 1, ?, 'gpt-test', 'test', 100, 50, 0, 1, 3, 0, 1, 1, 20, 1, 'upstream_error', 'skip', datetime('now'))
        "#,
    )
    .bind(user.id)
    .bind(user.id)
    .bind(user.id)
    .bind(user.id)
    .execute(&state.db.pool)
    .await
    .unwrap();

    let leaderboards = state
        .db
        .leaderboards(LeaderboardPeriod::Day, Some("Asia/Shanghai"))
        .await
        .unwrap();
    assert_eq!(leaderboards["period"], "day");
    assert_eq!(leaderboards["timezone"], "Asia/Shanghai");
    assert_eq!(leaderboards["providers"][0]["score"], 15.0);
    assert_eq!(leaderboards["consumers"][0]["score"], 2.0);
}

#[tokio::test]
async fn users_create_channels_and_list_only_their_masked_channels() {
    let state = setup_state().await;
    let alice = state
        .db
        .create_user("alice-channel@example.com", "password123", "Alice")
        .await
        .unwrap();
    let bob = state
        .db
        .create_user("bob-channel@example.com", "password123", "Bob")
        .await
        .unwrap();
    state.db.create_session(bob.id).await.unwrap();
    let alice_token = state.db.create_session(alice.id).await.unwrap();
    let bob_channel = state
        .db
        .upsert_channel(
            bob.id,
            ChannelInput {
                name: "bob-private".to_string(),
                provider: "openai".to_string(),
                base_url: "http://127.0.0.1:9".to_string(),
                api_key_secret: "bob-secret".to_string(),
                models: vec!["gpt-bob".to_string()],
                enabled: true,
                cycle_limit_tokens: 1000,
                cycle_reset_day: 1,
                daily_limit_tokens: 1000,
                hourly_limit_tokens: 1000,
                fire_sale_days_before: 3,
                fire_sale_remaining_pct: 0.25,
                fire_sale_discount: 0.2,
                provider_share: 0.7,
            },
        )
        .await
        .unwrap();
    let app = build_router(state, &test_config("unused"));

    let response = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/api/channels")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {alice_token}"))
                .body(Body::from(
                    json!({
                        "name": "alice-pool",
                        "provider": "openai",
                        "base_url": "http://127.0.0.1:9",
                        "api_key_secret": "alice-secret",
                        "models": ["gpt-alice"],
                        "enabled": true,
                        "cycle_limit_tokens": 1000,
                        "cycle_reset_day": 1,
                        "daily_limit_tokens": 1000,
                        "hourly_limit_tokens": 1000,
                        "fire_sale_days_before": 3,
                        "fire_sale_remaining_pct": 0.25,
                        "fire_sale_discount": 0.2,
                        "provider_share": 0.7
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .oneshot(
            axum::http::Request::builder()
                .method("GET")
                .uri("/api/channels")
                .header("authorization", format!("Bearer {alice_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let channels: Value = serde_json::from_slice(&body).unwrap();
    let channels = channels.as_array().unwrap();
    assert_eq!(channels.len(), 1);
    assert_eq!(channels[0]["name"], "alice-pool");
    assert_ne!(channels[0]["id"], bob_channel.id);
    assert!(channels[0].get("api_key_secret").is_none());
}

#[tokio::test]
async fn api_key_management_updates_rotates_and_soft_deletes_keys() {
    let state = setup_state().await;
    let alice = state
        .db
        .create_user("key-owner@example.com", "password123", "KeyOwner")
        .await
        .unwrap();
    let alice_token = state.db.create_session(alice.id).await.unwrap();
    let (gateway_key, record) = state
        .db
        .create_api_key(alice.id, "mutable", Some(100.0))
        .await
        .unwrap();
    let app = build_router(state.clone(), &test_config("unused"));

    let response = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("PATCH")
                .uri(format!("/api/api-keys/{}", record.id))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {alice_token}"))
                .body(Body::from(
                    json!({
                        "name": "prod-agent",
                        "enabled": true,
                        "spend_limit_points": 42,
                        "expires_at": null,
                        "allowed_models": ["gpt-4o*", "claude-3*"]
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
    let updated: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(updated["name"], "prod-agent");
    assert_eq!(updated["allowed_models"][0], "gpt-4o*");

    let response = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri(format!("/api/api-keys/{}/rotate", record.id))
                .header("authorization", format!("Bearer {alice_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let rotated: Value = serde_json::from_slice(&body).unwrap();
    let rotated_key = rotated["token"].as_str().unwrap();
    assert_ne!(rotated_key, gateway_key);
    assert!(
        state
            .db
            .find_api_key(&token_hash(&gateway_key))
            .await
            .is_err()
    );
    assert!(
        state
            .db
            .find_api_key(&token_hash(rotated_key))
            .await
            .is_ok()
    );

    let response = app
        .oneshot(
            axum::http::Request::builder()
                .method("DELETE")
                .uri(format!("/api/api-keys/{}", record.id))
                .header("authorization", format!("Bearer {alice_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        state
            .db
            .find_api_key(&token_hash(rotated_key))
            .await
            .is_err()
    );
    assert!(state.db.list_api_keys(alice.id).await.unwrap().is_empty());
}

#[tokio::test]
async fn channel_management_updates_copies_batches_and_soft_deletes() {
    let state = setup_state().await;
    let alice = state
        .db
        .create_user("channel-owner@example.com", "password123", "ChannelOwner")
        .await
        .unwrap();
    let alice_token = state.db.create_session(alice.id).await.unwrap();
    let channel = state
        .db
        .upsert_channel(
            alice.id,
            ChannelInput {
                name: "editable".to_string(),
                provider: "openai".to_string(),
                base_url: "http://127.0.0.1:9".to_string(),
                api_key_secret: "old-secret".to_string(),
                models: vec!["gpt-old".to_string()],
                enabled: true,
                cycle_limit_tokens: 1000,
                cycle_reset_day: 1,
                daily_limit_tokens: 500,
                hourly_limit_tokens: 100,
                fire_sale_days_before: 3,
                fire_sale_remaining_pct: 0.25,
                fire_sale_discount: 0.2,
                provider_share: 0.7,
            },
        )
        .await
        .unwrap();
    let app = build_router(state.clone(), &test_config("unused"));

    let response = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("PATCH")
                .uri(format!("/api/channels/{}", channel.id))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {alice_token}"))
                .body(Body::from(
                    json!({
                        "name": "editable-renamed",
                        "provider": "anthropic",
                        "base_url": "https://api.anthropic.com",
                        "api_key_secret": "",
                        "models": ["claude-3*"],
                        "enabled": true,
                        "cycle_limit_tokens": 2000,
                        "cycle_reset_day": 2,
                        "daily_limit_tokens": 1000,
                        "hourly_limit_tokens": 100,
                        "fire_sale_days_before": 4,
                        "fire_sale_remaining_pct": 0.5,
                        "fire_sale_discount": 0.3,
                        "provider_share": 0.6
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let updated = state.db.get_channel(channel.id).await.unwrap();
    assert_eq!(updated.name, "editable-renamed");
    assert_eq!(updated.api_key_secret, "old-secret");
    assert_eq!(updated.models, vec!["claude-3*"]);

    let response = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri(format!("/api/channels/{}/copy", channel.id))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {alice_token}"))
                .body(Body::from(
                    json!({"suffix": " clone", "reset_usage": true}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let copied: Value = serde_json::from_slice(&body).unwrap();
    let copied_id = copied["id"].as_i64().unwrap();
    assert_ne!(copied_id, channel.id);

    let response = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/api/channels/batch-enabled")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {alice_token}"))
                .body(Body::from(
                    json!({"ids": [channel.id, copied_id], "enabled": false}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(!state.db.get_channel(channel.id).await.unwrap().enabled);
    assert!(!state.db.get_channel(copied_id).await.unwrap().enabled);

    let response = app
        .oneshot(
            axum::http::Request::builder()
                .method("DELETE")
                .uri(format!("/api/channels/{copied_id}"))
                .header("authorization", format!("Bearer {alice_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(state.db.get_channel(copied_id).await.is_err());
    let visible = state.db.list_public_channels(&alice).await.unwrap();
    assert!(visible.iter().all(|item| item.id != copied_id));
}

async fn setup_state() -> AppState {
    let config = Config {
        bind: "127.0.0.1:0".parse().unwrap(),
        database_url: "sqlite::memory:".to_string(),
        admin_email: None,
        admin_password: None,
        frontend_dist: "frontend/dist".into(),
        leaderboard_timezone: None,
    };
    let state = AppState::new(&config).await.unwrap();
    state
        .db
        .bootstrap_admin("admin@example.com", "password123")
        .await
        .unwrap();
    let admin = state
        .db
        .find_user_with_hash("admin@example.com")
        .await
        .unwrap()
        .unwrap()
        .0;
    state
        .db
        .create_api_key(admin.id, "test", None)
        .await
        .unwrap();
    state
        .db
        .upsert_channel(
            admin.id,
            ChannelInput {
                name: "test".to_string(),
                provider: "openai".to_string(),
                base_url: "http://127.0.0.1:9".to_string(),
                api_key_secret: "test".to_string(),
                models: vec!["*".to_string()],
                enabled: true,
                cycle_limit_tokens: 1000,
                cycle_reset_day: 1,
                daily_limit_tokens: 1000,
                hourly_limit_tokens: 1000,
                fire_sale_days_before: 3,
                fire_sale_remaining_pct: 0.25,
                fire_sale_discount: 0.2,
                provider_share: 0.7,
            },
        )
        .await
        .unwrap();
    state
}

fn token_hash(token: &str) -> String {
    tokenaltar::auth::hash_token(token)
}

fn test_config(database_url: &str) -> Config {
    Config {
        bind: "127.0.0.1:0".parse().unwrap(),
        database_url: database_url.to_string(),
        admin_email: None,
        admin_password: None,
        frontend_dist: "frontend/dist".into(),
        leaderboard_timezone: None,
    }
}
