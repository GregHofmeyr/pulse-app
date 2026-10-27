//! Which ICE servers our LiveKit hands to clients. LiveKit falls back to third-party STUN servers
//! (Twilio, Google) in the join response unless the node offers its own TURN/STUN, so a client
//! would contact them on every call. Only the node itself may appear.
//!
//! `prod_and_dev_configs_offer_only_our_own_turn` reads the config files (production, dev and the Windows test
//! kit's) and always runs;
//! `join_response_lists_only_our_own_ice_servers` needs the dev LiveKit (`just voice-it`). Pointed at another
//! LiveKit with ICE_CHECK_URL / ICE_CHECK_KEY / ICE_CHECK_SECRET / ICE_CHECK_NODE_IP (e.g. the production
//! livekit.yaml in a throwaway container) it checks that one instead.

use std::time::Duration;

use futures::StreamExt;
use livekit_api::access_token::{AccessToken, VideoGrants};
use livekit_protocol::{SignalResponse, signal_response};
use prost::Message as _;
use tokio_tungstenite::tungstenite::Message as Ws;
use yaml_rust2::YamlLoader;

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

/// The ICE-relevant settings of one LiveKit config file; panics with the file name on any violation.
fn assert_only_own_turn(path: &str, yaml: &str) {
    let docs = YamlLoader::load_from_str(yaml).unwrap_or_else(|e| panic!("{path}: {e}"));
    let conf = &docs[0];
    let (rtc, turn) = (&conf["rtc"], &conf["turn"]);
    // Every stun_servers entry is handed to every client (and used for IP discovery); we run no STUN server
    // of our own, and an external TURN would be someone else's too.
    assert!(
        rtc["stun_servers"].is_badvalue(),
        "{path}: rtc.stun_servers set"
    );
    assert!(
        rtc["turn_servers"].is_badvalue(),
        "{path}: rtc.turn_servers set"
    );
    // The embedded TURN's UDP port is what stops LiveKit adding its default (third-party) STUN servers.
    assert_eq!(
        turn["enabled"].as_bool(),
        Some(true),
        "{path}: turn.enabled"
    );
    assert!(
        turn["udp_port"].as_i64().is_some_and(|p| p > 0),
        "{path}: turn.udp_port"
    );
    // use_external_ip discovers the node's address by asking third-party STUN servers at start-up.
    assert_eq!(
        rtc["use_external_ip"].as_bool(),
        Some(false),
        "{path}: rtc.use_external_ip"
    );
}

/// Every LiveKit config in the repository: production, dev (docker-compose.yml) and the Windows test kit.
const CONFIGS: [&str; 3] = [
    "../deploy/livekit.yaml",
    "../docker/livekit.yaml",
    "../kit/windows/livekit.yaml",
];

#[test]
fn prod_and_dev_configs_offer_only_our_own_turn() {
    for path in CONFIGS {
        let full = concat!(env!("CARGO_MANIFEST_DIR"), "/").to_string() + path;
        assert_only_own_turn(
            path,
            &std::fs::read_to_string(&full).unwrap_or_else(|e| panic!("{path}: {e}")),
        );
    }
}

/// The HTTP/signalling listener (`port`) must be loopback-only: in production Caddy is the only way in,
/// and with network_mode: host an unset `bind_addresses` would expose 7880 on every interface (tailnet
/// included). In LiveKit v1.13.10 the top-level `bind_addresses` governs only that listener (plus the
/// metrics/debug ones, unused here); RTC and TURN listeners are configured separately.
fn assert_signalling_on_loopback(path: &str, yaml: &str) {
    let docs = YamlLoader::load_from_str(yaml).unwrap_or_else(|e| panic!("{path}: {e}"));
    let addrs: Vec<_> = docs[0]["bind_addresses"]
        .as_vec()
        .unwrap_or_else(|| panic!("{path}: top-level bind_addresses missing"))
        .iter()
        .map(|a| a.as_str())
        .collect();
    assert_eq!(
        addrs,
        [Some("127.0.0.1")],
        "{path}: bind_addresses must be exactly [127.0.0.1]"
    );
}

#[test]
fn prod_and_dev_signalling_binds_loopback_only() {
    for path in CONFIGS {
        let full = concat!(env!("CARGO_MANIFEST_DIR"), "/").to_string() + path;
        assert_signalling_on_loopback(
            path,
            &std::fs::read_to_string(&full).unwrap_or_else(|e| panic!("{path}: {e}")),
        );
    }
}

#[test]
fn signalling_bind_check_catches_each_violation() {
    assert_signalling_on_loopback("good", "bind_addresses: [\"127.0.0.1\"]\n");
    for bad in [
        "port: 7880\n",
        "bind_addresses: [\"0.0.0.0\"]\n",
        "bind_addresses: [\"127.0.0.1\", \"0.0.0.0\"]\n",
        // right key, wrong section (the TURN one governs only TURN)
        "turn:\n  bind_addresses: [\"127.0.0.1\"]\n",
    ] {
        let caught =
            std::panic::catch_unwind(|| assert_signalling_on_loopback("bad", bad)).is_err();
        assert!(caught, "not caught:\n{bad}");
    }
}

