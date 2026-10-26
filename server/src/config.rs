use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Context;

use crate::auth::totp::TotpKey;

#[derive(Clone)]
pub struct Config {
    pub db_url: String,
    pub bind: SocketAddr,
    pub livekit_url: String,
    /// Where the server calls LiveKit's API (RoomService). Prod: loopback `http://`; the public
    /// `wss://` URL won't work here (no TLS stack for server-side API calls).
    pub livekit_api_url: String,
    pub livekit_key: String,
    pub livekit_secret: String,
    /// Time a new gateway socket has to send `Hello`.
    pub hello_timeout: Duration,
    /// Silence after which a gateway socket is considered dead.
    pub heartbeat_timeout: Duration,
    /// Rate limits (spec §3.1).
    pub limits: crate::limits::LimitsConfig,
    /// How long a streamer's video connection may be gone before the stream ends.
    pub stream_media_timeout: Duration,
    /// Where uploaded media is stored.
    pub media_dir: PathBuf,
    /// Uploads are refused when the media filesystem has less free space than this.
    pub media_min_free_bytes: u64,
    /// Encrypts TOTP secrets at rest. `None` = two-factor can't be set up (`PULSE_TOTP_KEY` unset).
    pub totp_key: Option<TotpKey>,
}

pulse_protocol::redacting_debug!(Config {
    db_url, bind, livekit_url, livekit_api_url, livekit_key, hello_timeout, heartbeat_timeout,
    limits, stream_media_timeout, media_dir, media_min_free_bytes, totp_key ;
    livekit_secret
});

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let var = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.to_string());
        Ok(Self {
            db_url: var("PULSE_DB_URL", "sqlite://pulse.db"),
            bind: var("PULSE_BIND", "127.0.0.1:7890")
                .parse()
                .context("PULSE_BIND")?,
            livekit_api_url: var(
                "PULSE_LIVEKIT_API_URL",
                &var("PULSE_LIVEKIT_URL", "ws://localhost:7880"),
            ),
            livekit_url: var("PULSE_LIVEKIT_URL", "ws://localhost:7880"),
            livekit_key: std::env::var("PULSE_LIVEKIT_KEY").context("PULSE_LIVEKIT_KEY")?,
            livekit_secret: std::env::var("PULSE_LIVEKIT_SECRET")
                .context("PULSE_LIVEKIT_SECRET")?,
            hello_timeout: Duration::from_secs(10),
            heartbeat_timeout: Duration::from_secs(60),
            limits: crate::limits::LimitsConfig::default(),
            stream_media_timeout: Duration::from_secs(60),
            media_dir: var("PULSE_MEDIA_DIR", "/data/media").into(),
            media_min_free_bytes: 2 * 1024 * 1024 * 1024,
            totp_key: totp_key(std::env::var("PULSE_TOTP_KEY").ok().as_deref())?,
        })
    }
}

/// Unset or empty = no key. A malformed one is an error that names the variable, never its value.
fn totp_key(raw: Option<&str>) -> anyhow::Result<Option<TotpKey>> {
    match raw {
        None | Some("") => Ok(None),
        Some(v) => TotpKey::parse(v)
            .map(Some)
            .context("PULSE_TOTP_KEY must be exactly 64 hex characters"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn totp_key_env() {
        assert!(totp_key(None).unwrap().is_none());
        assert!(totp_key(Some("")).unwrap().is_none());
        assert!(totp_key(Some(&"ab".repeat(32))).unwrap().is_some());
        let bad = "not-a-key-s3cret";
        let msg = format!("{:#}", totp_key(Some(bad)).unwrap_err());
        assert!(msg.contains("PULSE_TOTP_KEY"));
        assert!(!msg.contains(bad));
    }

    /// Debug output can end up in logs: never the LiveKit API secret.
    #[test]
    fn debug_never_shows_the_livekit_secret() {
        let cfg = Config {
            db_url: "sqlite://pulse.db".into(),
            bind: "127.0.0.1:7890".parse().unwrap(),
            livekit_url: "ws://localhost:7880".into(),
            livekit_api_url: "http://127.0.0.1:7880".into(),
            livekit_key: "devkey".into(),
            livekit_secret: "SECRET-LIVEKIT".into(),
            hello_timeout: Duration::from_secs(10),
            heartbeat_timeout: Duration::from_secs(60),
            limits: crate::limits::LimitsConfig::default(),
            stream_media_timeout: Duration::from_secs(60),
            media_dir: "/data/media".into(),
            media_min_free_bytes: 1,
            totp_key: Some(TotpKey::parse(&"ab".repeat(32)).unwrap()),
        };
        let d = format!("{cfg:?}");
        assert!(!d.contains("SECRET"), "{d}");
        assert!(!d.contains(&"ab".repeat(32)), "{d}");
        assert!(d.contains("devkey") && d.contains("7890"), "{d}");
    }
}
