//! Which ICE servers our LiveKit hands to clients. LiveKit falls back to third-party STUN servers
//! (Twilio, Google) in the join response unless the node offers its own TURN/STUN, so a client
//! would contact them on every call. Only the node itself may appear.
//!
//! `prod_and_dev_configs_offer_only_our_own_turn` reads the config files and always runs;
//! `join_response_lists_only_our_own_ice_servers` needs the dev LiveKit (`just voice-it`). Pointed at another
//! LiveKit with ICE_CHECK_URL / ICE_CHECK_KEY / ICE_CHECK_SECRET / ICE_CHECK_NODE_IP (e.g. the production
//! livekit.yaml in a throwaway container) it checks that one instead.

use std::time::Duration;

use futures::StreamExt;
use livekit_api::access_token::{AccessToken, VideoGrants};
use livekit_protocol::{SignalResponse, signal_response};
use prost::Message as _;
use tokio_tungstenite::tungstenite::Message as Ws;

const DEV_SECRET: &str = "pulse-dev-secret-0123456789abcdefghij";
/// `rtc.node_ip` in docker/livekit.yaml.
const DEV_NODE_IP: &str = "127.0.0.1";

/// Host part of an ICE URL such as `turn:1.2.3.4:3478?transport=udp` or `stun:host:19302`.
fn ice_host(url: &str) -> &str {
    let rest = url.split_once(':').map_or(url, |(_, r)| r);
    let rest = rest.split('?').next().unwrap_or(rest);
    rest.rsplit_once(':').map_or(rest, |(h, _)| h)
}

#[test]
fn ice_host_parses_turn_and_stun_urls() {
    assert_eq!(ice_host("turn:127.0.0.1:3478?transport=udp"), "127.0.0.1");
    assert_eq!(
        ice_host("stun:stun.l.google.com:19302"),
        "stun.l.google.com"
    );
    assert_eq!(
        ice_host("turns:voice.example.com:443?transport=tcp"),
        "voice.example.com"
    );
}

#[test]
fn prod_and_dev_configs_offer_only_our_own_turn() {
    for path in ["../deploy/livekit.yaml", "../docker/livekit.yaml"] {
        let full = concat!(env!("CARGO_MANIFEST_DIR"), "/").to_string() + path;
        let yaml = std::fs::read_to_string(&full).unwrap();
        let lines: Vec<&str> = yaml
            .lines()
            .map(str::trim)
            .filter(|l| !l.starts_with('#'))
            .collect();
        // Any stun_servers entry is handed to every client; none of ours run a STUN server of their own.
        assert!(
            !lines.iter().any(|l| l.starts_with("stun_servers")),
            "{path}: stun_servers set"
        );
        // The embedded TURN's UDP port is what stops LiveKit adding its default (third-party) STUN servers.
        assert!(lines.contains(&"turn:"), "{path}: no turn section");
        assert!(lines.contains(&"enabled: true"), "{path}: turn not enabled");
        assert!(
            lines.iter().any(|l| l.starts_with("udp_port: 3478")),
            "{path}: no turn udp_port"
        );
        // use_external_ip discovers the node's address by asking third-party STUN servers at start-up.
        assert!(
            lines.contains(&"use_external_ip: false"),
            "{path}: use_external_ip not off"
        );
    }
}

#[tokio::test]
#[ignore]
async fn join_response_lists_only_our_own_ice_servers() {
    let env = |k: &str, dev: &str| std::env::var(k).unwrap_or_else(|_| dev.to_string());
    let base = env("ICE_CHECK_URL", &format!("ws://{DEV_NODE_IP}:7880"));
    let node_ip = env("ICE_CHECK_NODE_IP", DEV_NODE_IP);
    let jwt = AccessToken::with_api_key(
        &env("ICE_CHECK_KEY", "devkey"),
        &env("ICE_CHECK_SECRET", DEV_SECRET),
    )
    .with_identity("alex")
    .with_grants(VideoGrants {
        room_join: true,
        room: "ice-servers-check".into(),
        ..Default::default()
    })
    .to_jwt()
    .unwrap();
    let url = format!("{base}/rtc?access_token={jwt}&protocol=16&auto_subscribe=0&sdk=rust");
    let (mut ws, _) = tokio_tungstenite::connect_async(url)
        .await
        .expect("dev LiveKit up? (just dev-livekit)");
    let join = tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(msg) = ws.next().await {
            if let Ws::Binary(bytes) = msg.unwrap()
                && let Some(signal_response::Message::Join(j)) =
                    SignalResponse::decode(bytes).unwrap().message
            {
                return j;
            }
        }
        panic!("signal socket closed before the join response");
    })
    .await
    .expect("no join response");
    let _ = ws.close(None).await;

    let urls: Vec<String> = join
        .ice_servers
        .iter()
        .flat_map(|s| s.urls.clone())
        .collect();
    println!("ice servers: {urls:?}");
    assert!(
        urls.iter().any(|u| u.starts_with("turn:")),
        "the node's own TURN is missing: {urls:?}"
    );
    for u in &urls {
        assert_eq!(
            ice_host(u),
            node_ip,
            "third-party ICE server handed to clients: {u}"
        );
    }
}
