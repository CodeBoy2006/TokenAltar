use tokenaltar::{app::AppState, config::Config, db::ChannelInput};

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
    let points = state.db.claim_red_packet(claimer.id, "RustIsBest").await.unwrap();
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
    state.db.set_anonymous_leaderboard(user.id, true).await.unwrap();
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

    let leaderboards = state.db.leaderboards().await.unwrap();
    assert!(leaderboards["providers"][0]["name"]
        .as_str()
        .unwrap()
        .starts_with("Anonymous #"));
    assert!(leaderboards["providers"][0]["user_id"].is_null());
}

async fn setup_state() -> AppState {
    let config = Config {
        bind: "127.0.0.1:0".parse().unwrap(),
        database_url: "sqlite::memory:".to_string(),
        admin_email: None,
        admin_password: None,
        frontend_dist: "frontend/dist".into(),
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
    state.db.create_api_key(admin.id, "test", None).await.unwrap();
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
