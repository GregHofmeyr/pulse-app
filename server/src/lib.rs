pub mod access;
pub mod activity;
pub mod attachments;
pub mod auth;
pub mod badges;
pub mod bookmarks;
pub mod calls;
pub mod clips;
pub mod config;
pub mod db;
pub mod deadlock;
pub mod dms;
pub mod embeds;
pub mod emoji;
pub mod error;
pub mod f1;
pub mod forwards;
pub mod gateway;
pub mod gifs;
pub mod hygiene;
pub mod inbox;
pub mod limits;
pub mod media;
pub mod messages;
pub mod models;
pub mod pins;
pub mod plans;
pub mod polls;
pub mod presence;
pub mod profiles;
pub mod qoty;
pub mod quotes;
pub mod reads;
pub mod reminders;
pub mod reports;
pub mod scheduled;
pub mod search;
pub mod servers;
pub mod sounds;
pub mod sports;
pub mod stage;
pub mod state;
pub mod steam;
pub mod streams;
pub mod updates;
pub mod voice;
pub mod watch;
pub mod wheels;

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
        .merge(servers::nicknames::router())
        .merge(servers::order::router())
        .merge(servers::categories::router())
        .merge(messages::routes::router())
        .merge(messages::reactions::router())
        .merge(messages::threads::router())
        .merge(inbox::router())
        .merge(pins::router())
        .merge(polls::router())
        .merge(plans::router())
        .merge(scheduled::router())
        .merge(reminders::router())
        .merge(bookmarks::router())
        .merge(forwards::router())
        .merge(f1::router())
        .merge(gifs::router())
        .merge(quotes::routes::router())
        .merge(qoty::router())
        .merge(dms::routes::router())
        .merge(dms::layout::router())
        .merge(voice::routes::router())
        .merge(calls::routes::router())
        .merge(streams::routes::router())
        .merge(streams::notify::router())
        .merge(stage::routes::router())
        .merge(watch::routes::router())
        .merge(reports::routes::router())
        .merge(media::routes::router())
        .merge(emoji::routes::router())
        .merge(sounds::routes::router())
        .merge(clips::games::router())
        .merge(clips::routes::router())
        .merge(attachments::routes::router())
        .merge(profiles::routes::router())
        .merge(profiles::private::router())
        .merge(presence::routes::router())
        .merge(badges::routes::router())
        .merge(search::routes::router())
        .merge(embeds::routes::router())
        .merge(embeds::page::router())
        .merge(updates::router())
        .merge(models::router())
        .merge(wheels::routes::router())
        .merge(steam::routes::router())
        .merge(deadlock::routes::router())
        .merge(sports::routes::router())
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
    // The bound address, not the configured one: with port 0 (the tests) it names the real port.
    tracing::info!(addr = %listener.local_addr()?, "pulse-app-server listening");
    let hub = gateway::Hub::default();
    // A failure here (e.g. a dev machine without /data) only matters once someone uploads.
    if let Err(e) = std::fs::create_dir_all(&cfg.media_dir) {
        tracing::warn!(
            error = %e,
            dir = %cfg.media_dir.display(),
            "media dir unavailable, so emoji uploads will fail: set PULSE_MEDIA_DIR to a writable directory (`just dev-server` uses data/media)"
        );
    }
    // App updates (#82): `just publish-update` fills it; until then the route answers 404 ("no update").
    if let Err(e) = std::fs::create_dir_all(&cfg.updates_dir) {
        tracing::warn!(
            error = %e,
            dir = %cfg.updates_dir.display(),
            "updates dir unavailable, so app updates can't be served: set PULSE_UPDATES_DIR to a writable directory"
        );
    }
    media::spawn_cleanup(db.clone(), cfg.media_dir.clone());
    hygiene::spawn_prune(db.clone());
    let r2 = cfg.clips.as_ref().map(media::r2::R2::new).transpose()?;
    let state = AppState {
        db,
        r2,
        limits: std::sync::Arc::new(limits::Limits::new(&cfg.limits)),
        embeds: std::sync::Arc::new(embeds::Unfurler::new(&cfg.embeds)),
        steam: steam::Steam::new(&cfg.steam),
        deadlock: deadlock::Deadlock::new(&cfg.deadlock),
        sports: sports::Sports::new(&cfg.sports),
        cfg: std::sync::Arc::new(cfg),
        hub: hub.clone(),
        voice: voice::VoiceState::default(),
        streams: streams::state::StreamState::default(),
    };
    // Before serving (connections queue in the listen backlog), so the first Ready is right.
    voice::restore::on_startup(&state, &started_at).await?;
    // Nothing is live yet (streams live in memory): close what a previous run left open.
    if let Err(e) = badges::streams::reconcile_on_startup(&state.db).await {
        tracing::warn!(error = %e, "couldn't close stream sessions left open");
    }
    // Votes whose time ran out while the server was down end now, before anyone connects.
    // A failure here must not keep the server down: the sweeper tries again shortly.
    if let Err(e) = qoty::end_overdue(&state, &db::now()).await {
        tracing::warn!(error = %e, "Quote of the Year boot sweep failed; the sweeper will retry");
    }
    qoty::spawn_sweeper(state.clone(), qoty::SWEEP_EVERY);
    clips::sweep::spawn(state.clone());
    attachments::sweep::spawn(state.clone());
    pins::spawn_sweeper(state.clone());
    polls::spawn_sweeper(state.clone());
    plans::spawn_sweeper(state.clone());
    scheduled::spawn_sweeper(state.clone());
    reminders::spawn_sweeper(state.clone());
    // F1 race weekends (#386): nothing is fetched while no server has an F1 channel
    f1::worker::spawn(state.clone());
    badges::sweep::spawn(state.clone());
    servers::channels::spawn_sweeper(state.clone());
    servers::deleted::spawn_sweeper(state.clone());
    dms::purge::spawn_sweeper(state.clone());
    presence::sweep::spawn_sweeper(state.clone());
    dms::sweep::spawn_sweeper(state.clone());
    // Steam achievements (#379): the poller, and any backfill a restart cut off.
    steam::poller::spawn(state.clone());
    steam::backfill::kick(&state);
    // Deadlock (#390): match cards, ranks, rank movers and patch notes from deadlock-api.com
    deadlock::poller::spawn(state.clone());
    // Sports score pings (#385): never started without an API-Sports key
    sports::poller::spawn(state.clone());
    // badges whose Steam tiers changed reach those whose stored games already earn them
    {
        let s = state.clone();
        tokio::spawn(async move { steam::store::recompute_all(&s).await });
    }
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
