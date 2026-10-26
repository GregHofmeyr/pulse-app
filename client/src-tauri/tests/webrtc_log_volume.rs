//! How chatty is libwebrtc in a call? webrtc-sys registers its log sink at LS_VERBOSE, so every
//! RTC_LOG line is formatted in C++ and handed to Rust, where our filter (libwebrtc=warn) drops it.
//! This counts those lines. Needs the dev LiveKit and .env. Run: `just webrtc-log-volume`.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use pulse_client::api::Api;
use pulse_client::voice::devices::AudioConfig;
use pulse_client::voice::{AudioMode, VoiceManager};
use pulse_server::testing;

/// Above this, filtering at the source (a patched webrtc-sys) is worth its upkeep.
const CHATTY_PER_SEC: f64 = 5.0;

static LINES: AtomicU64 = AtomicU64::new(0);

struct CountLibwebrtc;

impl log::Log for CountLibwebrtc {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.target() == "libwebrtc"
    }
    fn log(&self, r: &log::Record) {
        if r.target() == "libwebrtc" {
            LINES.fetch_add(1, Ordering::Relaxed);
        }
    }
    fn flush(&self) {}
}

static LOGGER: CountLibwebrtc = CountLibwebrtc;

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn libwebrtc_log_lines_per_second_in_a_call() {
    log::set_logger(&LOGGER).expect("logger");
    log::set_max_level(log::LevelFilter::Trace);

    // The dev LiveKit from .env (`cp .env.example .env`); `just` loads it (dotenv-load). PULSE_DEV_LIVEKIT_PORT
    // (docker-compose.yml) wins when set: this checkout's LiveKit on alternate ports.
    let mut cfg = testing::test_config();
    cfg.livekit_url = match std::env::var("PULSE_DEV_LIVEKIT_PORT") {
        Ok(port) => format!("ws://127.0.0.1:{port}"),
        Err(_) => std::env::var("PULSE_LIVEKIT_URL").expect("PULSE_LIVEKIT_URL: run via just"),
    };
    cfg.livekit_secret =
        std::env::var("PULSE_LIVEKIT_SECRET").expect("PULSE_LIVEKIT_SECRET: run via just");
    let app = testing::spawn_with(cfg).await;
    let (_, alex) = testing::register(&app, "alex").await;
    let s = testing::create_server(&app, &alex, "Main").await;
    let (_, sam) = testing::register(&app, "sam").await;
    let r = testing::post_json(
        &app,
        &sam,
        &format!("/servers/{}/join", s.id),
        serde_json::json!({}),
    )
    .await;
    assert!(r.status().is_success(), "join server: {}", r.status());
    let api = Api::new(&format!("http://{}", app.addr));
    let lounge = api
        .channels(&alex, s.id)
        .await
        .unwrap()
        .into_iter()
        .find(|c| c.name.as_deref() == Some("Lounge"))
        .unwrap();

    let a = VoiceManager::new(std::sync::Arc::new(|_| {}));
    let b = VoiceManager::new(std::sync::Arc::new(|_| {}));
    for (vm, token) in [(&a, &alex), (&b, &sam)] {
        vm.join(
            &api,
            token,
            lounge.id,
            AudioConfig::default(),
            AudioMode::Null(48_000),
        )
        .await
        .unwrap();
    }
    tokio::time::sleep(Duration::from_secs(5)).await; // connect chatter is not the steady state
    let start = LINES.load(Ordering::Relaxed);
    let secs = 60;
    tokio::time::sleep(Duration::from_secs(secs)).await;
    let per_sec = (LINES.load(Ordering::Relaxed) - start) as f64 / secs as f64;
    println!("libwebrtc: {per_sec:.1} log lines/s in a two-person call");
    a.leave().await;
    b.leave().await;
    assert!(
        per_sec < CHATTY_PER_SEC,
        "{per_sec:.1} lines/s: filter libwebrtc's logging at the source"
    );
}
