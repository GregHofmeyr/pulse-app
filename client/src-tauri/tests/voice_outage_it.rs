//! Needs the dev LiveKit (`just dev-livekit`). Run: `just voice-it`.
//! Its own test binary: it pauses the shared dev LiveKit, which would break any voice test running
//! alongside (cargo runs test binaries one after another).

use std::process::Command;
use std::time::Duration;

use pulse_client::api::Api;
use pulse_client::voice::devices::AudioConfig;
use pulse_client::voice::{AudioMode, VoiceManager};
use pulse_server::testing;

const DEV_SECRET: &str = "pulse-dev-secret-0123456789abcdefghij";

/// The dev LiveKit's address: `PULSE_DEV_LIVEKIT_PORT` when set (`just voice-it-alt`, beside another
/// checkout's LiveKit), else 7880 as in docker/livekit.yaml.
fn dev_livekit(scheme: &str) -> String {
    let port = std::env::var("PULSE_DEV_LIVEKIT_PORT").unwrap_or_else(|_| "7880".into());
    format!("{scheme}://127.0.0.1:{port}")
}

/// Pauses the dev LiveKit (a server that stops answering: no refusal, nothing) until dropped.
struct PausedLiveKit;

impl PausedLiveKit {
    fn compose(cmd: &str) -> bool {
        Command::new("docker")
            .current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
            .args(["compose", cmd, "livekit"])
            .status()
            .is_ok_and(|st| st.success())
    }

    fn new() -> Self {
        assert!(
            Self::compose("pause"),
            "docker compose pause livekit failed"
        );
        Self
    }
}

impl Drop for PausedLiveKit {
    fn drop(&mut self) {
        if !Self::compose("unpause") {
            // a panic here while the test is already failing would abort the run, hiding why
            eprintln!("docker compose unpause livekit failed: run it by hand");
            assert!(
                std::thread::panicking(),
                "docker compose unpause livekit failed"
            );
        }
    }
}

/// R2: against a server that just stops answering, LiveKit's own reconnect can hang for minutes
/// and never report a disconnect. Voice gives up on it (after a minute; 5 s here) and rejoins by
/// itself.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn a_livekit_that_stops_answering_is_rejoined() {
    let mut cfg = testing::test_config();
    cfg.livekit_url = dev_livekit("ws");
    cfg.livekit_secret = DEV_SECRET.into();
    let app = testing::spawn_with(cfg).await;
    let (_, token) = testing::register(&app, "alex").await;
    let s = testing::create_server(&app, &token, "Main").await;
    let api = Api::new(&format!("http://{}", app.addr));
    let lounge = api
        .channels(&token, s.id)
        .await
        .unwrap()
        .into_iter()
        .find(|c| c.name.as_deref() == Some("Lounge"))
        .unwrap();
    let vm = std::sync::Arc::new(
        VoiceManager::new(std::sync::Arc::new(|_| {}))
            .with_reconnect_give_up(Duration::from_secs(5)),
    );
    let runner = {
        let vm = vm.clone();
        tokio::spawn(async move { vm.run_rejoins().await })
    };
    // the rejoin loop must be listening before the join captures it
    tokio::time::sleep(Duration::from_millis(100)).await;
    vm.join(
        &api,
        &token,
        lounge.id,
        AudioConfig::default(),
        AudioMode::Null(48_000),
    )
    .await
    .unwrap();
    let paused = PausedLiveKit::new();
    // LiveKit notices in ~15 s (ping timeout), then we give it 5 s
    tokio::time::timeout(Duration::from_secs(120), async {
        while vm.rejoining().is_none() {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("a reconnect against a silent server was never given up on");
    drop(paused);
    tokio::time::timeout(Duration::from_secs(90), async {
        while vm.current_channel().await != Some(lounge.id) {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("never rejoined");
    assert!(vm.rejoins() >= 1);
    assert!(vm.in_voice());
    vm.leave().await;
    runner.abort();
}
