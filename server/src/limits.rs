//! Rate limits: in-memory token buckets (one node, so no shared store is needed).

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Mutex;
use std::time::Instant;

use axum::extract::{ConnectInfo, FromRequestParts};
use axum::http::request::Parts;

/// A bucket of `burst` tokens, topped up at `refill_per_sec`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rate {
    pub burst: u32,
    pub refill_per_sec: f64,
}

impl Rate {
    pub const fn new(burst: u32, refill_per_sec: f64) -> Self {
        Self {
            burst,
            refill_per_sec,
        }
    }
    pub fn per_minute(n: u32) -> Self {
        Self::new(n, n as f64 / 60.0)
    }
    pub fn per_hour(n: u32) -> Self {
        Self::new(n, n as f64 / 3600.0)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct LimitsConfig {
    /// Failed logins per client IP.
    pub login_ip: Rate,
    /// Failed logins per username.
    pub login_user: Rate,
    /// Failed registrations (unknown/used/expired invite) per client IP.
    pub register_ip: Rate,
    /// Messages sent per user.
    pub send_user: Rate,
    /// Typing frames per user (extra ones are dropped).
    pub typing_user: Rate,
}

impl Default for LimitsConfig {
    fn default() -> Self {
        Self {
            login_ip: Rate::per_minute(5),
            login_user: Rate::per_hour(10),
            register_ip: Rate::per_hour(5),
            send_user: Rate::new(20, 5.0),
            typing_user: Rate::new(1, 1.0),
        }
    }
}

impl LimitsConfig {
    /// Effectively no limits (most tests).
    pub fn unlimited() -> Self {
        let r = Rate::new(u32::MAX, 1e9);
        Self {
            login_ip: r,
            login_user: r,
            register_ip: r,
            send_user: r,
            typing_user: r,
        }
    }
}

struct Bucket {
    tokens: f64,
    at: Instant,
}

/// Buckets kept before idle (full again) ones are swept out.
const MAX_KEYS: usize = 10_000;

pub struct Limiter {
    rate: Rate,
    buckets: Mutex<HashMap<String, Bucket>>,
}

impl Limiter {
    pub fn new(rate: Rate) -> Self {
        Self {
            rate,
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// Whether `key` has a token left (doesn't spend it).
    pub fn allowed(&self, key: &str) -> bool {
        self.allowed_at(key, Instant::now())
    }

    /// Spend a token for `key`; false if there was none.
    pub fn hit(&self, key: &str) -> bool {
        self.hit_at(key, Instant::now())
    }

    pub fn allowed_at(&self, key: &str, now: Instant) -> bool {
        self.with(key, now, |t| *t >= 1.0)
    }

    pub fn hit_at(&self, key: &str, now: Instant) -> bool {
        self.with(key, now, |t| {
            if *t >= 1.0 {
                *t -= 1.0;
                true
            } else {
                false
            }
        })
    }

    fn with(&self, key: &str, now: Instant, f: impl FnOnce(&mut f64) -> bool) -> bool {
        let rate = self.rate;
        let mut map = self.buckets.lock().unwrap();
        if map.len() >= MAX_KEYS && !map.contains_key(key) {
            map.retain(|_, b| refilled(b, now, rate) < rate.burst as f64);
        }
        let b = map.entry(key.to_owned()).or_insert(Bucket {
            tokens: rate.burst as f64,
            at: now,
        });
        b.tokens = refilled(b, now, rate);
        b.at = now;
        f(&mut b.tokens)
    }

    /// Give back a token spent by [`Limiter::hit`] (never above the burst; unknown keys are ignored).
    pub fn refund(&self, key: &str) {
        if let Some(b) = self.buckets.lock().unwrap().get_mut(key) {
            b.tokens = (b.tokens + 1.0).min(self.rate.burst as f64);
        }
    }

    /// Whether no keys are tracked.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// How many keys are tracked (tests, and a future metrics page).
    pub fn len(&self) -> usize {
        self.buckets.lock().unwrap().len()
    }
}

fn refilled(b: &Bucket, now: Instant, rate: Rate) -> f64 {
    let secs = now.saturating_duration_since(b.at).as_secs_f64();
    (b.tokens + secs * rate.refill_per_sec).min(rate.burst as f64)
}

pub struct Limits {
    pub login_ip: Limiter,
    pub login_user: Limiter,
    pub register_ip: Limiter,
    pub send_user: Limiter,
    pub typing_user: Limiter,
}

impl Limits {
    pub fn new(c: &LimitsConfig) -> Self {
        Self {
            login_ip: Limiter::new(c.login_ip),
            login_user: Limiter::new(c.login_user),
            register_ip: Limiter::new(c.register_ip),
            send_user: Limiter::new(c.send_user),
            typing_user: Limiter::new(c.typing_user),
        }
    }
}

/// The real client address: the right-most `X-Forwarded-For` entry (the one Caddy wrote),
/// else the socket peer. Trusting the header is safe only because production listens on
/// loopback behind Caddy.
pub fn client_ip(forwarded_for: Option<&str>, peer: Option<IpAddr>) -> IpAddr {
    forwarded_for
        .and_then(|h| h.rsplit(',').next())
        .and_then(|s| s.trim().parse().ok())
        .or(peer)
        .unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED))
}

/// Extractor for [`client_ip`]. Never rejects.
pub struct ClientIp(pub IpAddr);

impl<S: Send + Sync> FromRequestParts<S> for ClientIp {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        let xff = parts
            .headers
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok());
        let peer = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|c| c.0.ip());
        Ok(Self(client_ip(xff, peer)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn bucket_allows_a_burst_then_refills_up_to_the_burst() {
        let l = Limiter::new(Rate::new(3, 1.0));
        let t0 = Instant::now();
        assert!(l.hit_at("a", t0));
        assert!(l.hit_at("a", t0));
        assert!(l.hit_at("a", t0));
        assert!(!l.hit_at("a", t0));
        assert!(l.hit_at("b", t0), "keys are independent");
        assert!(!l.allowed_at("a", t0 + Duration::from_millis(500)));
        assert!(l.allowed_at("a", t0 + Duration::from_secs(1)));
        assert!(
            l.allowed_at("a", t0 + Duration::from_secs(1)),
            "allowed() doesn't spend"
        );
        let later = t0 + Duration::from_secs(3600);
        for _ in 0..3 {
            assert!(l.hit_at("a", later));
        }
        assert!(
            !l.hit_at("a", later),
            "never more than the burst, however long it idled"
        );
    }

    #[test]
    fn refund_gives_a_token_back_but_never_above_the_burst() {
        let l = Limiter::new(Rate::new(2, 0.0));
        let t0 = Instant::now();
        assert!(l.hit_at("a", t0));
        assert!(l.hit_at("a", t0));
        assert!(!l.hit_at("a", t0));
        l.refund("a");
        assert!(l.hit_at("a", t0), "the refunded token is spendable");
        l.refund("a");
        l.refund("a");
        l.refund("a");
        assert!(
            l.hit_at("a", t0) && l.hit_at("a", t0) && !l.hit_at("a", t0),
            "capped at the burst"
        );
        l.refund("never-seen");
        assert_eq!(l.len(), 1, "refunding an unknown key creates nothing");
    }

    #[test]
    fn idle_buckets_are_swept_when_the_map_is_full() {
        let l = Limiter::new(Rate::new(2, 1.0));
        let t0 = Instant::now();
        for i in 0..MAX_KEYS {
            l.hit_at(&format!("k{i}"), t0);
        }
        assert_eq!(l.len(), MAX_KEYS);
        // 10 s later every bucket is full again: a new key sweeps them out.
        l.hit_at("new", t0 + Duration::from_secs(10));
        assert_eq!(l.len(), 1);
    }

    #[test]
    fn client_ip_takes_the_entry_caddy_appended() {
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        let peer = Some(ip("127.0.0.1"));
        assert_eq!(client_ip(Some("203.0.113.7"), peer), ip("203.0.113.7"));
        // a client can't dodge limits with its own header: the proxy's entry is last
        assert_eq!(
            client_ip(Some("1.2.3.4, 203.0.113.7"), peer),
            ip("203.0.113.7")
        );
        assert_eq!(client_ip(Some(" 2001:db8::1 "), peer), ip("2001:db8::1"));
        assert_eq!(client_ip(Some("garbage"), peer), ip("127.0.0.1"));
        assert_eq!(client_ip(None, peer), ip("127.0.0.1"));
        assert_eq!(client_ip(None, None), ip("0.0.0.0"));
    }
}
