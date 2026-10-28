pub mod access;
pub mod auth;
pub mod clips;
pub mod config;
pub mod db;
pub mod dms;
pub mod emoji;
pub mod error;
pub mod gateway;
pub mod hygiene;
pub mod limits;
pub mod media;
pub mod messages;
pub mod qoty;
pub mod quotes;
pub mod reads;
pub mod reports;
pub mod servers;
pub mod state;
pub mod streams;
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
        .merge(auth::two_factor::router())
        .merge(servers::routes::router())
        .merge(messages::routes::router())
        .merge(messages::reactions::router())
        .merge(messages::threads::router())
        .merge(quotes::routes::router())
        .merge(qoty::router())
        .merge(dms::routes::router())
        .merge(voice::routes::router())
        .merge(streams::routes::router())
        .merge(reports::routes::router())
        .merge(media::routes::router())
        .merge(emoji::routes::router())
        .merge(clips::games::router())
        .merge(clips::routes::router())
        .fallback(|| async { AppError::NotFound })
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .with_state(state)
}

/// Bind and serve until the process is stopped.
pub async fn serve(cfg: config::Config) -> anyhow::Result<()> {
    let db = db::connect(&cfg.db_url).await?;
    // Before binding: a wrong or missing key must not bring up a server that locks 2FA users out.
    auth::two_factor::check_key(&db, cfg.totp_key.as_ref()).await?;
    let started_at = db::now();
    let listener = tokio::net::TcpListener::bind(cfg.bind).await?;
    tracing::info!(addr = %cfg.bind, "pulse-app-server listening");
    let hub = gateway::Hub::default();
    // A failure here (e.g. a dev machine without /data) only matters once someone uploads.
    if let Err(e) = std::fs::create_dir_all(&cfg.media_dir) {
        tracing::warn!(
            error = %e,
            dir = %cfg.media_dir.display(),
            "media dir unavailable, so emoji uploads will fail: set PULSE_MEDIA_DIR to a writable directory (`just dev-server` uses data/media)"
        );
    }
    media::spawn_cleanup(db.clone(), cfg.media_dir.clone());
    hygiene::spawn_prune(db.clone());
    let r2 = cfg.clips.as_ref().map(media::r2::R2::new).transpose()?;
    let state = AppState {
        db,
        r2,
        limits: std::sync::Arc::new(limits::Limits::new(&cfg.limits)),
        cfg: std::sync::Arc::new(cfg),
        hub: hub.clone(),
        voice: voice::VoiceState::default(),
        streams: streams::state::StreamState::default(),
    };
    // Before serving (connections queue in the listen backlog), so the first Ready is right.
    voice::restore::on_startup(&state, &started_at).await?;
    // Votes whose time ran out while the server was down end now, before anyone connects.
    // A failure here must not keep the server down: the sweeper tries again shortly.
    if let Err(e) = qoty::end_overdue(&state, &db::now()).await {
        tracing::warn!(error = %e, "Quote of the Year boot sweep failed; the sweeper will retry");
    }
    qoty::spawn_sweeper(state.clone(), qoty::SWEEP_EVERY);
    clips::sweep::spawn(state.clone());
    streams::liveness::spawn(
        state.clone(),
        streams::liveness::CHECK_EVERY,
        state.cfg.stream_media_timeout,
    );
    axum::serve(
        listener,
        router(state).into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        shutdown_signal().await;
        // Upgraded sockets aren't covered by graceful shutdown: tell clients to reconnect.
        hub.close_all(1012);
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    })
    .await?;
    Ok(())
}

/// Ctrl-C (SIGINT) or SIGTERM, which is what `docker stop` sends. As PID 1 in its container the
/// server gets no default action for a signal it doesn't handle, so without this SIGTERM was
/// ignored: Docker waited 10 s, SIGKILLed it, and clients never got the 1012 close.
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut term = signal(SignalKind::terminate()).expect("install SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    let _ = tokio::signal::ctrl_c().await;
}
