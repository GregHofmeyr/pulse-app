//! What the mic really sends at each voice-channel bitrate (#75), measured against the dev LiveKit
//! (`just dev-livekit`, or the alternate ports `just voice-it-alt` uses). Run:
//! `cargo test -p pulse-client --test voice_bitrate_it -- --ignored --nocapture`.
//!
//! The mic is published exactly as the app publishes it (`mic_publish_options`, DTX and RED on)
//! and fed full-scale-ish noise, the hardest thing for Opus to compress, so the encoder spends all
//! the bitrate it is allowed (speech, with DTX and Opus' VBR, spends less). The rate is the
//! sender's own `outbound-rtp` count (payload bytes; RED's redundant copies are inside it).
//!
//! What it found (2026-11-10, LiveKit 0.9.7 / libwebrtc m137): the encoder aims at exactly the
//! cap, and a change of the RTP sender's `max_bitrate` retargets it mid-call. RED doubles what goes
//! on the wire (Default 64 → ~122 kbps, 192 → ~386) until its packets would pass its size limit,
//! from 256 on, where it sends the encoder's output alone. And past 256 it sends ~260 kbps whatever
//! the encoder is allowed (384 and 510 too, every time), so 256 is the top preset. With RED off the
//! same mono publish at 510 sent ~510 on one connection and ~260 on the next (the send-side
//! estimate was above the rate both times; the cause isn't pinned down): not something to offer
//! as a preset (#283 has it, with stereo, for a music mode).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use livekit::options::TrackPublishOptions;
use livekit::prelude::*;
use livekit::webrtc::audio_frame::AudioFrame;
use livekit::webrtc::audio_source::native::NativeAudioSource;
use livekit::webrtc::audio_source::{AudioSourceOptions, RtcAudioSource};
use livekit::webrtc::stats::RtcStats;
use pulse_client::api::Api;
use pulse_client::voice::{mic_publish_options, set_mic_bitrate};
use pulse_protocol::rest::CHANNEL_BITRATES_KBPS;
use pulse_server::testing;

mod common;
use common::livekit_turn;

const DEV_SECRET: &str = "pulse-dev-secret-0123456789abcdefghij";
const WARM_UP: Duration = Duration::from_secs(4);
const MEASURE: Duration = Duration::from_secs(5);

fn dev_livekit(scheme: &str) -> String {
    let port = std::env::var("PULSE_DEV_LIVEKIT_PORT").unwrap_or_else(|_| "7880".into());
    format!("{scheme}://127.0.0.1:{port}")
}

/// Feeds `source` 10 ms frames of noise in real time until `run` is cleared.
fn feed_noise(source: NativeAudioSource, run: Arc<AtomicBool>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut seed: u32 = 0x1234_5678;
        let mut tick = tokio::time::interval(Duration::from_millis(10));
        while run.load(Ordering::SeqCst) {
            tick.tick().await;
            let data: Vec<i16> = (0..480)
                .map(|_| {
                    seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    ((seed >> 16) as i16) / 3
                })
                .collect();
            let frame = AudioFrame {
                data: data.into(),
                sample_rate: 48_000,
                num_channels: 1,
                samples_per_channel: 480,
            };
            let _ = source.capture_frame(&frame).await;
        }
    })
}

/// (payload bytes sent, the encoder's target bitrate) for `track`'s outbound RTP stream.
async fn sent(track: &LocalAudioTrack) -> (u64, f64) {
    for s in track.get_stats().await.unwrap_or_default() {
        if let RtcStats::OutboundRtp(o) = s {
            return (o.sent.bytes_sent, o.outbound.target_bitrate);
        }
    }
    (0, 0.0)
}

/// The kbps `track` sends over the next [`MEASURE`], and the encoder's target at its end.
async fn measure(track: &LocalAudioTrack) -> (f64, f64) {
    let (b0, _) = sent(track).await;
    let t = Instant::now();
    tokio::time::sleep(MEASURE).await;
    let (b1, target) = sent(track).await;
    let kbps = (b1 - b0) as f64 * 8.0 / t.elapsed().as_secs_f64() / 1000.0;
    (kbps, target / 1000.0)
}

struct Publisher {
    room: Room,
    track: LocalAudioTrack,
    run: Arc<AtomicBool>,
    feed: tokio::task::JoinHandle<()>,
}

impl Publisher {
    async fn start(url: &str, token: &str, kbps: u32, red: bool) -> Self {
        let mut opts = RoomOptions::default();
        opts.auto_subscribe = false;
        let (room, _events) = Room::connect(url, token, opts).await.unwrap();
        let source = NativeAudioSource::new(AudioSourceOptions::default(), 48_000, 1, 0);
        let track =
            LocalAudioTrack::create_audio_track("mic", RtcAudioSource::Native(source.clone()));
        room.local_participant()
            .publish_track(
                LocalTrack::Audio(track.clone()),
                TrackPublishOptions {
                    red,
                    ..mic_publish_options(kbps)
                },
            )
            .await
            .unwrap();
        let run = Arc::new(AtomicBool::new(true));
        let feed = feed_noise(source, run.clone());
        tokio::time::sleep(WARM_UP).await;
        Self {
            room,
            track,
            run,
            feed,
        }
    }

