pub mod access;
pub mod auth;
pub mod config;
pub mod db;
pub mod error;
pub mod gateway;
pub mod messages;
pub mod servers;
pub mod state;

#[cfg(feature = "testing")]
pub mod testing;

use axum::Router;
use axum::routing::get;

use crate::error::AppError;
pub use crate::state::AppState;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/gateway", get(gateway::socket::handler))
        .merge(auth::routes::router())
        .merge(servers::routes::router())
        .merge(messages::routes::router())
        .fallback(|| async { AppError::NotFound })
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .with_state(state)
}

/// Bind and serve until the process is stopped.
pub async fn serve(cfg: config::Config) -> anyhow::Result<()> {
    let db = db::connect(&cfg.db_url).await?;
    let listener = tokio::net::TcpListener::bind(cfg.bind).await?;
    tracing::info!(addr = %cfg.bind, "pulse-app-server listening");
    let state = AppState {
        db,
        cfg: std::sync::Arc::new(cfg),
        hub: gateway::Hub::default(),
    };
    axum::serve(listener, router(state)).await?;
    Ok(())
}
