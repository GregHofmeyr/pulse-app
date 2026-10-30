//! Needs the dev LiveKit (`just dev-livekit`). Run: `just voice-it`.

use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pulse_client::api::Api;
use pulse_client::voice::devices::AudioConfig;
use pulse_client::voice::{AudioMode, Connection, VoiceEvent, VoiceManager};
use pulse_protocol::rest::ChannelKind;
use pulse_server::testing;

mod common;
use common::livekit_turn;

const DEV_SECRET: &str = "pulse-dev-secret-0123456789abcdefghij";

/// The dev LiveKit's address: `PULSE_DEV_LIVEKIT_PORT` when set (`just voice-it-alt`, beside another
/// checkout's LiveKit), else 7880 as in docker/livekit.yaml.
fn dev_livekit(scheme: &str) -> String {
    let port = std::env::var("PULSE_DEV_LIVEKIT_PORT").unwrap_or_else(|_| "7880".into());
    format!("{scheme}://127.0.0.1:{port}")
}

/// Polls `check` until it holds or `within` passes; whether it held. For start-up that takes
/// longer on a loaded machine (subscriptions, publishes), so the tests wait for the event
/// itself instead of a fixed time.
#[must_use]
async fn eventually<F, Fut>(within: Duration, mut check: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = tokio::time::Instant::now() + within;
    loop {
        if check().await {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Publishes spikes/voice/tone.ogg (8 s) as `bot` in `room` and returns once it has all been sent.
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
            &dev_livekit("ws"),
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
    let stderr = String::from_utf8_lossy(&st.stderr);
    // livekit-cli (v2.18, and upstream still) can panic on its way out: with `--exit-after-publish`
    // it closes its `done` channel when the file is done, then the disconnect its own leave causes
    // closes it again ("close of closed channel"). That leave (CLIENT_INITIATED) only comes after
    // the publish finished; a leave for any other reason, or none, is still a failure.
    let exit_race = stderr.contains("published track")
        && stderr.contains("server initiated leave\t{\"reason\": \"CLIENT_INITIATED\"}")
        && stderr.contains("panic: close of closed channel");
    assert!(st.status.success() || exit_race, "lk bot failed: {stderr}");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn receives_peer_audio_at_real_time_across_rejoins() {
    let _turn = livekit_turn().await;
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
        // the last of the tone drains from the jitter buffer: wait until nothing more arrives
        let mut last = vm.mixer_pushed().await;
        for _ in 0..15 {
            tokio::time::sleep(Duration::from_millis(200)).await;
            let now = vm.mixer_pushed().await;
            if std::mem::replace(&mut last, now) == now {
                break;
            }
        }
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
    let _turn = livekit_turn().await;
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
    let _turn = livekit_turn().await;
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
    let _turn = livekit_turn().await;
    let mut cfg = testing::test_config();
    cfg.livekit_url = dev_livekit("ws");
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
    // subscription start-up: sam's receive task is running and alex's frames reach the mixer
    assert!(
        eventually(Duration::from_secs(15), || async {
            b.rx_len().await == 1 && b.mixer_pushed().await > 0
        })
        .await,
        "sam never received alex (rx tasks {})",
        b.rx_len().await
    );
    // past the jitter buffer's start-up, so the window measures the steady state
    tokio::time::sleep(Duration::from_secs(1)).await;
    let (before, t) = (b.mixer_pushed().await, std::time::Instant::now());
    tokio::time::sleep(Duration::from_secs(4)).await;
    // over the time that really passed: a loaded machine oversleeps
    let ratio = (b.mixer_pushed().await - before) as f64 / (t.elapsed().as_secs_f64() * 48_000.0);
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
    let _turn = livekit_turn().await;
    // also shows the voice log lines (join phases, path stats) on a real machine
    let _ = tracing_subscriber::fmt()
        .with_env_filter("info,libwebrtc=warn")
        .try_init();
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

/// Leave while the devices are still opening, after the room already connected: the room must
/// be closed, not dropped (a dropped livekit Room stays connected: a ghost others still see).
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn leaving_while_devices_open_leaves_no_ghost() {
    let _turn = livekit_turn().await;
    let mut cfg = testing::test_config();
    cfg.livekit_url = dev_livekit("ws");
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

    // sam watches the room directly
    let vt = api.voice_token(&sam, lounge.id).await.unwrap();
    let (watcher, _events) = livekit::Room::connect(&vt.url, &vt.token, Default::default())
        .await
        .unwrap();
    let others = || watcher.remote_participants().len();
    let wait_for =
        |want: usize, within: Duration| eventually(within, move || async move { others() == want });

    let vm = std::sync::Arc::new(VoiceManager::new(std::sync::Arc::new(|_| {})));
    let (vm2, api2, lounge_id) = (vm.clone(), api.clone(), lounge.id);
    let joining = tokio::spawn(async move {
        vm2.join(
            &api2,
            &alex,
            lounge_id,
            AudioConfig::default(),
            // devices still opening long after the room connected, even on a loaded machine
            AudioMode::SlowNull(48_000, Duration::from_secs(15)),
        )
        .await
    });
    // well inside the devices' 15 s, so the leave below lands while they are still opening
    assert!(
        wait_for(1, Duration::from_secs(10)).await,
        "alex's room never connected"
    );
    vm.leave().await;
    assert!(joining.await.unwrap().is_err(), "the join was cancelled");
    assert!(
        wait_for(0, Duration::from_secs(15)).await,
        "alex is still in the room: a ghost"
    );
    watcher.close().await.unwrap();
}

/// A TCP proxy in front of the dev LiveKit that lets each signal connection's first message
/// (LiveKit's JoinResponse: the join is accepted) through, then holds the rest of what LiveKit
/// says (the SDP answer the peer connection needs) until `release`. A join through it stops
/// inside `Room::connect` after LiveKit accepted it, for as long as the test likes.
struct HoldAfterJoin {
    addr: std::net::SocketAddr,
    release: tokio::sync::watch::Sender<bool>,
}

impl HoldAfterJoin {
    async fn start() -> Self {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (release, released) = tokio::sync::watch::channel(false);
        let upstream = dev_livekit("ws").replace("ws://", "");
        tokio::spawn(async move {
            while let Ok((client, _)) = listener.accept().await {
                let (upstream, mut released) = (upstream.clone(), released.clone());
                tokio::spawn(async move {
                    let server = tokio::net::TcpStream::connect(&upstream).await.unwrap();
                    let (mut cr, mut cw) = client.into_split();
                    let (mut sr, mut sw) = server.into_split();
                    tokio::spawn(async move {
                        let _ = tokio::io::copy(&mut cr, &mut sw).await;
                        let _ = sw.shutdown().await;
                    });
                    // LiveKit's HTTP response head, then (an upgrade) its first frame
                    let mut head = Vec::new();
                    while !head.ends_with(b"\r\n\r\n") {
                        let Ok(b) = sr.read_u8().await else { return };
                        head.push(b);
                    }
                    if cw.write_all(&head).await.is_err() {
                        return;
                    }
                    if head.starts_with(b"HTTP/1.1 101") {
                        let mut frame = vec![0u8; 2];
                        if sr.read_exact(&mut frame).await.is_err() {
                            return;
                        }
                        let ext = match frame[1] & 0x7f {
                            126 => 2,
                            127 => 8,
                            _ => 0,
                        };
                        let mask = if frame[1] & 0x80 != 0 { 4 } else { 0 };
                        let mut more = vec![0u8; ext];
                        if sr.read_exact(&mut more).await.is_err() {
                            return;
                        }
                        let len = match ext {
                            0 => (frame[1] & 0x7f) as usize,
                            _ => more.iter().fold(0usize, |n, b| n << 8 | *b as usize),
                        };
                        frame.extend_from_slice(&more);
                        let mut rest = vec![0u8; mask + len];
                        if sr.read_exact(&mut rest).await.is_err() {
                            return;
                        }
                        frame.extend_from_slice(&rest);
                        if cw.write_all(&frame).await.is_err() {
                            return;
                        }
                        let _ = released.wait_for(|r| *r).await;
                    }
                    let _ = tokio::io::copy(&mut sr, &mut cw).await;
                });
            }
        });
        Self { addr, release }
    }

    fn url(&self) -> String {
        format!("ws://{}", self.addr)
    }

    fn release(&self) {
        let _ = self.release.send(true);
    }
}

/// Leave while `Room::connect` itself is still running, after LiveKit accepted the join (it
/// lists the participant) but before the peer connection is up. The join must still end with
/// LiveKit told: a dropped connect left the participant listed until LiveKit timed it out
/// (~15 s), and one that came up meanwhile would have been a ghost everyone saw.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn leaving_mid_connect_leaves_no_ghost() {
    let _turn = livekit_turn().await;
    let hold = HoldAfterJoin::start().await;
    let mut cfg = testing::test_config();
    // the server hands out the proxy's address as LiveKit's
    cfg.livekit_url = hold.url();
    cfg.livekit_secret = DEV_SECRET.into();
    let app = testing::spawn_with(cfg).await;
    let (_, alex) = testing::register(&app, "alex").await;
    let s = testing::create_server(&app, &alex, "Main").await;
    let (sam_id, sam) = testing::register(&app, "sam").await;
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
    let room_name = lounge.id.to_string();
    let sam_identity = sam_id.to_string();

    // LiveKit's own view of the room
    let lk = livekit_api::services::room::RoomClient::with_api_key(
        &dev_livekit("http"),
        "devkey",
        DEV_SECRET,
    );
    let sam_listed = || {
        let (lk, room, who) = (&lk, room_name.clone(), sam_identity.clone());
        async move {
            lk.list_participants(&room)
                .await
                .expect("list participants")
                .iter()
                .any(|p| p.identity == who)
        }
    };

    // alex is in the room, straight to LiveKit (a plain LiveKit client: who the others see)
    let vt = api.voice_token(&alex, lounge.id).await.unwrap();
    let (alex_room, _events) =
        livekit::Room::connect(&dev_livekit("ws"), &vt.token, Default::default())
            .await
            .unwrap();
    let alex_sees_sam = || {
        alex_room
            .remote_participants()
            .keys()
            .any(|id| id.as_str() == sam_identity)
    };

    let vm = Arc::new(VoiceManager::new(Arc::new(|_| {})));
    let (vm2, api2, lounge_id) = (vm.clone(), api.clone(), lounge.id);
    let joining = tokio::spawn(async move {
        vm2.join(
            &api2,
            &sam,
            lounge_id,
            AudioConfig::default(),
            AudioMode::Null(48_000),
        )
        .await
    });
    assert!(
        eventually(Duration::from_secs(10), sam_listed).await,
        "LiveKit never accepted sam's join"
    );
    assert!(!joining.is_finished(), "sam's join got past Room::connect");
    tokio::time::timeout(Duration::from_secs(10), vm.leave())
        .await
        .expect("leave waited for the connect");
    assert!(joining.await.unwrap().is_err(), "the join was cancelled");
    // LiveKit's answer reaches sam now: what's left of the cancelled join may finish connecting
    hold.release();

    // well inside LiveKit's own timeouts for a participant that went quiet (~15-20 s)
    assert!(
        eventually(Duration::from_secs(2), || async { !sam_listed().await }).await,
        "LiveKit still lists sam 2 s after the leave: a ghost"
    );
    assert!(
        eventually(Duration::from_secs(2), || async { !alex_sees_sam() }).await,
        "alex still sees sam 2 s after the leave: a ghost"
    );
    alex_room.close().await.unwrap();
}

/// "Connected" (and the join sound) must not wait a round trip for the mic publish.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn connected_shows_before_the_mic_is_published() {
    let _turn = livekit_turn().await;
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
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let sink = seen.clone();
    let vm = VoiceManager::new(Arc::new(move |e| {
        let tag = match e {
            VoiceEvent::State {
                connection: Connection::Connected,
                ..
            } => "connected".to_string(),
            VoiceEvent::Mic { live, .. } => format!("mic:{live}"),
            _ => return,
        };
        sink.lock().unwrap().push(tag);
    }));
    vm.join(
        &api,
        &token,
        lounge.id,
        AudioConfig::default(),
        AudioMode::Null(48_000),
    )
    .await
    .unwrap();
    // the publish is a round trip after "connected": slower on a loaded machine, so wait for it
    assert!(
        eventually(Duration::from_secs(15), || async {
            seen.lock().unwrap().iter().any(|t| t == "mic:true")
        })
        .await,
        "mic never published: {:?}",
        seen.lock().unwrap()
    );
    let seen = seen.lock().unwrap().clone();
    let connected = seen
        .iter()
        .position(|t| t == "connected")
        .expect("connected");
    let live = seen
        .iter()
        .position(|t| t == "mic:true")
        .expect("mic published");
    assert!(connected < live, "{seen:?}");
    vm.leave().await;
}

/// The join reports its stages in order, with "connected" (state) between starting audio and the
/// final connected stage.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn stages_arrive_in_order() {
    let _turn = livekit_turn().await;
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
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let sink = seen.clone();
    let vm = VoiceManager::new(Arc::new(move |e| {
        let tag = match e {
            VoiceEvent::Stage { stage, .. } => format!(
                "stage:{}",
                serde_json::to_value(stage).unwrap().as_str().unwrap()
            ),
            VoiceEvent::State {
                connection: Connection::Connected,
                ..
            } => "state:connected".to_string(),
            _ => return,
        };
        sink.lock().unwrap().push(tag);
    }));
    vm.join(
        &api,
        &token,
        lounge.id,
        AudioConfig::default(),
        AudioMode::Null(48_000),
    )
    .await
    .unwrap();
    assert!(
        eventually(Duration::from_secs(15), || async {
            seen.lock().unwrap().iter().any(|t| t == "stage:connected")
        })
        .await,
        "stage:connected never arrived: {:?}",
        seen.lock().unwrap()
    );
    let seen = seen.lock().unwrap().clone();
    assert_eq!(
        seen,
        [
            "stage:requesting_access",
            "stage:connecting",
            "stage:starting_audio",
            "state:connected",
            "stage:connected"
        ],
        "{seen:?}"
    );
    vm.leave().await;
}

/// A switch goes straight to the new room: no Disconnected flash (leave sound) between rooms,
/// and no waiting for the old room's close.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn switching_channels_goes_straight_to_the_new_room() {
    let _turn = livekit_turn().await;
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
    let den = api
        .create_channel(&token, s.id, ChannelKind::Voice, "Den")
        .await
        .unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    let vm = VoiceManager::new(Arc::new(move |e| {
        if let VoiceEvent::State {
            channel_id,
            connection,
            ..
        } = e
        {
            sink.lock().unwrap().push((channel_id, connection));
        }
    }));
    vm.join(
        &api,
        &token,
        lounge.id,
        AudioConfig::default(),
        AudioMode::Null(48_000),
    )
    .await
    .unwrap();
    seen.lock().unwrap().clear();
    let t = std::time::Instant::now();
    vm.join(
        &api,
        &token,
        den.id,
        AudioConfig::default(),
        AudioMode::Null(48_000),
    )
    .await
    .unwrap();
    println!("switch took {:?}", t.elapsed());
    tokio::time::sleep(Duration::from_secs(1)).await; // the old room's close lands meanwhile
    let states = seen.lock().unwrap().clone();
    assert_eq!(
        states.first(),
        Some(&(Some(den.id), Connection::Connecting)),
        "{states:?}"
    );
    assert!(
        !states.iter().any(|(_, c)| *c == Connection::Disconnected),
        "Disconnected between rooms: {states:?}"
    );
    assert_eq!(vm.current_channel().await, Some(den.id));
    vm.leave().await;
}

