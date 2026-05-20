use std::time::Duration;

use axum::{body::Body, http::StatusCode};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tokenaltar::{
    app::{AppState, build_router},
    config::Config,
    db::{ChannelInput, ChannelQuotaWindowInput, LeaderboardPeriod},
    models::{GatewayReservation, LedgerEvent, ModelPrice, Usage},
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
async fn runtime_settings_drive_seed_balances() {
    let state = setup_state().await;
    state
        .db
        .upsert_settings(&[
            tokenaltar::db::SettingUpdate {
                key: "initial_admin_points".to_string(),
                value: "2222".to_string(),
            },
            tokenaltar::db::SettingUpdate {
                key: "initial_user_points".to_string(),
                value: "333".to_string(),
            },
        ])
        .await
        .unwrap();

    state
        .db
        .bootstrap_admin("seeded-admin@example.com", "password123")
        .await
        .unwrap();
    let admin = state
        .db
        .find_user_with_hash("seeded-admin@example.com")
        .await
        .unwrap()
        .unwrap()
        .0;
    let user = state
        .db
        .create_user("seeded-user@example.com", "password123", "Seeded")
        .await
        .unwrap();

    assert_eq!(admin.points_balance, 2222.0);
    assert_eq!(user.points_balance, 333.0);
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
          input_tokens, output_tokens, cache_tokens, input_price_per_1m, output_price_per_1m,
          cache_price_per_1m, surge_multiplier, fire_sale_discount, total_points,
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
          input_tokens, output_tokens, cache_tokens, input_price_per_1m, output_price_per_1m,
          cache_price_per_1m, surge_multiplier, fire_sale_discount, total_points,
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
                windows: quota_windows(1000),
                fire_sale_days_before: 3,
                fire_sale_remaining_pct: 0.25,
                fire_sale_discount: 0.2,
            },
        )
        .await
        .unwrap();
    let app = build_router(state);

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
                        "windows": quota_windows_json(1000),
                        "fire_sale_days_before": 3,
                        "fire_sale_remaining_pct": 0.25,
                        "fire_sale_discount": 0.2
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .clone()
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

    let response = app
        .oneshot(
            axum::http::Request::builder()
                .method("GET")
                .uri("/api/route-channels")
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
    let route_channels: Value = serde_json::from_slice(&body).unwrap();
    let route_channels = route_channels.as_array().unwrap();
    assert!(
        route_channels
            .iter()
            .any(|channel| channel["id"] == bob_channel.id)
    );
    assert!(
        route_channels
            .iter()
            .all(|channel| channel.get("api_key_secret").is_none())
    );
}

