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
    sqlx::query(
        "UPDATE channel_limits SET used_cycle_tokens = cycle_limit_tokens WHERE channel_id = 1",
    )
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

#[tokio::test]
async fn openai_responses_to_openai_channel_is_passthrough() {
    let upstream = spawn_echo_upstream().await;
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
    let app = build_router(state.clone(), &test_config("unused"));

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
    let app = build_router(state.clone(), &test_config("unused"));

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