/// A switch whose join fails ends Disconnected (not stuck "connecting", not back in A), and
/// the old room's close never pulls us back into it (the rejoin loop runs).
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn a_failed_switch_ends_disconnected_and_the_old_room_stays_left() {
    let _turn = livekit_turn().await;
    let mut cfg = testing::test_config();
    cfg.livekit_url = dev_livekit("ws");
    cfg.livekit_secret = DEV_SECRET.into();
    let app = testing::spawn_with(cfg).await;
    let (_, token) = testing::register(&app, "alex").await;
    let s = testing::create_server(&app, &token, "Main").await;
    let text = testing::general(&app, &token, s.id).await; // no voice: its token request fails
    let api = Api::new(&format!("http://{}", app.addr));
    let lounge = api
        .channels(&token, s.id)
        .await
        .unwrap()
        .into_iter()
        .find(|c| c.name.as_deref() == Some("Lounge"))
        .unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    let vm = Arc::new(VoiceManager::new(Arc::new(move |e| {
        if let VoiceEvent::State {
            channel_id,
            connection,
            ..
        } = e
        {
            sink.lock().unwrap().push((channel_id, connection));
        }
    })));
    let rejoins = tokio::spawn({
        let vm = vm.clone();
        async move { vm.run_rejoins().await }
    });
    vm.join(
        &api,
        &token,
        lounge.id,
        AudioConfig::default(),
        AudioMode::Null(48_000),
    )
    .await
    .unwrap();
    seen.lock().unwrap().clear();
    let r = vm
        .join(
            &api,
            &token,
            text.id,
            AudioConfig::default(),
            AudioMode::Null(48_000),
        )
        .await;
    assert!(r.is_err());
    tokio::time::sleep(Duration::from_secs(3)).await; // A's close (and any rejoin) would land by now
    let states = seen.lock().unwrap().clone();
    assert_eq!(
        states.last(),
        Some(&(None, Connection::Disconnected)),
        "{states:?}"
    );
    assert!(
        !states.iter().any(|(c, _)| *c == Some(lounge.id)),
        "pulled back into the old room: {states:?}"
    );
    assert!(!vm.in_voice());
    assert_eq!(vm.current_channel().await, None);
    assert_eq!(vm.rejoins(), 0, "the old room's close started a rejoin");
    rejoins.abort();
}

