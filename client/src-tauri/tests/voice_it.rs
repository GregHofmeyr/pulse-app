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

/// Field bug (2026-10-02): rapid leave/rejoin left the client stuck "connecting" and Leave hung behind it.
/// Every join and leave must finish (or fail) within a bounded time, whatever the timing.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn rapid_join_leave_never_hangs() {
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
    let vm = std::sync::Arc::new(VoiceManager::new(std::sync::Arc::new(|_| {})));
    for i in 0..12u64 {
        let pause = Duration::from_millis([0, 50, 300, 1500, 2000][i as usize % 5]);
        let joined = tokio::time::timeout(
            Duration::from_secs(20),
            vm.join(
                &api,
                &token,
                lounge.id,
                AudioConfig::default(),
                AudioMode::Null(48_000),
            ),
        )
        .await;
        assert!(joined.is_ok(), "join #{i} hung");
        tokio::time::sleep(pause).await;
        let left = tokio::time::timeout(Duration::from_secs(10), vm.leave()).await;
        assert!(left.is_ok(), "leave #{i} hung (pause {pause:?})");
    }
    // a leave issued while a join is still in flight must not wait forever either
    let vm2 = vm.clone();
    let (api2, token2, ch) = (api.clone(), token.clone(), lounge.id);
    let joining = tokio::spawn(async move {
        vm2.join(
            &api2,
            &token2,
            ch,
            AudioConfig::default(),
            AudioMode::Null(48_000),
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(
        tokio::time::timeout(Duration::from_secs(10), vm.leave())
            .await
            .is_ok(),
        "leave during join hung"
    );
    let _ = tokio::time::timeout(Duration::from_secs(20), joining)
        .await
        .expect("join never finished");
    assert_eq!(
        vm.current_channel().await,
        None,
        "leave during join must win"
    );
}

/// Field bug (2026-10-12): ~2 s mouth-to-ear on a clean network. The mic runs from the moment a
/// join starts, but audio only goes to LiveKit once the room is connected; whatever piled up in
/// between must not be sent late for the rest of the call.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn mic_audio_is_not_sent_late_after_joining() {
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
    let mut worst = 0;
    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        worst = worst.max(vm.send_queue_ms().await);
    }
    assert!(vm.in_voice());
    let now = vm.send_queue_ms().await;
    println!("send queue after joining: now {now} ms, worst over 2 s {worst} ms");
    assert!(now <= 50, "mic audio waits {now} ms before it is sent");
    assert!(worst <= 50, "mic audio waited up to {worst} ms");
    vm.leave().await;
    assert!(!vm.in_voice(), "left");
}

/// Probe on this machine's real default devices: how much audio piles up in the mic test's playback
/// queue, beyond its deliberate 300 ms delay. Run: `cargo test -p pulse-client --test voice_it mic_test -- --ignored --nocapture`.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn mic_test_queue_on_real_devices() {
    let t = pulse_client::voice::mictest::MicTest::default();
    t.start(&AudioConfig::default(), std::sync::Arc::new(|_| {}))
        .unwrap();
    let mut samples = Vec::new();
    for _ in 0..30 {
        tokio::time::sleep(Duration::from_millis(200)).await;
        samples.push(t.queue_ms());
    }
    t.stop();
    println!("mic test playback queue (ms, every 200 ms): {samples:?}");
}

/// Our own mic path end to end: what one Pulse client sends, another receives at real time
/// (the send side uses WebRTC's unbuffered fast path, paced by our 10 ms frames).
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn one_client_hears_another_at_real_time() {
    let mut cfg = testing::test_config();
    cfg.livekit_url = "ws://127.0.0.1:7880".into();
    cfg.livekit_secret = DEV_SECRET.into();
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
    tokio::time::sleep(Duration::from_secs(2)).await; // subscription start-up
    let before = b.mixer_pushed().await;
    tokio::time::sleep(Duration::from_secs(4)).await;
    let ratio = (b.mixer_pushed().await - before) as f64 / (4.0 * 48_000.0);
    println!("sam receives alex at {ratio:.2}x real time");
    assert!((0.9..1.1).contains(&ratio), "{ratio:.2}x real time");
    a.leave().await;
    b.leave().await;
}

/// On this machine's real devices: a call that nobody touches never reopens them (device
/// following must not churn, e.g. on ALSA's "default" or a name that differs between lists).
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn real_devices_are_not_reopened_without_a_reason() {
    // also shows the voice log lines (join phases, path stats) on a real machine
    let _ = tracing_subscriber::fmt()
        .with_env_filter("info,libwebrtc=warn")
        .try_init();
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
        AudioMode::Real,
    )
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_secs(9)).await; // four device checks
    let n = vm.reopens().await;
    vm.leave().await;
    assert_eq!(n, 0, "reopened {n} times with nothing changed");
}

/// Probe: what looking up a device by name costs on this machine (UI sounds do it per sound).
#[test]
#[ignore]
fn device_lookup_cost() {
    use pulse_client::voice::devices::{device_names, find_device};
    let (_, outs) = device_names();
    let name = outs.last().cloned();
    for _ in 0..3 {
        let t = std::time::Instant::now();
        let _ = find_device(false, name.as_deref());
        println!("find_device({name:?}): {:?}", t.elapsed());
    }
    let t = std::time::Instant::now();
    let _ = find_device(false, None);
    println!("find_device(default): {:?}", t.elapsed());
}

/// On this machine's real devices: reopening the output leaves the mic stream running.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn reopening_one_side_leaves_the_other_alone() {
    use pulse_client::voice::devices::{AudioIo, Shared, Sides};
    let cfg = AudioConfig::default();
    let shared = Shared::new(&cfg, Default::default(), 200);
    let (raw_tx, _raw_rx) = std::sync::mpsc::channel();
    let mut io = AudioIo::start(&cfg, shared, raw_tx).unwrap();
    let (mic, spk) = io.generations();
    io.reopen(
        &cfg,
        Sides {
            input: false,
            output: true,
        },
        || true,
    )
    .unwrap();
    let (mic2, spk2) = io.generations();
    assert_eq!(mic, mic2, "the mic was not touched");
    assert_ne!(spk, spk2, "the speakers were reopened");
    assert!(io.output_rate().is_some());
}
