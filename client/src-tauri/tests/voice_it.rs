//! Needs the dev LiveKit (`just dev-livekit`). Run: `just voice-it`.

use std::process::Command;
use std::time::Duration;

use pulse_client::api::Api;
use pulse_client::voice::devices::AudioConfig;
use pulse_client::voice::{AudioMode, VoiceManager};
use pulse_server::testing;

const DEV_SECRET: &str = "pulse-dev-secret-0123456789abcdefghij";

fn bot_publishes_tone(room: &str) {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../spikes/voice");
    let st = Command::new("docker")
        .args([
            "run",
            "--rm",
            "--network",
            "host",
            "-v",
            &format!("{root}:/w"),
            "livekit/livekit-cli:v2.18",
            "room",
            "join",
            "--url",
            "ws://127.0.0.1:7880",
            "--api-key",
            "devkey",
            "--api-secret",
            DEV_SECRET,
            "--identity",
            "bot",
            "--publish",
            "/w/tone.ogg",
            "--exit-after-publish",
            room,
        ])
        .output()
        .expect("docker");
    assert!(
        st.status.success(),
        "lk bot failed: {}",
        String::from_utf8_lossy(&st.stderr)
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn receives_peer_audio_at_real_time_across_rejoins() {
    let mut cfg = testing::test_config();
    cfg.livekit_url = "ws://127.0.0.1:7880".into();
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

    let vm = VoiceManager::new(std::sync::Arc::new(|_| {}));
    vm.join(
        &api,
        &token,
        lounge.id,
        AudioConfig::default(),
        AudioMode::Null(48_000),
    )
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;

    let room = lounge.id.to_string();
    for round in 1..=3 {
        let before = vm.mixer_pushed().await;
        let r = room.clone();
        tokio::task::spawn_blocking(move || bot_publishes_tone(&r))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        let got = (vm.mixer_pushed().await - before) as f64;
        let expected = 8.0 * 48_000.0; // tone.ogg is 8 s
        let ratio = got / expected;
        println!(
            "round {round}: received {got} samples = {ratio:.2}x real time, rx tasks {}",
            vm.rx_len().await
        );
        // Lower bound is loose (round 1 loses ~1 s to subscription start-up); the upper bound catches the leak.
        assert!(
            (0.7..1.2).contains(&ratio),
            "round {round}: {ratio:.2}x real time (robot-voice leak?)"
        );
        assert!(vm.rx_len().await <= 1);
    }
    vm.leave().await;
}
