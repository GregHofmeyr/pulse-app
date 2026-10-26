//! R5 probe: is NetEq's "buffer" (jitterBufferDelay / emitted) a real standing delay? Measures the
//! actual onset-to-arrival lag of the bare LiveKit path (no Pulse pipeline) between two
//! participants in this process and prints it next to the path-line numbers.
//! Needs the dev LiveKit (`just dev-livekit`, host networking on lo).
//! Run: `cargo test -p pulse-client --test neteq_probe -- --ignored --nocapture`
//! With delay: `sudo tc qdisc add dev lo root netem delay 40ms 1ms` first and
//! `NETEM_ONEWAY_MS=40` (the qdisc's delay) in the environment; `sudo tc qdisc del dev lo root` after.
//! Both participants reach the SFU over lo, so the audio crosses the qdisc twice: the probe
//! subtracts 2 × NETEM_ONEWAY_MS from the measured lag.
//! `PROBE_PATTERNS=bursts,continuous` picks the patterns and their order (default
//! `continuous,bursts`; also `gated` and `noise`). Each test run is a fresh room and stream; the
//! patterns follow each other on it with no break in the frames.
//! `PROBE_STALL_MS=500` stops sending frames for that long before every pattern after the first:
//! a send stall with no silence fill (what `GapFill` prevents in the app).

use std::time::{Duration, Instant};

use futures::StreamExt;
use livekit::prelude::*;
use livekit::webrtc::audio_frame::AudioFrame;
use livekit::webrtc::audio_source::native::NativeAudioSource;
use livekit::webrtc::audio_source::{AudioSourceOptions, RtcAudioSource};
use livekit::webrtc::audio_stream::native::NativeAudioStream;
use pulse_client::api::Api;
use pulse_client::voice::{mic_publish_options, netstats};
use pulse_server::testing;
use tokio::sync::mpsc;

const DEV_SECRET: &str = "pulse-dev-secret-0123456789abcdefghij";

/// The dev LiveKit's address: `PULSE_DEV_LIVEKIT_PORT` when set (`just voice-it-alt`, beside another
/// checkout's LiveKit), else 7880 as in docker/livekit.yaml.
fn dev_livekit(scheme: &str) -> String {
    let port = std::env::var("PULSE_DEV_LIVEKIT_PORT").unwrap_or_else(|_| "7880".into());
    format!("{scheme}://127.0.0.1:{port}")
}
const RATE: u32 = 48_000;
const FRAME: usize = 480;
const SECONDS: u64 = 30;
/// Onset to arrival that isn't the jitter buffer: ~5 ms waiting for the 20 ms Opus packet to
/// fill, ~5 ms for the receiver's 10 ms pull. ±5 ms; the question is a 50+ ms gap.
const PATH_FIXED_MS: f64 = 10.0;
/// Received 10 ms frames louder than this count as "on".
const ON_RMS: f64 = 0.05;

#[derive(Clone, Copy, Debug)]
enum Pattern {
    /// 0.8 s loud tone, 0.2 s quiet tone (−20 dB, not zero: no DTX). One onset per second.
    Continuous,
    /// 1 s loud tone, 1 s exact zeros: DTX engages in every gap, like our gate.
    Bursts,
    /// Like the real gate on real speech: talk 1-2 s and gaps 0.3-2 s (seeded, irregular), each
    /// talk spurt fading out over the gate's 100 ms before exact zeros.
    Gated,
    /// Bursts with −60 dBFS white noise in the gaps instead of exact zeros: does DTX still engage?
    Noise,
}

/// What frame `k` (10 ms) carries: tone amplitude and white-noise amplitude.
#[derive(Clone, Copy)]
struct Frame {
    tone: f32,
    noise: f32,
}

/// The gate's fade-out (`voice/gate.rs` FADE_BLOCKS: 10 blocks of 10 ms).
const GATE_FADE_FRAMES: u64 = 10;
/// −60 dBFS.
const NOISE_AMP: f32 = 0.001;

/// Small deterministic generator (the probe must be reproducible run to run).
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }

    /// Uniform in `lo..=hi`.
    fn range(&mut self, lo: u64, hi: u64) -> u64 {
        lo + self.next() % (hi - lo + 1)
    }

    /// Uniform in −1..1.
    fn unit(&mut self) -> f32 {
        (self.next() % 65_536) as f32 / 32_768.0 - 1.0
    }
}

impl Pattern {
    fn parse(name: &str) -> Self {
        match name.trim() {
            "continuous" => Pattern::Continuous,
            "bursts" => Pattern::Bursts,
            "gated" => Pattern::Gated,
            "noise" => Pattern::Noise,
            other => panic!("unknown pattern {other:?}"),
        }
    }

