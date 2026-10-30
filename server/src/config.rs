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
    /// Where clip videos live (S3-compatible, Cloudflare R2). `None` = clips storage isn't set up
    /// on this server.
    pub clips: Option<ClipStoreConfig>,
}

pulse_protocol::redacting_debug!(Config {
    db_url, bind, livekit_url, livekit_api_url, livekit_key, hello_timeout, heartbeat_timeout,
    limits, stream_media_timeout, media_dir, media_min_free_bytes, totp_key, clips ;
    livekit_secret
});

/// An S3-compatible bucket for clip videos, path style.
#[derive(Clone)]
pub struct ClipStoreConfig {
    pub endpoint: String,
    pub bucket: String,
    pub region: String,
    pub key_id: String,
    pub secret: String,
}

pulse_protocol::redacting_debug!(ClipStoreConfig { endpoint, bucket, region, key_id ; secret });

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let var = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.to_string());
        let livekit_url = var("PULSE_LIVEKIT_URL", "ws://localhost:7880");
        Ok(Self {
            db_url: var("PULSE_DB_URL", "sqlite://pulse.db"),
            bind: var("PULSE_BIND", "127.0.0.1:7890")
                .parse()
                .context("PULSE_BIND")?,
            livekit_api_url: var("PULSE_LIVEKIT_API_URL", &livekit_url),
            livekit_url,
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
            clips: ClipStoreConfig::from_env()?,
        })
    }
}

impl ClipStoreConfig {
    /// The clip store from the environment, `None` when it isn't set up (see [`clip_store`]).
    pub fn from_env() -> anyhow::Result<Option<Self>> {
        clip_store(|k| std::env::var(k).ok())
    }
}

/// The clip store from `PULSE_CLIPS_S3_*`. The bucket, key id and secret decide: none of the three
/// (even with an endpoint, which deploys always set) = no clips storage; some but not all = an
/// error naming the missing variables; all three = the endpoint is required too. The region
/// defaults to `auto`. Errors name variables, never values.
fn clip_store(get: impl Fn(&str) -> Option<String>) -> anyhow::Result<Option<ClipStoreConfig>> {
    let get = |k: &str| get(k).filter(|v| !v.is_empty());
    let (bucket, key_id, secret) = (
        get("PULSE_CLIPS_S3_BUCKET"),
        get("PULSE_CLIPS_S3_KEY_ID"),
        get("PULSE_CLIPS_S3_SECRET"),
    );
    let missing: Vec<&str> = [
        ("PULSE_CLIPS_S3_BUCKET", bucket.is_none()),
        ("PULSE_CLIPS_S3_KEY_ID", key_id.is_none()),
        ("PULSE_CLIPS_S3_SECRET", secret.is_none()),
    ]
    .into_iter()
    .filter_map(|(k, gone)| gone.then_some(k))
    .collect();
    let (Some(bucket), Some(key_id), Some(secret)) = (bucket, key_id, secret) else {
        if missing.len() == 3 {
            return Ok(None);
        }
        anyhow::bail!(
            "clips storage is half configured: set {} too (or unset the others)",
            missing.join(", ")
        );
    };
    let endpoint = get("PULSE_CLIPS_S3_ENDPOINT")
        .context("PULSE_CLIPS_S3_ENDPOINT is required when clips storage is configured")?;
    Ok(Some(ClipStoreConfig {
        endpoint,
        bucket,
        region: get("PULSE_CLIPS_S3_REGION").unwrap_or_else(|| "auto".into()),
        key_id,
        secret,
    }))
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
            clips: None,
        };
        let d = format!("{cfg:?}");
        assert!(!d.contains("SECRET"), "{d}");
        assert!(!d.contains(&"ab".repeat(32)), "{d}");
        assert!(d.contains("devkey") && d.contains("7890"), "{d}");
    }

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |k| {
            pairs
                .iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| v.to_string())
        }
    }

    #[test]
    fn clip_store_env_is_all_or_nothing() {
        // Nothing set (or only empty values): no clips storage.
        assert!(clip_store(env(&[])).unwrap().is_none());
        assert!(
            clip_store(env(&[("PULSE_CLIPS_S3_BUCKET", "")]))
                .unwrap()
                .is_none()
        );
        // Deploys always set the endpoint: on its own it changes nothing.
        assert!(
            clip_store(env(&[(
                "PULSE_CLIPS_S3_ENDPOINT",
                "https://s3.example.test"
            )]))
            .unwrap()
            .is_none()
        );

        let all = [
            ("PULSE_CLIPS_S3_ENDPOINT", "https://s3.example.test"),
            ("PULSE_CLIPS_S3_BUCKET", "clips"),
            ("PULSE_CLIPS_S3_KEY_ID", "kid"),
            ("PULSE_CLIPS_S3_SECRET", "SECRET-VALUE"),
        ];
        let c = clip_store(env(&all)).unwrap().unwrap();
        assert_eq!(
            (c.endpoint.as_str(), c.bucket.as_str(), c.region.as_str()),
            ("https://s3.example.test", "clips", "auto")
        );
        let mut with_region = all.to_vec();
        with_region.push(("PULSE_CLIPS_S3_REGION", "eu"));
        assert_eq!(clip_store(env(&with_region)).unwrap().unwrap().region, "eu");

        // Three of the four decisive/required ones: an error that names the missing one only.
        let msg = format!("{:#}", clip_store(env(&all[1..])).unwrap_err());
        assert!(msg.contains("PULSE_CLIPS_S3_ENDPOINT"), "{msg}");
        let no_secret: Vec<_> = all
            .iter()
            .copied()
            .filter(|(k, _)| !k.ends_with("SECRET"))
            .collect();
        let msg = format!("{:#}", clip_store(env(&no_secret)).unwrap_err());
        assert!(msg.contains("PULSE_CLIPS_S3_SECRET"), "{msg}");
        assert!(
            !msg.contains("SECRET-VALUE") && !msg.contains("kid"),
            "{msg}"
        );
        let msg = format!(
            "{:#}",
            clip_store(env(&[("PULSE_CLIPS_S3_BUCKET", "clips")])).unwrap_err()
        );
        assert!(msg.contains("KEY_ID") && msg.contains("SECRET"), "{msg}");
    }

    #[test]
    fn debug_never_shows_the_clip_secret() {
        let c = ClipStoreConfig {
            endpoint: "https://s3.example.test".into(),
            bucket: "clips".into(),
            region: "auto".into(),
            key_id: "kid".into(),
            secret: "SECRET-CLIP".into(),
        };
        let d = format!("{c:?}");
        assert!(!d.contains("SECRET"), "{d}");
        assert!(d.contains("clips") && d.contains("s3.example.test"), "{d}");
    }
}