/// A switch keeps the devices, the processor and Strong's model; Leave lets them go.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn switching_keeps_the_audio_pipeline_and_leave_drops_it() {
    let _turn = livekit_turn().await;
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
    let den = api
        .create_channel(&token, s.id, ChannelKind::Voice, "Den")
        .await
        .unwrap();
    let vm = VoiceManager::new(Arc::new(|_| {}));
    let join = |id| {
        vm.join(
            &api,
            &token,
            id,
            AudioConfig::default(),
            AudioMode::Null(48_000),
        )
    };
    join(lounge.id).await.unwrap();
    assert_eq!(vm.pipelines_built(), 1);
    join(den.id).await.unwrap();
    assert_eq!(vm.pipelines_built(), 1, "the switch rebuilt the pipeline");
    // the kept mixer plays the new room's people
    let before = vm.mixer_pushed().await;
    let room = den.id.to_string();
    tokio::task::spawn_blocking(move || bot_publishes_tone(&room))
        .await
        .unwrap();
    assert!(
        vm.mixer_pushed().await > before,
        "not mixing in the new room"
    );
    vm.leave().await;
    join(lounge.id).await.unwrap();
    assert_eq!(vm.pipelines_built(), 2, "after Leave a join starts fresh");
    vm.leave().await;
}

