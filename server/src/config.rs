use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Context;

#[derive(Clone, Debug)]
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
}

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
        })
    }
}
