//! Remote audio must keep playing after the last PeerConnection in the process has gone (#216).
//!
//! Field of the bug: after a call, every later call in the same app run subscribed to the peer
//! but never heard them. When the last PeerConnection is released, libwebrtc terminates the voice
//! engine (`ConnectionContext::ReleaseMediaEngine` -> `WebRtcVoiceEngine::Terminate` ->
//! `AdmProxy::Terminate`), which took the synthetic ADM's 10 ms `NeedMorePlayData` pump with it.
//! The next PeerConnection initialises the engine again, but upstream's `AdmProxy::Init` is a
//! no-op, so the pump never came back: RTP arrives, NetEq is never pulled, `NativeAudioStream`
//! yields nothing. Pulse keeps the factory for the app's life (9d0e977d), so it lasted until a
//! restart. Upstream: livekit/rust-sdks#1468. Fixed in vendor/webrtc-sys (src/adm_proxy.cpp
//! `AdmProxy::Init`, src/synthetic_audio_device.cpp).
//!
//! This plays it without LiveKit: one factory for the whole run (as the app), and per round an
//! in-process sender/receiver PeerConnection pair on it with a tone, then both closed, dropped and
//! released before the next round. Needs no server or devices. Before the fix: frames with sound
//! ~300 in round 1, 0 in rounds 2 and 3.

use std::time::Duration;

use futures::StreamExt;
use livekit::webrtc::audio_frame::AudioFrame;
use livekit::webrtc::audio_source::AudioSourceOptions;
use livekit::webrtc::audio_source::native::NativeAudioSource;
use livekit::webrtc::audio_stream::native::NativeAudioStream;
use livekit::webrtc::ice_candidate::IceCandidate;
use livekit::webrtc::media_stream_track::MediaStreamTrack;
use livekit::webrtc::peer_connection::{AnswerOptions, OfferOptions, PeerConnection};
use livekit::webrtc::peer_connection_factory::native::PeerConnectionFactoryExt;
use livekit::webrtc::peer_connection_factory::{PeerConnectionFactory, RtcConfiguration};
use livekit::webrtc::stats::RtcStats;
use tokio::sync::mpsc;

/// How long each round listens. The tone is 10 ms frames, so a working pump yields ~300 frames.
const LISTEN: Duration = Duration::from_secs(3);
/// Frames with sound a round must yield: well under the ~300 a working round gets (round 1 loses
/// some to start-up on a loaded machine), far above the 0 of a dead pump.
const MIN_SOUND_FRAMES: usize = 100;

struct Round {
    frames: usize,
    sound: usize,
    packets: u64,
    samples: u64,
}

fn trickle(from: &PeerConnection) -> mpsc::UnboundedReceiver<IceCandidate> {
    let (tx, rx) = mpsc::unbounded_channel();
    from.on_ice_candidate(Some(Box::new(move |c| {
        let _ = tx.send(c);
    })));
    rx
}

fn forward(
    mut rx: mpsc::UnboundedReceiver<IceCandidate>,
    to: PeerConnection,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(c) = rx.recv().await {
            let _ = to.add_ice_candidate(c).await;
        }
    })
}

/// A sender/receiver pair on `factory`: a tone for `LISTEN`, counting what the receiver's
/// `NativeAudioStream` yields; then everything is closed and dropped.
async fn round(factory: &PeerConnectionFactory) -> Round {
    let a = factory
        .create_peer_connection(RtcConfiguration::default())
        .unwrap();
    let b = factory
        .create_peer_connection(RtcConfiguration::default())
        .unwrap();
    let (ca, cb) = (trickle(&a), trickle(&b));
    let (track_tx, mut track_rx) = mpsc::unbounded_channel();
    b.on_track(Some(Box::new(move |ev| {
        if let MediaStreamTrack::Audio(t) = ev.track {
            let _ = track_tx.send(t);
        }
    })));

    // queue 0, as the app's mic
    let source = NativeAudioSource::new(AudioSourceOptions::default(), 48_000, 1, 0);
    let track = factory.create_audio_track("mic", source.clone());
    a.add_track(MediaStreamTrack::Audio(track), &["s"]).unwrap();

    let offer = a.create_offer(OfferOptions::default()).await.unwrap();
    a.set_local_description(offer.clone()).await.unwrap();
    b.set_remote_description(offer).await.unwrap();
    let answer = b.create_answer(AnswerOptions::default()).await.unwrap();
    b.set_local_description(answer.clone()).await.unwrap();
    a.set_remote_description(answer).await.unwrap();
    let fwd = [forward(ca, b.clone()), forward(cb, a.clone())];

    let remote = tokio::time::timeout(Duration::from_secs(10), track_rx.recv())
        .await
        .expect("no remote track")
        .unwrap();
    let mut stream = NativeAudioStream::new(remote, 48_000, 1);

    let feed = tokio::spawn(async move {
        let tone: Vec<i16> = (0..480)
            .map(|i| ((i as f32 * 0.13).sin() * 8000.0) as i16)
            .collect();
        let start = tokio::time::Instant::now();
        for k in 0..=u64::MAX {
            let frame = AudioFrame {
                data: tone.clone().into(),
                sample_rate: 48_000,
                num_channels: 1,
                samples_per_channel: 480,
            };
            let _ = source.capture_frame(&frame).await;
            tokio::time::sleep_until(start + Duration::from_millis(10 * (k + 1))).await;
        }
    });

    let (mut frames, mut sound) = (0, 0);
    let until = tokio::time::Instant::now() + LISTEN;
    while let Ok(Some(f)) = tokio::time::timeout_at(until, stream.next()).await {
        frames += 1;
        if f.data.iter().any(|s| *s != 0) {
            sound += 1;
        }
    }
    let (mut packets, mut samples) = (0, 0);
    for s in b.get_stats().await.unwrap_or_default() {
        if let RtcStats::InboundRtp(r) = s
            && r.stream.kind == "audio"
        {
            packets += r.received.packets_received;
            samples += r.inbound.total_samples_received;
        }
    }

    feed.abort();
    let _ = feed.await;
    for f in fwd {
        f.abort();
        let _ = f.await;
    }
    a.on_ice_candidate(None);
    b.on_ice_candidate(None);
    b.on_track(None);
    drop(stream);
    a.close();
    b.close();
    drop((a, b));
    Round {
        frames,
        sound,
        packets,
        samples,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn remote_audio_plays_after_the_last_peer_connection_has_gone() {
    let factory = PeerConnectionFactory::default();
    let mut rounds = Vec::new();
    for n in 1..=3 {
        let r = round(&factory).await;
        println!(
            "round {n}: frames {}, with sound {}, packetsReceived {}, totalSamplesReceived {}",
            r.frames, r.sound, r.packets, r.samples
        );
        rounds.push(r);
        // let libwebrtc release both PeerConnections, and with the last one terminate the voice
        // engine (the field: leave, later join again)
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    for (n, r) in rounds.iter().enumerate() {
        let n = n + 1;
        assert!(
            r.packets > 0,
            "round {n}: no RTP arrived (the test is broken)"
        );
        assert!(
            r.sound >= MIN_SOUND_FRAMES,
            "round {n}: {} frames with sound ({} frames, totalSamplesReceived {}): \
             WebRTC is not pulling remote audio",
            r.sound,
            r.frames,
            r.samples
        );
    }
}
