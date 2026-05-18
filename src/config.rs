use std::{env, net::SocketAddr, path::PathBuf};

#[derive(Clone, Debug)]
pub struct Config {
    pub bind: SocketAddr,
    pub database_url: String,
    pub admin_email: Option<String>,
    pub admin_password: Option<String>,
    pub frontend_dist: PathBuf,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let bind = env::var("TOKENALTAR_BIND")
            .unwrap_or_else(|_| "127.0.0.1:8080".to_string())
            .parse()?;
        let database_url = env::var("TOKENALTAR_DATABASE_URL")
            .unwrap_or_else(|_| "sqlite://tokenaltar.sqlite3".to_string());
        let frontend_dist = env::var("TOKENALTAR_FRONTEND_DIST")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("frontend/dist"));

        Ok(Self {
            bind,
            database_url,
            admin_email: env::var("TOKENALTAR_ADMIN_EMAIL").ok(),
            admin_password: env::var("TOKENALTAR_ADMIN_PASSWORD").ok(),
            frontend_dist,
        })
    }
}
