use tokenaltar::{app, config::Config};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "tokenaltar=info,tower_http=info".into()),
        )
        .init();

    let config = Config::from_env()?;
    let state = app::AppState::new(&config).await?;
    let router = app::build_router(state, &config);
    let listener = tokio::net::TcpListener::bind(config.bind).await?;
    tracing::info!("TokenAltar listening on http://{}", config.bind);
    axum::serve(listener, router).await?;
    Ok(())
}