#[tokio::test]
async fn admin_manages_users_and_disabled_accounts_cannot_authenticate() {
    let state = setup_state().await;
    let admin = state
        .db
        .find_user_with_hash("admin@example.com")
        .await
        .unwrap()
        .unwrap()
        .0;
    let admin_token = state.db.create_session(admin.id).await.unwrap();
    let app = build_router(state.clone());

    let response = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/api/users")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {admin_token}"))
                .body(Body::from(
                    json!({
                        "email": "managed@example.com",
                        "password": "password123",
                        "role": "user",
                        "display_name": "Managed",
                        "points_balance": 123.5,
                        "enabled": true
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
    let created: Value = serde_json::from_slice(&body).unwrap();
    let managed_id = created["id"].as_i64().unwrap();
    assert_eq!(created["points_balance"], 123.5);

    let managed_session = state.db.create_session(managed_id).await.unwrap();
    let (gateway_key, managed_key_record) = state
        .db
        .create_api_key(managed_id, "managed-key", Some(10.0))
        .await
        .unwrap();
    let channel = state
        .db
        .upsert_channel(
            managed_id,
            ChannelInput {
                name: "managed-channel".to_string(),
                provider: "openai".to_string(),
                base_url: "http://127.0.0.1:9".to_string(),
                api_key_secret: "managed-secret".to_string(),
                models: vec!["*".to_string()],
                enabled: true,
                windows: quota_windows(1000),
                fire_sale_days_before: 3,
                fire_sale_remaining_pct: 0.25,
                fire_sale_discount: 0.2,
            },
        )
        .await
        .unwrap();

    let response = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("PATCH")
                .uri(format!("/api/users/{managed_id}"))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {admin_token}"))
                .body(Body::from(
                    json!({
                        "email": "managed-renamed@example.com",
                        "role": "user",
                        "display_name": "Managed Renamed",
                        "points_balance": 77.25,
                        "enabled": true
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        state.db.get_user(managed_id).await.unwrap().display_name,
        "Managed Renamed"
    );

    let response = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri(format!("/api/users/{managed_id}/password"))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {admin_token}"))
                .body(Body::from(json!({ "password": "newpass123" }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("GET")
                .uri("/api/me")
                .header("authorization", format!("Bearer {managed_session}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let login_response = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/api/auth/login")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "email": "managed-renamed@example.com",
                        "password": "newpass123"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(login_response.status(), StatusCode::OK);

    let response = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri(format!("/api/users/{managed_id}/enabled"))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {admin_token}"))
                .body(Body::from(json!({ "enabled": false }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(!state.db.get_user(managed_id).await.unwrap().enabled);
    assert!(
        !state
            .db
            .get_api_key(managed_key_record.id)
            .await
            .unwrap()
            .enabled
    );
    assert!(!state.db.get_channel(channel.id).await.unwrap().enabled);

    let response = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/api/auth/login")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "email": "managed-renamed@example.com",
                        "password": "newpass123"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(
        state
            .db
            .find_api_key(&token_hash(&gateway_key))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn gateway_reservation_can_be_released_without_leaking_usage() {
    let state = setup_state().await;
    let alice = state
        .db
        .create_user(
            "reserve-release@example.com",
            "password123",
            "ReserveRelease",
        )
        .await
        .unwrap();
    let (_, api_key) = state
        .db
        .create_api_key(alice.id, "reserve", Some(10.0))
        .await
        .unwrap();

    let reservation = state
        .db
        .reserve_gateway_request(alice.id, api_key.id, 1, 25, 2.5)
        .await
        .unwrap();
    assert_eq!(
        state.db.get_user(alice.id).await.unwrap().points_balance,
        997.5
    );
    assert_eq!(
        state.db.get_api_key(api_key.id).await.unwrap().spent_points,
        2.5
    );
    assert_eq!(
        state.db.get_channel(1).await.unwrap().limits.windows[0].used_tokens,
        25
    );

    state
        .db
        .release_gateway_reservation(&reservation)
        .await
        .unwrap();
    assert_eq!(
        state.db.get_user(alice.id).await.unwrap().points_balance,
        1000.0
    );
    assert_eq!(
        state.db.get_api_key(api_key.id).await.unwrap().spent_points,
        0.0
    );
    assert_eq!(
        state.db.get_channel(1).await.unwrap().limits.windows[0].used_tokens,
        0
    );
}

#[tokio::test]
async fn gateway_reservation_enforces_every_quota_window() {
    let state = setup_state().await;
    let owner = state
        .db
        .create_user("strict-window@example.com", "password123", "StrictWindow")
        .await
        .unwrap();
    let (_, api_key) = state
        .db
        .create_api_key(owner.id, "strict", Some(100.0))
        .await
        .unwrap();
    let channel = state
        .db
        .upsert_channel(
            owner.id,
            ChannelInput {
                name: "strict".to_string(),
                provider: "openai".to_string(),
                base_url: "http://127.0.0.1:9".to_string(),
                api_key_secret: "strict-secret".to_string(),
                models: vec!["*".to_string()],
                enabled: true,
                windows: vec![
                    ChannelQuotaWindowInput {
                        name: "Quarter".to_string(),
                        limit_tokens: 1000,
                        period_unit: "month".to_string(),
                        period_count: 3,
                        anchor_at: "2026-01-01T00:00:00".to_string(),
                        timezone: "UTC".to_string(),
                    },
                    ChannelQuotaWindowInput {
                        name: "Minute burst".to_string(),
                        limit_tokens: 5,
                        period_unit: "minute".to_string(),
                        period_count: 15,
                        anchor_at: "2026-05-18T00:00:00".to_string(),
                        timezone: "Asia/Shanghai".to_string(),
                    },
                ],
                fire_sale_days_before: 3,
                fire_sale_remaining_pct: 0.25,
                fire_sale_discount: 0.2,
            },
        )
        .await
        .unwrap();

    let rejected = state
        .db
        .reserve_gateway_request(owner.id, api_key.id, channel.id, 6, 1.0)
        .await;
    assert!(rejected.is_err());
    let after = state.db.get_channel(channel.id).await.unwrap();
    assert_eq!(after.limits.windows[0].used_tokens, 0);
    assert_eq!(after.limits.windows[1].used_tokens, 0);
}

#[tokio::test]
async fn ledger_settlement_applies_only_reservation_delta() {
    let state = setup_state().await;
    let provider = state
        .db
        .find_user_with_hash("admin@example.com")
        .await
        .unwrap()
        .unwrap()
        .0;
    let consumer = state
        .db
        .create_user("reserve-delta@example.com", "password123", "ReserveDelta")
        .await
        .unwrap();
    let (_, api_key) = state
        .db
        .create_api_key(consumer.id, "reserve", Some(10.0))
        .await
        .unwrap();
    let reservation = state
        .db
        .reserve_gateway_request(consumer.id, api_key.id, 1, 10, 1.0)
        .await
        .unwrap();

    state
        .db
        .apply_ledger_event(&LedgerEvent {
            request_id: "req_reservation_delta".to_string(),
            user_id: consumer.id,
            api_key_id: api_key.id,
            channel_id: 1,
            provider_user_id: provider.id,
            model: "gpt-test".to_string(),
            tokenizer: "test".to_string(),
            usage: Usage {
                input_tokens: 12,
                output_tokens: 0,
                cache_tokens: 0,
            },
            price: ModelPrice {
                channel_id: None,
                model_pattern: "default".to_string(),
                input_price_per_1m: 1.0,
                output_price_per_1m: 3.0,
                cache_price_per_1m: 0.2,
            },
            surge_multiplier: 1.0,
            fire_sale_discount: 1.0,
            total_points: 1.5,
            provider_points: 0.6,
            status: "success".to_string(),
            formula_note: "test".to_string(),
            reservation: GatewayReservation {
                user_id: consumer.id,
                api_key_id: api_key.id,
                channel_id: 1,
                points: reservation.points,
                tokens: reservation.tokens,
            },
        })
        .await
        .unwrap();

    assert_eq!(
        state.db.get_user(consumer.id).await.unwrap().points_balance,
        998.5
    );
    assert_eq!(
        state.db.get_user(provider.id).await.unwrap().points_balance,
        1_000_000.6
    );
    assert_eq!(
        state.db.get_api_key(api_key.id).await.unwrap().spent_points,
        1.5
    );
    assert_eq!(
        state.db.get_channel(1).await.unwrap().limits.windows[0].used_tokens,
        12
    );
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
    let app = build_router(state.clone());

    let response = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/api/api-keys")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {alice_token}"))
                .body(Body::from(
                    json!({
                        "name": "staged-client",
                        "enabled": false,
                        "spend_limit_points": 7,
                        "expires_at": null,
                        "allowed_models": ["gpt-4o*"]
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
    let created: Value = serde_json::from_slice(&body).unwrap();
    let created_id = created["record"]["id"].as_i64().unwrap();
    assert_eq!(created["record"]["enabled"], false);
    assert_eq!(created["record"]["allowed_models"][0], "gpt-4o*");
    assert_eq!(
        created["record"]["allowed_channel_ids"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

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
                        "allowed_models": ["gpt-4o*", "claude-3*"],
                        "allowed_channel_ids": [1]
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
        .clone()
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
    assert!(
        state
            .db
            .list_api_keys(alice.id)
            .await
            .unwrap()
            .iter()
            .all(|item| item.id != record.id)
    );

    let response = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/api/api-keys/batch-delete")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {alice_token}"))
                .body(Body::from(json!({ "ids": [created_id] }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(state.db.list_api_keys(alice.id).await.unwrap().is_empty());
}

#[tokio::test]
async fn api_key_channel_selection_controls_route_pool() {
    let state = setup_state().await;
    let consumer = state
        .db
        .create_user("route-key@example.com", "password123", "RouteKey")
        .await
        .unwrap();
    let (token, record) = state
        .db
        .create_api_key(consumer.id, "route-key", Some(100.0))
        .await
        .unwrap();
    let second_channel = state
        .db
        .upsert_channel(
            consumer.id,
            ChannelInput {
                name: "route-selected".to_string(),
                provider: "openai".to_string(),
                base_url: "http://127.0.0.1:9".to_string(),
                api_key_secret: "route-secret".to_string(),
                models: vec!["gpt-selected".to_string()],
                enabled: true,
                windows: quota_windows(1000),
                fire_sale_days_before: 3,
                fire_sale_remaining_pct: 0.25,
                fire_sale_discount: 0.2,
            },
        )
        .await
        .unwrap();
    let consumer_session = state.db.create_session(consumer.id).await.unwrap();
    let app = build_router(state.clone());

    let response = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("PATCH")
                .uri(format!("/api/api-keys/{}", record.id))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {consumer_session}"))
                .body(Body::from(
                    json!({
                        "name": "route-key",
                        "enabled": true,
                        "spend_limit_points": 100,
                        "expires_at": null,
                        "allowed_models": [],
                        "allowed_channel_ids": [second_channel.id]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let updated = state.db.get_api_key(record.id).await.unwrap();
    assert_eq!(updated.allowed_channel_ids, vec![second_channel.id]);

    let disallowed = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/responses")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(
                    json!({ "model": "gpt-test", "input": "hello" }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(disallowed.status(), StatusCode::BAD_REQUEST);
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
                windows: quota_windows(1000),
                fire_sale_days_before: 3,
                fire_sale_remaining_pct: 0.25,
                fire_sale_discount: 0.2,
            },
        )
        .await
        .unwrap();
    let app = build_router(state.clone());

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
                        "windows": quota_windows_json(2000),
                        "fire_sale_days_before": 4,
                        "fire_sale_remaining_pct": 0.5,
                        "fire_sale_discount": 0.3,
                        "provider_share": 0.01
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
    let stored_share: (f64,) =
        sqlx::query_as("SELECT provider_share FROM channel_limits WHERE channel_id = ?")
            .bind(channel.id)
            .fetch_one(&state.db.pool)
            .await
            .unwrap();
    assert_eq!(stored_share.0, 0.7);

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

#[tokio::test]
async fn frontend_assets_are_served_from_embedded_binary() {
    let state = setup_state().await;
    let app = build_router(state);

    let response = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("GET")
                .uri("/")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let html = String::from_utf8(body.to_vec()).unwrap();
    assert!(html.contains(r#"<div id="app"></div>"#));

    let response = app
        .oneshot(
            axum::http::Request::builder()
                .method("GET")
                .uri("/console/channels")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn console_events_push_channel_topic_invalidations() {
    let state = setup_state().await;
    let admin = state
        .db
        .find_user_with_hash("admin@example.com")
        .await
        .unwrap()
        .unwrap()
        .0;
    let session = state.db.create_session(admin.id).await.unwrap();
    let app = build_router(state);

    let response = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("GET")
                .uri("/api/events")
                .header("authorization", format!("Bearer {session}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let mut body = response.into_body();
    let connected = next_sse_chunk(&mut body).await;
    assert!(connected.contains("connected"));

    let response = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/api/channels")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {session}"))
                .body(Body::from(
                    json!({
                        "name": "live",
                        "provider": "openai",
                        "base_url": "http://127.0.0.1:9",
                        "api_key_secret": "live-secret",
                        "models": ["gpt-live"],
                        "enabled": true,
                        "windows": quota_windows_json(1500),
                        "fire_sale_days_before": 3,
                        "fire_sale_remaining_pct": 0.25,
                        "fire_sale_discount": 0.2
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let update = next_sse_chunk(&mut body).await;
    assert!(update.contains("channels"));
    assert!(update.contains("dashboard"));
}

async fn setup_state() -> AppState {
    let config = Config {
        bind: "127.0.0.1:0".parse().unwrap(),
        database_url: "sqlite::memory:".to_string(),
        admin_email: None,
        admin_password: None,
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
                windows: quota_windows(1000),
                fire_sale_days_before: 3,
                fire_sale_remaining_pct: 0.25,
                fire_sale_discount: 0.2,
            },
        )
        .await
        .unwrap();
    state
}

async fn next_sse_chunk(body: &mut Body) -> String {
    let frame = tokio::time::timeout(Duration::from_secs(1), body.frame())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let data = frame.into_data().unwrap();
    String::from_utf8(data.to_vec()).unwrap()
}

fn token_hash(token: &str) -> String {
    tokenaltar::auth::hash_token(token)
}

fn quota_windows(limit_tokens: i64) -> Vec<ChannelQuotaWindowInput> {
    vec![
        ChannelQuotaWindowInput {
            name: "Monthly".to_string(),
            limit_tokens,
            period_unit: "month".to_string(),
            period_count: 1,
            anchor_at: "2026-05-01T00:00:00".to_string(),
            timezone: "UTC".to_string(),
        },
        ChannelQuotaWindowInput {
            name: "Daily".to_string(),
            limit_tokens,
            period_unit: "day".to_string(),
            period_count: 1,
            anchor_at: "2026-05-18T00:00:00".to_string(),
            timezone: "UTC".to_string(),
        },
    ]
}

fn quota_windows_json(limit_tokens: i64) -> Value {
    json!([
        {
            "name": "Monthly",
            "limit_tokens": limit_tokens,
            "period_unit": "month",
            "period_count": 1,
            "anchor_at": "2026-05-01T00:00:00",
            "timezone": "UTC"
        },
        {
            "name": "Daily",
            "limit_tokens": limit_tokens,
            "period_unit": "day",
            "period_count": 1,
            "anchor_at": "2026-05-18T00:00:00",
            "timezone": "UTC"
        }
    ])
}