    /// Every 10 ms frame of the run.
    fn schedule(self) -> Vec<Frame> {
        let n = (SECONDS * 100) as usize;
        let tone = |tone| Frame { tone, noise: 0.0 };
        match self {
            Pattern::Continuous => (0..n)
                .map(|k| tone(if k % 100 < 20 { 0.03 } else { 0.3 }))
                .collect(),
            Pattern::Bursts => (0..n)
                .map(|k| tone(if k % 200 < 100 { 0.3 } else { 0.0 }))
                .collect(),
            Pattern::Noise => (0..n)
                .map(|k| {
                    if k % 200 < 100 {
                        tone(0.3)
                    } else {
                        Frame {
                            tone: 0.0,
                            noise: NOISE_AMP,
                        }
                    }
                })
                .collect(),
            Pattern::Gated => {
                // talk spurts ≥ 1 s and gaps ≥ 0.3 s keep onsets ≥ 1.3 s apart, so an arrival
                // still pairs with its own onset at lags up to that
                let mut rng = Lcg(0x5EED);
                let mut out = Vec::with_capacity(n);
                while out.len() < n {
                    out.extend((0..rng.range(100, 200)).map(|_| tone(0.3)));
                    out.extend(
                        (0..GATE_FADE_FRAMES)
                            .map(|i| tone(0.3 * (1.0 - (i + 1) as f32 / GATE_FADE_FRAMES as f32))),
                    );
                    out.extend((0..rng.range(30, 200)).map(|_| tone(0.0)));
                }
                out.truncate(n);
                out
            }
        }
    }
}

fn onset(schedule: &[Frame], k: usize) -> bool {
    k > 0 && schedule[k].tone > 0.1 && schedule[k - 1].tone < 0.1
}

/// Exact zeros after each pattern while its last arrivals are in flight. The frames must not stop:
/// RTP timestamps count samples, so a gap with no frames makes every later packet look that much
/// late to NetEq, which raises its target by the gap (520 ms after the 500 ms wait this replaced).
const TAIL_FRAMES: usize = 50;

async fn send(source: NativeAudioSource, pattern: Pattern, onsets: mpsc::UnboundedSender<Instant>) {
    let mut schedule = pattern.schedule();
    schedule.extend((0..TAIL_FRAMES).map(|_| Frame {
        tone: 0.0,
        noise: 0.0,
    }));
    let mut rng = Lcg(1);
    let mut tick = tokio::time::interval(Duration::from_millis(10));
    let step = 2.0 * std::f32::consts::PI * 440.0 / RATE as f32;
    let mut phase = 0f32;
    for (k, f) in schedule.iter().enumerate() {
        tick.tick().await;
        let data: Vec<i16> = (0..FRAME)
            .map(|_| {
                phase = (phase + step) % (2.0 * std::f32::consts::PI);
                let v = phase.sin() * f.tone + rng.unit() * f.noise;
                (v * 32767.0) as i16
            })
            .collect();
        if onset(&schedule, k) {
            let _ = onsets.send(Instant::now());
        }
        let frame = AudioFrame {
            data: data.into(),
            sample_rate: RATE,
            num_channels: 1,
            samples_per_channel: FRAME as u32,
        };
        let _ = source.capture_frame(&frame).await;
    }
}

fn percentile(sorted: &[f64], p: f64) -> Option<f64> {
    (!sorted.is_empty()).then(|| sorted[((sorted.len() - 1) as f64 * p).round() as usize])
}

fn median(mut v: Vec<f64>) -> Option<f64> {
    v.sort_by(f64::total_cmp);
    percentile(&v, 0.5)
}

