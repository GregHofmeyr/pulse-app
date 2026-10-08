pub mod access;
pub mod auth;
pub mod config;
pub mod db;
pub mod dms;
pub mod error;
pub mod gateway;
pub mod messages;
pub mod reads;
pub mod servers;
pub mod state;
pub mod voice;

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
        .merge(dms::routes::router())
        .merge(voice::routes::router())
        .fallback(|| async { AppError::NotFound })
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .with_state(state)
}

/// Bind and serve until the process is stopped.
pub async fn serve(cfg: config::Config) -> anyhow::Result<()> {
    let db = db::connect(&cfg.db_url).await?;
    voice::reconcile_on_startup(&db).await?;
    let listener = tokio::net::TcpListener::bind(cfg.bind).await?;
    tracing::info!(addr = %cfg.bind, "pulse-app-server listening");
    let hub = gateway::Hub::default();
    let state = AppState {
        db,
        cfg: std::sync::Arc::new(cfg),
        hub: hub.clone(),
        voice: voice::VoiceState::default(),
    };
    axum::serve(listener, router(state))
        .with_graceful_shutdown(async move {
            let _ = tokio::signal::ctrl_c().await;
            // Upgraded sockets aren't covered by graceful shutdown: tell clients to reconnect.
            hub.close_all(1012);
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        })
        .await?;
    Ok(())
}