/// The Windows test kit runs everything on one PC: UDP media and TURN stay on loopback. Interface names differ on
/// Windows ("Loopback Pseudo-Interface 1", not `lo`), so the kit filters candidates by address (`rtc.ips`).
/// The ICE-TCP port (rtc.tcp_port) can't be restricted; LiveKit binds it on every interface.
fn assert_media_on_loopback(path: &str, yaml: &str) {
    let docs = YamlLoader::load_from_str(yaml).unwrap_or_else(|e| panic!("{path}: {e}"));
    let (rtc, turn) = (&docs[0]["rtc"], &docs[0]["turn"]);
    let strs = |v: &yaml_rust2::Yaml| -> Option<Vec<String>> {
        v.as_vec()?
            .iter()
            .map(|s| s.as_str().map(str::to_string))
            .collect()
    };
    assert_eq!(
        rtc["node_ip"].as_str(),
        Some("127.0.0.1"),
        "{path}: rtc.node_ip"
    );
    assert_eq!(
        rtc["enable_loopback_candidate"].as_bool(),
        Some(true),
        "{path}: rtc.enable_loopback_candidate"
    );
    assert_eq!(
        strs(&rtc["ips"]["includes"]),
        Some(vec!["127.0.0.1/32".to_string()]),
        "{path}: rtc.ips.includes must be exactly [127.0.0.1/32]"
    );
    assert_eq!(
        strs(&turn["bind_addresses"]),
        Some(vec!["127.0.0.1".to_string()]),
        "{path}: turn.bind_addresses must be exactly [127.0.0.1]"
    );
}

#[test]
fn kit_media_and_turn_on_loopback_only() {
    let path = "../kit/windows/livekit.yaml";
    let full = concat!(env!("CARGO_MANIFEST_DIR"), "/").to_string() + path;
    assert_media_on_loopback(
        path,
        &std::fs::read_to_string(&full).unwrap_or_else(|e| panic!("{path}: {e}")),
    );
}

#[test]
fn kit_loopback_check_catches_each_violation() {
    let good = "rtc:\n  node_ip: 127.0.0.1\n  enable_loopback_candidate: true\n  ips:\n    includes: [127.0.0.1/32]\n\
                turn:\n  bind_addresses: [\"127.0.0.1\"]\n";
    assert_media_on_loopback("good", good);
    for bad in [
        good.replace("  node_ip: 127.0.0.1\n", "  node_ip: 192.0.2.1\n"),
        good.replace(
            "enable_loopback_candidate: true",
            "enable_loopback_candidate: false",
        ),
        good.replace("includes: [127.0.0.1/32]", "includes: [0.0.0.0/0]"),
        good.replace("includes: [127.0.0.1/32]", "excludes: [192.0.2.0/24]"),
        good.replace("[\"127.0.0.1\"]", "[\"0.0.0.0\"]"),
        good.replace(
            "turn:\n  bind_addresses: [\"127.0.0.1\"]\n",
            "turn:\n  enabled: true\n",
        ),
    ] {
        let caught = std::panic::catch_unwind(|| assert_media_on_loopback("bad", &bad)).is_err();
        assert!(caught, "not caught:\n{bad}");
    }
}

#[test]
fn config_check_catches_each_violation() {
    let good = "rtc:\n  use_external_ip: false\nturn:\n  enabled: true\n  udp_port: 3478\n";
    assert_only_own_turn("good", good);
    for bad in [
        "rtc:\n  use_external_ip: true\nturn:\n  enabled: true\n  udp_port: 3478\n",
        "rtc:\n  use_external_ip: false\n  stun_servers: [a:1]\nturn:\n  enabled: true\n  udp_port: 3478\n",
        "rtc:\n  use_external_ip: false\nturn:\n  enabled: false\n  udp_port: 3478\n",
        "rtc:\n  use_external_ip: false\nturn:\n  enabled: true\n",
        // the right keys in the wrong section
        "rtc:\n  enabled: true\n  udp_port: 3478\nturn:\n  use_external_ip: false\n",
    ] {
        let caught = std::panic::catch_unwind(|| assert_only_own_turn("bad", bad)).is_err();
        assert!(caught, "not caught:\n{bad}");
    }
}

#[tokio::test]
#[ignore]
async fn join_response_lists_only_our_own_ice_servers() {
    let env = |k: &str, dev: &str| std::env::var(k).unwrap_or_else(|_| dev.to_string());
    let port = env("PULSE_DEV_LIVEKIT_PORT", "7880"); // docker-compose.yml; `just voice-it-alt` moves it
    let base = env("ICE_CHECK_URL", &format!("ws://{DEV_NODE_IP}:{port}"));
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
