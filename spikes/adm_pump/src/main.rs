//! #216 reproduction: remote audio after the last PeerConnection in the process has closed.
//!
//! One PeerConnectionFactory for the whole run (as Pulse keeps LiveKit's runtime since 9d0e977d).
//! Each round: an in-process sender/receiver PeerConnection pair on that factory, a tone for 2 s,
//! count the frames with sound that `NativeAudioStream` yields, then close and drop both.
//!
//!   cargo run -- plain       nothing else alive between rounds (the field: leave, later rejoin)
//!   cargo run -- keep-idle   an idle PeerConnection lives for the whole run (voice_it's situation:
//!                            the test's own peer shares the process)
//!
//! No server, no audio devices.

use std::time::{Duration, Instant};

use libwebrtc::audio_frame::AudioFrame;
use libwebrtc::audio_source::AudioSourceOptions;
use libwebrtc::audio_source::native::NativeAudioSource;
use libwebrtc::audio_stream::native::NativeAudioStream;
use libwebrtc::media_stream_track::MediaStreamTrack;
use libwebrtc::peer_connection::{AnswerOptions, OfferOptions, PeerConnection};
use libwebrtc::peer_connection_factory::native::PeerConnectionFactoryExt;
use libwebrtc::peer_connection_factory::{PeerConnectionFactory, RtcConfiguration};
use libwebrtc::stats::RtcStats;
use tokio::sync::mpsc;
use tokio_stream::StreamExt;

fn trickle(from: &PeerConnection) -> mpsc::UnboundedReceiver<libwebrtc::ice_candidate::IceCandidate> {
    let (tx, rx) = mpsc::unbounded_channel();
    from.on_ice_candidate(Some(Box::new(move |c| {
        let _ = tx.send(c);
    })));
    rx
}

fn forward(
    mut rx: mpsc::UnboundedReceiver<libwebrtc::ice_candidate::IceCandidate>,
    to: PeerConnection,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(c) = rx.recv().await {
            let _ = to.add_ice_candidate(c).await;
        }
    })
}

async fn round(factory: &PeerConnectionFactory, n: usize) -> (usize, usize, u64, u64) {
    let a = factory.create_peer_connection(RtcConfiguration::default()).unwrap();
    let b = factory.create_peer_connection(RtcConfiguration::default()).unwrap();
    let (ca, cb) = (trickle(&a), trickle(&b));
    let (track_tx, mut track_rx) = mpsc::unbounded_channel();
    b.on_track(Some(Box::new(move |ev| {
        if let MediaStreamTrack::Audio(t) = ev.track {
            let _ = track_tx.send(t);
        }
    })));

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

    let remote = tokio::time::timeout(Duration::from_secs(5), track_rx.recv())
        .await
        .expect("no remote track")
        .unwrap();
    let mut stream = NativeAudioStream::new(remote, 48_000, 1);

    let feed = tokio::spawn(async move {
        let tone: Vec<i16> = (0..480).map(|i| ((i as f32 * 0.13).sin() * 8000.0) as i16).collect();
        let start = tokio::time::Instant::now();
        for k in 0u64.. {
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

    let (mut frames, mut sound) = (0usize, 0usize);
    let until = Instant::now() + Duration::from_secs(3);
    while let Ok(Some(f)) = tokio::time::timeout(until.saturating_duration_since(Instant::now()), stream.next()).await {
        frames += 1;
        if f.data.iter().any(|s| *s != 0) {
            sound += 1;
        }
    }
    let (mut packets, mut samples) = (0, 0);
    for s in b.get_stats().await.unwrap_or_default() {
        if let RtcStats::InboundRtp(r) = s {
            if r.stream.kind == "audio" {
                packets += r.received.packets_received;
                samples += r.inbound.total_samples_received;
            }
        }
    }
    feed.abort();
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
    // let libwebrtc release the PeerConnections (and with the last, the media engine)
    tokio::time::sleep(Duration::from_millis(500)).await;
    println!("round {n}: frames {frames}, with sound {sound}, packetsReceived {packets}, totalSamplesReceived {samples}");
    (frames, sound, packets, samples)
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "plain".into());
    let factory = PeerConnectionFactory::default();
    let _idle = (mode == "keep-idle")
        .then(|| factory.create_peer_connection(RtcConfiguration::default()).unwrap());
    println!("mode {mode}");
    for n in 1..=3 {
        round(&factory, n).await;
    }
}