/// One person publishing a mic AND stream audio: the listener plays both, and unpublishing the
/// stream audio leaves the mic playing (screen-share spec §2, step zero).
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn two_tracks_from_one_person_both_play() {
    use livekit::options::TrackPublishOptions;
    use livekit::prelude::*;
    use livekit::webrtc::audio_frame::AudioFrame;
    use livekit::webrtc::audio_source::native::NativeAudioSource;
    use livekit::webrtc::audio_source::{AudioSourceOptions, RtcAudioSource};

    let _turn = livekit_turn().await;
    let mut cfg = testing::test_config();
    cfg.livekit_url = dev_livekit("ws");
    cfg.livekit_secret = DEV_SECRET.into();
    let app = testing::spawn_with(cfg).await;
    let (_, alex) = testing::register(&app, "alex").await;
    let (_, sam) = testing::register(&app, "sam").await;
    let s = testing::create_server(&app, &alex, "Main").await;
    let api = Api::new(&format!("http://{}", app.addr));
    api.join_server(&sam, s.id).await.unwrap();
    let lounge = api
        .channels(&alex, s.id)
        .await
        .unwrap()
        .into_iter()
        .find(|c| c.name.as_deref() == Some("Lounge"))
        .unwrap();

    // alex listens through Pulse's voice pipeline
    let vm = VoiceManager::new(std::sync::Arc::new(|_| {}));
    vm.join(
        &api,
        &alex,
        lounge.id,
        AudioConfig::default(),
        AudioMode::Null(48_000),
    )
    .await
    .unwrap();

    // sam is a bare SDK client publishing two tracks. Voice tickets may only publish `Microphone`
    // (stream audio lives in the stream room now), so both are mic-sourced: this checks the mixer, not the source.
    let vt = api.voice_token(&sam, lounge.id).await.unwrap();
    let (room, _events) = Room::connect(&vt.url, &vt.token, RoomOptions::default())
        .await
        .unwrap();
    let publish = |name: &str, source: TrackSource| {
        let src = NativeAudioSource::new(AudioSourceOptions::default(), 48_000, 1, 100);
        let track = LocalAudioTrack::create_audio_track(name, RtcAudioSource::Native(src.clone()));
        (
            src,
            track,
            TrackPublishOptions {
                source,
                ..Default::default()
            },
        )
    };
    let (mic_src, mic, mic_opts) = publish("mic", TrackSource::Microphone);
    let (game_src, game, game_opts) = publish("game", TrackSource::Microphone);
    let lp = room.local_participant();
    lp.publish_track(LocalTrack::Audio(mic), mic_opts)
        .await
        .unwrap();
    let game_pub = lp
        .publish_track(LocalTrack::Audio(game), game_opts)
        .await
        .unwrap();

    // feed both with a tone at real time until the end of the test (aborted there)
    let feed = |src: NativeAudioSource| {
        tokio::spawn(async move {
            let tone: Vec<i16> = (0..480)
                .map(|i| ((i as f32 * 0.13).sin() * 8000.0) as i16)
                .collect();
            loop {
                let frame = AudioFrame {
                    data: tone.clone().into(),
                    sample_rate: 48_000,
                    num_channels: 1,
                    samples_per_channel: 480,
                };
                src.capture_frame(&frame).await.unwrap();
            }
        })
    };
    let (a, b) = (feed(mic_src), feed(game_src));
    // subscribing to both takes a moment (longer on a loaded machine)
    assert!(
        eventually(Duration::from_secs(15), || async {
            vm.rx_len().await >= 2 && vm.mixer_sources().await >= 2
        })
        .await,
        "both tracks never arrived: rx tasks {}, mixer sources {}",
        vm.rx_len().await,
        vm.mixer_sources().await
    );
    // and stay one each: a late duplicate task or source would show up here
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(vm.rx_len().await, 2, "one receive task per track");
    assert_eq!(vm.mixer_sources().await, 2, "both tracks in the mixer");

    lp.unpublish_track(&game_pub.sid()).await.unwrap();
    // the unsubscribe reaches alex a moment later
    assert!(
        eventually(Duration::from_secs(15), || async {
            vm.rx_len().await <= 1 && vm.mixer_sources().await <= 1
        })
        .await,
        "the unpublished track was never dropped: rx tasks {}, mixer sources {}",
        vm.rx_len().await,
        vm.mixer_sources().await
    );
    assert_eq!(vm.rx_len().await, 1, "the mic task survives");
    assert_eq!(vm.mixer_sources().await, 1, "the mic source survives");
    let before = vm.mixer_pushed().await;
    // Frames alone could be WebRTC's silence: the tone itself must still be heard. A single 250 ms
    // window is not a reliable "heard" (measured: even one lone mic track gives an all-zero window
    // now and then, so it is not about the second track), so look over up to 10 s for any window with the tone.
    vm.mixer_levels().await; // start a fresh level window
    let mut heard = Vec::new();
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        heard.extend(vm.mixer_levels().await);
        if heard.iter().any(|(_, l)| *l > 0.01) {
            break;
        }
    }
    assert!(vm.mixer_pushed().await > before, "the mic is still playing");
    assert!(
        heard.iter().any(|(_, l)| *l > 0.01),
        "the mic's tone is still heard: {heard:?}"
    );

    a.abort();
    b.abort();
    room.close().await.unwrap();
    vm.leave().await;
}

