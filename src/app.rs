use std::time::Duration;

use axum::{
    Router,
    http::{HeaderName, HeaderValue, Method, header},
    routing::{get, post},
};
use reqwest::Client;
use tower_http::{
    cors::{Any, CorsLayer},
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};

use crate::{
    affinity::AffinityCache,
    billing::{LedgerSender, spawn_ledger_worker},
    config::Config,
    db::Database,
    gateway,
    routing::RuntimeRouterState,
    state::MetricsState,
};

#[derive(Clone)]
pub struct AppState {
    pub db: Database,
    pub http: Client,
    pub affinity_cache: AffinityCache,
    pub router_state: RuntimeRouterState,
    pub metrics: MetricsState,
    pub ledger_tx: LedgerSender,
}

impl AppState {
    pub async fn new(config: &Config) -> anyhow::Result<Self> {
        let db = Database::connect(&config.database_url).await?;
        if let (Some(email), Some(password)) = (&config.admin_email, &config.admin_password) {
            db.bootstrap_admin(email, password).await?;
        }
        db.refresh_channel_windows().await?;
        let metrics = MetricsState::default();
        let ledger_tx = spawn_ledger_worker(db.clone(), metrics.clone());
        let http = Client::builder()
            .timeout(Duration::from_secs(120))
            .pool_idle_timeout(Duration::from_secs(30))
            .build()?;
        Ok(Self {
            db,
            http,
            affinity_cache: AffinityCache::new(4096),
            router_state: RuntimeRouterState::default(),
            metrics,
            ledger_tx,
        })
    }
}

pub fn build_router(state: AppState, config: &Config) -> Router {
    let cors = CorsLayer::new()
        .allow_methods([Method::GET, Method::POST, Method::PATCH, Method::DELETE])
        .allow_headers(Any)
        .allow_origin(Any);

    let api = Router::new()
        .route("/auth/register", post(crate::admin::register))
        .route("/auth/login", post(crate::admin::login))
        .route("/me", get(crate::admin::me))
        .route("/api-keys", get(crate::admin::list_api_keys).post(crate::admin::create_api_key))
        .route("/api-keys/{id}/enabled", post(crate::admin::set_api_key_enabled))
        .route("/channels", get(crate::admin::list_channels).post(crate::admin::create_channel))
        .route("/prices", get(crate::admin::list_prices).post(crate::admin::upsert_price))
        .route("/affinity-rules", get(crate::admin::list_affinity_rules).post(crate::admin::create_affinity_rule))
        .route("/ledger", get(crate::admin::list_ledger))
        .route("/dashboard", get(crate::admin::dashboard))
        .route("/settings", get(crate::admin::get_settings).post(crate::admin::update_settings))
        .route("/profile/anonymous-leaderboard", post(crate::admin::set_anonymous_leaderboard))
        .route("/transfers", get(crate::admin::list_transfers).post(crate::admin::transfer_points))
        .route("/red-packets", get(crate::admin::list_red_packets).post(crate::admin::create_red_packet))
        .route("/red-packets/claim", post(crate::admin::claim_red_packet))
        .route("/leaderboards", get(crate::admin::leaderboards));

    let static_service = ServeDir::new(&config.frontend_dist)
        .not_found_service(ServeFile::new(config.frontend_dist.join("index.html")));

    Router::new()
        .nest("/api", api)
        .route("/v1/chat/completions", post(gateway::openai_chat_completions))
        .route("/v1/responses", post(gateway::openai_responses))
        .route("/v1/messages", post(gateway::anthropic_messages))
        .fallback_service(static_service)
        .with_state(state)
        .layer(cors)
        .layer(TraceLayer::new_for_http())
}

pub fn copy_passthrough_headers(headers: &axum::http::HeaderMap) -> axum::http::HeaderMap {
    let mut outbound = axum::http::HeaderMap::new();
    for name in ["user-agent", "x-request-id"] {
        if let (Ok(header_name), Some(value)) = (HeaderName::from_bytes(name.as_bytes()), headers.get(name)) {
            outbound.insert(header_name, value.clone());
        }
    }
    outbound.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    outbound
}
