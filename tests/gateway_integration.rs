use std::{net::SocketAddr, time::Duration};

use axum::{Json, Router, http::StatusCode, routing::post};
use serde_json::{Value, json};
use tokenaltar::{
    app::{AppState, build_router},
    config::Config,
    db::ChannelInput,
    models::ModelPrice,
};
use tower::ServiceExt;

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
    let app = build_router(state.clone(), &test_config("unused"));

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
    assert_eq!(channel.limits.used_cycle_tokens, 16);
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
    let app = build_router(state, &test_config("unused"));

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
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
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
    let app = build_router(state.clone(), &test_config("unused"));

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
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
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
    sqlx::query("UPDATE channel_limits SET used_cycle_tokens = cycle_limit_tokens WHERE channel_id = 1")
        .execute(&state.db.pool)
        .await
        .unwrap();
    state.db.refresh_channel_windows().await.unwrap();
    let app = build_router(state, &test_config("unused"));
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
            input_price_per_1k: 1.0,
            output_price_per_1k: 1.0,
            cache_price_per_1k: 0.0,
        })
        .await
        .unwrap();
    state
        .db
        .upsert_price(&ModelPrice {
            channel_id: Some(1),
            model_pattern: "gpt-special".to_string(),
            input_price_per_1k: 10.0,
            output_price_per_1k: 20.0,
            cache_price_per_1k: 0.0,
        })
        .await
        .unwrap();
    let app = build_router(state.clone(), &test_config("unused"));

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
        "SELECT input_price_per_1k, output_price_per_1k FROM ledger_entries WHERE model = 'gpt-special'",
    )
    .fetch_one(&state.db.pool)
    .await
    .unwrap();
    assert_eq!(row.0, 10.0);
    assert_eq!(row.1, 20.0);
}

async fn setup_state(upstream: String) -> (AppState, String) {
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
                provider: "openai".to_string(),
                base_url: upstream,
                api_key_secret: "upstream-key".to_string(),
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
    (state, token)
}

fn test_config(database_url: &str) -> Config {
    Config {
        bind: "127.0.0.1:0".parse().unwrap(),
        database_url: database_url.to_string(),
        admin_email: None,
        admin_password: None,
        frontend_dist: "frontend/dist".into(),
    }
}

async fn spawn_upstream(body: Value) -> String {
    async fn handler(Json(body): Json<Value>) -> Json<Value> {
        Json(body)
    }
    let app = Router::new()
        .route("/v1/responses", post({
            let body = body.clone();
            move || async move { Json(body.clone()) }
        }))
        .route("/v1/messages", post(handler));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}