    async fn stop(self) {
        self.run.store(false, Ordering::SeqCst);
        let _ = self.feed.await;
        self.room.close().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn the_mic_sends_at_its_channels_bitrate() {
    let _turn = livekit_turn().await;
    let mut cfg = testing::test_config();
    cfg.livekit_url = dev_livekit("ws");
    cfg.livekit_secret = DEV_SECRET.into();
    let app = testing::spawn_with(cfg).await;
    let (_, alex) = testing::register(&app, "alex").await;
    let s = testing::create_server(&app, &alex, "Main").await;
    let api = Api::new(&format!("http://{}", app.addr));
    let lounge = api
        .channels(&alex, s.id)
        .await
        .unwrap()
        .into_iter()
        .find(|c| c.name.as_deref() == Some("Lounge"))
        .unwrap();

    // what each publish sends: (bitrate asked, kbps on the wire, the encoder's target)
    let run = |kbps: u32, red: bool| {
        let (api, alex, lounge) = (&api, &alex, lounge.id);
        async move {
            let vt = api.voice_token(alex, lounge).await.unwrap();
            let p = Publisher::start(&vt.url, &vt.token, kbps, red).await;
            let (sent, target) = measure(&p.track).await;
            p.stop().await;
            (kbps, sent, target)
        }
    };
    let within = |got: f64, want: f64, pct: f64| (got - want).abs() <= want * pct / 100.0;

    // 1. published as the app publishes (RED on) at Default (0) and each preset
    let mut presets = vec![0];
    presets.extend(CHANNEL_BITRATES_KBPS);
    let mut at_publish = Vec::new();
    for kbps in presets {
        let r = run(kbps, true).await;
        println!(
            "published at {:>3} (0 = Default): sends {:6.1} kbps, encoder target {:6.1} kbps",
            r.0, r.1, r.2
        );
        at_publish.push(r);
    }
    // 2. past the top preset, as the app publishes (RED on)
    let mut ceiling = Vec::new();
    for kbps in [256, 384, 510] {
        let r = run(kbps, true).await;
        println!(
            "past the presets, published at {:>3}: sends {:6.1} kbps, encoder target {:6.1} kbps",
            r.0, r.1, r.2
        );
        ceiling.push(r);
    }
    // 3. one publish, its bitrate changed live (no republish)
    let vt = api.voice_token(&alex, lounge.id).await.unwrap();
    let p = Publisher::start(&vt.url, &vt.token, 0, true).await;
    let mut live = Vec::new();
    for kbps in [0, 256, 96, 0] {
        set_mic_bitrate(&p.track, kbps).unwrap();
        // the encoder retargets at once; the window starts a moment later
        tokio::time::sleep(Duration::from_secs(1)).await;
        let (sent, target) = measure(&p.track).await;
        println!(
            "changed live to {kbps:>3} (0 = Default): sends {sent:6.1} kbps, encoder target {target:6.1} kbps"
        );
        live.push((kbps, sent, target));
    }
    p.stop().await;

    // The encoder aims at what the channel asks (Default: 64)
    for &(kbps, _, target) in &at_publish {
        let want = if kbps == 0 { 64.0 } else { f64::from(kbps) };
        assert!(within(target, want, 15.0), "{kbps}: {at_publish:?}");
    }
    // and really sends it: RED doubles it on the wire up to 192; at 256 its packets would be too
    // big, so RED sends the encoder's output alone
    for &(kbps, sent, target) in &at_publish {
        let want = if kbps == 256 { target } else { 2.0 * target };
        assert!(within(sent, want, 15.0), "{kbps}: {at_publish:?}");
    }
    // More than ~256 isn't sent, whatever the encoder is allowed: why 256 is the top preset
    let (_, at_256, _) = ceiling[0];
    assert!(at_256 > 230.0, "{ceiling:?}");
    for &(kbps, sent, _) in &ceiling[1..] {
        assert!(
            sent < at_256 * 1.1,
            "{kbps} sent more than 256: {ceiling:?}"
        );
    }
    // Live: up to the top, down, back to Default; the encoder follows each change
    for &(kbps, _, target) in &live {
        let want = if kbps == 0 { 64.0 } else { f64::from(kbps) };
        assert!(within(target, want, 15.0), "live {kbps}: {live:?}");
    }
    assert!(live[1].1 > live[0].1 * 1.8, "live change up: {live:?}");
    assert!(
        within(live[3].1, live[0].1, 15.0),
        "back to Default: {live:?}"
    );
}
