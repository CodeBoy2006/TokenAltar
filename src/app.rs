use std::time::Duration;

use axum::{
    Router,
    body::Body,
    http::{HeaderName, HeaderValue, Method, StatusCode, Uri, header},
    response::Response,
    routing::{get, post},
};
use reqwest::Client;
use rust_embed::Embed;
use tower_http::{
    cors::{Any, CorsLayer},
    trace::TraceLayer,
};

use crate::{
    affinity::AffinityCache,
    billing::{LedgerSender, spawn_ledger_worker},
    config::Config,
    db::Database,
    events::ConsoleEventBus,
    gateway,
    routing::RuntimeRouterState,
    state::MetricsState,
};

#[derive(Embed)]
#[folder = "frontend/dist/"]
struct FrontendAssets;

#[derive(Clone)]
pub struct AppState {
    pub db: Database,
    pub http: Client,
    pub affinity_cache: AffinityCache,
    pub router_state: RuntimeRouterState,
    pub metrics: MetricsState,
    pub ledger_tx: LedgerSender,
    pub leaderboard_timezone: Option<String>,
    pub events: ConsoleEventBus,
}

impl AppState {
    pub async fn new(config: &Config) -> anyhow::Result<Self> {
        let db = Database::connect(&config.database_url).await?;
        if let (Some(email), Some(password)) = (&config.admin_email, &config.admin_password) {
            db.bootstrap_admin(email, password).await?;
        }
        db.refresh_channel_windows().await?;
        let settings = db.runtime_settings().await?;
        let metrics = MetricsState::default();
        let events = ConsoleEventBus::default();
        let ledger_tx = spawn_ledger_worker(
            db.clone(),
            metrics.clone(),
            events.clone(),
            settings.ledger_queue_capacity,
        );
        let http = Client::builder()
            .timeout(Duration::from_secs(120))
            .pool_idle_timeout(Duration::from_secs(30))
            .build()?;
        Ok(Self {
            db,
            http,
            affinity_cache: AffinityCache::new(settings.affinity_cache_capacity),
            router_state: RuntimeRouterState::default(),
            metrics,
            ledger_tx,
            leaderboard_timezone: config.leaderboard_timezone.clone(),
            events,
        })
    }
}