async fn run(
    pattern: Pattern,
    source: &NativeAudioSource,
    sender: &Room,
    receiver: &Room,
    arrivals: &mut mpsc::UnboundedReceiver<Instant>,
    stall: Duration,
) {
    tokio::time::sleep(stall).await; // no frames at all (PROBE_STALL_MS)
    while arrivals.try_recv().is_ok() {} // leftovers from the previous pattern
    let (sent_tx, mut sent_rx) = mpsc::unbounded_channel();
    let sending = tokio::spawn(send(source.clone(), pattern, sent_tx));
    let mut totals = netstats::Totals::default();
    let mut sent_totals = netstats::Totals::default();
    let mut windows = Vec::new();
    let mut send_pps = Vec::new();
    let mut tick = tokio::time::interval(netstats::WINDOW);
    tick.tick().await;
    for _ in 0..SECONDS / 5 {
        tick.tick().await;
        if let Ok(s) = receiver.get_stats().await {
            let (p, t) = netstats::summarize(&s.publisher_stats, &s.subscriber_stats, &totals);
            totals = t;
            windows.push(p);
        }
        // the sender's packets/s: ~50 while sending, far fewer once DTX engages
        if let Ok(s) = sender.get_stats().await {
            let (p, t) = netstats::summarize(&s.publisher_stats, &[], &sent_totals);
            sent_totals = t;
            send_pps.extend(p.send_pps);
        }
    }
    sending.await.unwrap(); // its tail covered the last arrivals in flight
    let sent: Vec<Instant> = std::iter::from_fn(|| sent_rx.try_recv().ok()).collect();
    let got: Vec<Instant> = std::iter::from_fn(|| arrivals.try_recv().ok()).collect();
    let mut lags: Vec<f64> = got
        .iter()
        .filter_map(|g| {
            sent.iter()
                .rev()
                .find(|s| **s <= *g)
                .map(|s| g.duration_since(*s).as_secs_f64() * 1000.0)
        })
        .filter(|ms| *ms < 900.0)
        .collect();
    lags.sort_by(f64::total_cmp);
    let netem: f64 = std::env::var("NETEM_ONEWAY_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.0);
    // the first window includes the subscription start-up: leave it out
    let steady = || windows.iter().skip(1);
    let col = |f: fn(&netstats::PathStats) -> Option<f64>| {
        median(steady().filter_map(f).collect()).map_or("-".into(), |v| format!("{v:.0}"))
    };
    let lag50 = percentile(&lags, 0.5);
    println!(
        "| {pattern:?} | {netem:.0} | {} | {} | {} | {} | {} | {} | {} | {} | {} |",
        lag50.map_or("-".into(), |v| format!("{v:.0}")),
        percentile(&lags, 0.9).map_or("-".into(), |v| format!("{v:.0}")),
        lag50.map_or("-".into(), |v| format!(
            "{:.0}",
            v - 2.0 * netem - PATH_FIXED_MS
        )),
        col(|p| p.buffer_ms),
        col(|p| p.target_ms),
        col(|p| p.accel_ms),
        col(|p| p.silent_pct),
        median(send_pps).map_or("-".into(), |v| format!("{v:.0}")),
        lags.len(),
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn neteq_buffer_vs_measured_lag() {
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
    let connect = |token: String| {
        let api = api.clone();
        async move {
            let vt = api.voice_token(&token, lounge.id).await.unwrap();
            Room::connect(&vt.url, &vt.token, RoomOptions::default())
                .await
                .unwrap()
        }
    };
    let (sender, _sender_events) = connect(alex.clone()).await;
    let (receiver, mut events) = connect(sam.clone()).await;

    let source = NativeAudioSource::new(AudioSourceOptions::default(), RATE, 1, 0);
    let track = LocalAudioTrack::create_audio_track("mic", RtcAudioSource::Native(source.clone()));
    sender
        .local_participant()
        .publish_track(LocalTrack::Audio(track), mic_publish_options())
        .await
        .unwrap();

    // the receiver's onsets, timed as frames come out of NetEq
    let (arrival_tx, mut arrivals) = mpsc::unbounded_channel::<Instant>();
    tokio::spawn(async move {
        while let Some(ev) = events.recv().await {
            if let RoomEvent::TrackSubscribed {
                track: RemoteTrack::Audio(t),
                ..
            } = ev
            {
                let tx = arrival_tx.clone();
                tokio::spawn(async move {
                    let mut stream = NativeAudioStream::new(t.rtc_track(), RATE as i32, 1);
                    let mut on = false;
                    while let Some(f) = stream.next().await {
                        let n = f.data.len().max(1) as f64;
                        let rms = (f.data.iter().map(|&s| (s as f64).powi(2)).sum::<f64>() / n)
                            .sqrt()
                            / 32768.0;
                        let now_on = rms > ON_RMS;
                        if now_on && !on {
                            let _ = tx.send(Instant::now());
                        }
                        on = now_on;
                    }
                });
            }
        }
    });
    tokio::time::sleep(Duration::from_secs(2)).await; // subscription start-up

    println!(
        "| pattern | netem qdisc ms | lag p50 | lag p90 | jb from lag | buffer | target | accel ms/5s | silent % | send pkt/s | onsets |"
    );
    println!("|---|---|---|---|---|---|---|---|---|---|---|");
    let patterns = std::env::var("PROBE_PATTERNS").unwrap_or("continuous,bursts".into());
    let stall = Duration::from_millis(
        std::env::var("PROBE_STALL_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0),
    );
    for (i, pattern) in patterns.split(',').map(Pattern::parse).enumerate() {
        let stall = if i == 0 { Duration::ZERO } else { stall };
        run(pattern, &source, &sender, &receiver, &mut arrivals, stall).await;
    }
    let _ = sender.close().await;
    let _ = receiver.close().await;
}