/// A mic that delivers only digital silence while unmuted (the null devices send zeros) raises the
/// "no sound from your mic" warning, through the real call loop; muting clears it.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn a_silent_mic_is_reported_and_muting_clears_it() {
    let _turn = livekit_turn().await;
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

    let seen = Arc::new(Mutex::new(Vec::<bool>::new()));
    let log = seen.clone();
    let vm = VoiceManager::new(Arc::new(move |e| {
        if let VoiceEvent::MicSilent { silent } = e {
            log.lock().unwrap().push(silent);
        }
    }));
    vm.join(
        &api,
        &token,
        lounge.id,
        AudioConfig::default(),
        AudioMode::Null(48_000),
    )
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert!(
        seen.lock().unwrap().is_empty(),
        "not before 10 s of silence"
    );
    let warned = eventually(Duration::from_secs(10), || {
        let seen = seen.clone();
        async move { seen.lock().unwrap().last() == Some(&true) }
    })
    .await;
    assert!(warned, "silent mic reported: {:?}", seen.lock().unwrap());
    vm.toggle_mute().await;
    let cleared = eventually(Duration::from_secs(2), || {
        let seen = seen.clone();
        async move { seen.lock().unwrap().last() == Some(&false) }
    })
    .await;
    assert!(cleared, "muting clears it: {:?}", seen.lock().unwrap());
    vm.leave().await;
}