pub fn build_router(state: AppState) -> Router {
    let cors = CorsLayer::new()
        .allow_methods([Method::GET, Method::POST, Method::PATCH, Method::DELETE])
        .allow_headers(Any)
        .allow_origin(Any);

    let api = Router::new()
        .route("/auth/register", post(crate::admin::register))
        .route("/auth/login", post(crate::admin::login))
        .route("/me", get(crate::admin::me))
        .route("/events", get(crate::events::console_events))
        .route(
            "/users",
            get(crate::admin::list_users).post(crate::admin::create_user),
        )
        .route(
            "/users/{id}",
            axum::routing::patch(crate::admin::update_user),
        )
        .route("/users/{id}/enabled", post(crate::admin::set_user_enabled))
        .route(
            "/users/{id}/password",
            post(crate::admin::reset_user_password),
        )
        .route(
            "/api-keys",
            get(crate::admin::list_api_keys).post(crate::admin::create_api_key),
        )
        .route(
            "/api-keys/batch-delete",
            post(crate::admin::batch_delete_api_keys),
        )
        .route(
            "/api-keys/{id}/enabled",
            post(crate::admin::set_api_key_enabled),
        )
        .route(
            "/api-keys/{id}",
            axum::routing::patch(crate::admin::update_api_key).delete(crate::admin::delete_api_key),
        )
        .route("/api-keys/{id}/rotate", post(crate::admin::rotate_api_key))
        .route(
            "/route-channels",
            get(crate::admin::list_route_channel_options),
        )
        .route(
            "/channels",
            get(crate::admin::list_channels).post(crate::admin::create_channel),
        )
        .route(
            "/channels/batch-enabled",
            post(crate::admin::batch_set_channels_enabled),
        )
        .route(
            "/channels/{id}",
            axum::routing::patch(crate::admin::update_channel).delete(crate::admin::delete_channel),
        )
        .route(
            "/channels/{id}/enabled",
            post(crate::admin::set_channel_enabled),
        )
        .route("/channels/{id}/copy", post(crate::admin::copy_channel))
        .route("/channels/{id}/test", post(crate::admin::test_channel))
        .route(
            "/prices",
            get(crate::admin::list_prices).post(crate::admin::upsert_price),
        )
        .route(
            "/affinity-rules",
            get(crate::admin::list_affinity_rules).post(crate::admin::create_affinity_rule),
        )
        .route("/ledger", get(crate::admin::list_ledger))
        .route("/dashboard", get(crate::admin::dashboard))
        .route("/runtime-settings", get(crate::admin::runtime_settings))
        .route(
            "/settings",
            get(crate::admin::get_settings).post(crate::admin::update_settings),
        )
        .route(
            "/profile/anonymous-leaderboard",
            post(crate::admin::set_anonymous_leaderboard),
        )
        .route(
            "/transfers",
            get(crate::admin::list_transfers).post(crate::admin::transfer_points),
        )
        .route(
            "/red-packets",
            get(crate::admin::list_red_packets).post(crate::admin::create_red_packet),
        )
        .route("/red-packets/claim", post(crate::admin::claim_red_packet))
        .route("/leaderboards", get(crate::admin::leaderboards));

    Router::new()
        .nest("/api", api)
        .route(
            "/v1/chat/completions",
            post(gateway::openai_chat_completions),
        )
        .route("/v1/responses", post(gateway::openai_responses))
        .route("/v1/messages", post(gateway::anthropic_messages))
        .route(
            "/v1beta/models/{model_action}",
            post(gateway::gemini_generate_content),
        )
        .fallback(embedded_frontend)
        .with_state(state)
        .layer(cors)
        .layer(TraceLayer::new_for_http())
}

async fn embedded_frontend(uri: Uri) -> Response {
    let path = normalize_asset_path(uri.path());
    if let Some(response) = embedded_asset_response(&path, StatusCode::OK) {
        return response;
    }
    if looks_like_static_asset(&path) {
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from("not found"))
            .expect("static 404 response is valid");
    }
    embedded_asset_response("index.html", StatusCode::OK).unwrap_or_else(|| {
        Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(Body::from("embedded frontend index.html is missing"))
            .expect("static 500 response is valid")
    })
}

fn normalize_asset_path(path: &str) -> String {
    let trimmed = path.trim_start_matches('/');
    if trimmed.is_empty() {
        return "index.html".to_string();
    }
    if trimmed
        .split('/')
        .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return "index.html".to_string();
    }
    trimmed.to_string()
}

fn embedded_asset_response(path: &str, status: StatusCode) -> Option<Response> {
    let asset = FrontendAssets::get(path)?;
    let mut response = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, content_type(path))
        .body(Body::from(asset.data.into_owned()))
        .expect("embedded asset response is valid");
    if is_cacheable_asset(path) {
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=31536000, immutable"),
        );
    }
    Some(response)
}

fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or_default() {
        "css" => "text/css; charset=utf-8",
        "html" => "text/html; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "json" => "application/json",
        "png" => "image/png",
        "svg" => "image/svg+xml",
        "txt" => "text/plain; charset=utf-8",
        "wasm" => "application/wasm",
        "webp" => "image/webp",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    }
}

fn looks_like_static_asset(path: &str) -> bool {
    path.rsplit('/')
        .next()
        .is_some_and(|name| name.contains('.'))
}

fn is_cacheable_asset(path: &str) -> bool {
    path != "index.html"
}

pub fn copy_passthrough_headers(headers: &axum::http::HeaderMap) -> axum::http::HeaderMap {
    let mut outbound = axum::http::HeaderMap::new();
    for name in ["user-agent", "x-request-id"] {
        if let (Ok(header_name), Some(value)) =
            (HeaderName::from_bytes(name.as_bytes()), headers.get(name))
        {
            outbound.insert(header_name, value.clone());
        }
    }
    outbound.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    outbound
}
