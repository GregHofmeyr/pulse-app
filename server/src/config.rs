use std::net::SocketAddr;
use std::time::Duration;

use anyhow::Context;

#[derive(Clone, Debug)]
pub struct Config {
    pub db_url: String,
    pub bind: SocketAddr,
    pub livekit_url: String,
    pub livekit_key: String,
    pub livekit_secret: String,
    /// Time a new gateway socket has to send `Hello`.
    pub hello_timeout: Duration,
    /// Silence after which a gateway socket is considered dead.
    pub heartbeat_timeout: Duration,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let var = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.to_string());
        Ok(Self {
            db_url: var("PULSE_DB_URL", "sqlite://pulse.db"),
            bind: var("PULSE_BIND", "127.0.0.1:7890")
                .parse()
                .context("PULSE_BIND")?,
            livekit_url: var("PULSE_LIVEKIT_URL", "ws://localhost:7880"),
            livekit_key: std::env::var("PULSE_LIVEKIT_KEY").context("PULSE_LIVEKIT_KEY")?,
            livekit_secret: std::env::var("PULSE_LIVEKIT_SECRET")
                .context("PULSE_LIVEKIT_SECRET")?,
            hello_timeout: Duration::from_secs(10),
            heartbeat_timeout: Duration::from_secs(60),
        })
    }
}
