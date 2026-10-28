//! Dropping a PeerConnection while libwebrtc is still applying a remote description must not
//! abort the process.
//!
//! Field of the bug: `just voice-it` aborted about 1 run in 3 with `libc++abi: Pure virtual
//! function called!` at the end of `rapid_join_leave_never_hangs`. The test's tokio runtime shut
//! down while a cancelled join's LiveKit session was mid `SetRemoteDescription`; cancelling its
//! task dropped the last reference to the PeerConnection on a worker thread. libwebrtc's proxy
//! destructor then waited on the signaling thread, which was still applying that description and
//! called the half-destroyed observer's `OnTrack`, which copies the PeerConnection pointer being
//! destroyed (AddRef on a partly destroyed object: a pure virtual call).
//!
//! This reproduces that without LiveKit: start a remote offer on a fresh PeerConnection, then drop
//! the PeerConnection at once. Needs no server or devices. Fixed in vendor/webrtc-sys
//! (src/peer_connection.cpp, `~PeerConnection` closes the PeerConnection first).

use std::future::Future;
use std::pin::pin;
use std::task::{Context, Waker};

use livekit::webrtc::MediaType;
use livekit::webrtc::peer_connection::OfferOptions;
use livekit::webrtc::peer_connection_factory::{PeerConnectionFactory, RtcConfiguration};
use livekit::webrtc::rtp_transceiver::{RtpTransceiverDirection, RtpTransceiverInit};

/// Each round races the drop against libwebrtc applying the offer. Before the fix the first
/// round already aborted every time on the machine this was written on; the rounds are cheap.
const ROUNDS: usize = 50;

#[tokio::test(flavor = "multi_thread")]
async fn dropping_a_peer_connection_mid_set_remote_description_is_safe() {
    let factory = PeerConnectionFactory::default();
    let offerer = factory
        .create_peer_connection(RtcConfiguration::default())
        .unwrap();
    // an audio (and video) transceiver: applying this offer on the other side fires OnTrack
    for media in [MediaType::Audio, MediaType::Video] {
        offerer
            .add_transceiver_for_media(
                media,
                RtpTransceiverInit {
                    direction: RtpTransceiverDirection::SendRecv,
                    stream_ids: vec!["s".into()],
                    send_encodings: Vec::new(),
                },
            )
            .unwrap();
    }
    let offer = offerer.create_offer(OfferOptions::default()).await.unwrap();

    for _ in 0..ROUNDS {
        let answerer = factory
            .create_peer_connection(RtcConfiguration::default())
            .unwrap();
        {
            // poll once: libwebrtc starts applying the offer on its signaling thread
            let mut srd = pin!(answerer.set_remote_description(offer.clone()));
            let _ = srd.as_mut().poll(&mut Context::from_waker(Waker::noop()));
        } // the future is dropped unfinished: its completion callback has nowhere to go
        drop(answerer); // the last reference, while the offer is (most likely) still being applied
    }

    // a dropped PeerConnection is fully torn down: the factory still works afterwards
    let late = factory
        .create_peer_connection(RtcConfiguration::default())
        .unwrap();
    late.set_remote_description(offer).await.unwrap();
    late.close();
    offerer.close();
}
