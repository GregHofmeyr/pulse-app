//! Fetching other sites for link previews, guarded against SSRF: a sender picks the URL, so the
//! fetcher must never reach the server itself, the tailnet, cloud metadata or anything private.
//!
//! - `http`/`https` on ports 80 and 443 only, no credentials in the URL;
//! - our own DNS resolution, refusing a name if *any* answer isn't public, and connecting to the
//!   checked answers (a resolver inside the client, so no second lookup can rebind the name);
//! - IP-literal hosts (including the number and short forms the URL parser reads as IPv4, such
//!   as `2130706433` or `127.1`) checked the same way. "Public" is [`is_public`]: not private,
//!   loopback, link-local, carrier-grade NAT, benchmarking, multicast or reserved, and no IPv6
//!   form that reaches IPv4 or tunnels elsewhere (mapped, compatible, NAT64 `64:ff9b::/96` and
//!   `64:ff9b:1::/48`, 6to4 `2002::/16`, Teredo `2001::/32`) or discards (`100::/64`);
//! - redirects followed by hand, each one checked again;
//! - one timeout over the whole fetch, size caps, and only the content type asked for.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use reqwest::header::{ACCEPT, CONTENT_TYPE, LOCATION};
use reqwest::{StatusCode, Url, redirect};

use super::providers::{self, Detection};
use crate::config::EmbedConfig;

/// Most of a page read; the rest is cut off (the metadata is in the head).
pub const HTML_MAX: usize = 1 << 20;
/// Most of an oEmbed answer; more is refused.
pub const JSON_MAX: usize = 256 << 10;
/// Most of a thumbnail; more is refused.
pub const IMAGE_MAX: usize = 8 << 20;
/// Most redirects one fetch follows.
pub const MAX_REDIRECTS: usize = 5;
/// Who's asking, honestly (some sites only show their tags to bots they know).
pub const USER_AGENT: &str = "PulseBot/1.0 (+https://pulsechat.co.za)";

/// What a fetch is for: decides the content types accepted and the size cap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Want {
    /// `text/html`, cut at [`HTML_MAX`].
    Html,
    /// JSON (oEmbed), at most [`JSON_MAX`].
    Json,
    /// `image/*`, at most [`IMAGE_MAX`].
    Image,
}

impl Want {
    fn accept(self) -> &'static str {
        match self {
            Want::Html => "text/html",
            Want::Json => "application/json",
            Want::Image => "image/*",
        }
    }

    fn accepts(self, content_type: &str) -> bool {
        let mime = content_type
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        match self {
            Want::Html => mime == "text/html",
            // oEmbed's own type is `application/json+oembed`.
            Want::Json => {
                matches!(
                    mime.as_str(),
                    "application/json" | "text/json" | "application/json+oembed"
                ) || mime.ends_with("+json")
            }
            Want::Image => mime.starts_with("image/"),
        }
    }

    fn cap(self) -> usize {
        match self {
            Want::Html => HTML_MAX,
            Want::Json => JSON_MAX,
            Want::Image => IMAGE_MAX,
        }
    }
}

#[derive(Debug)]
pub struct Fetched {
    /// Where the body came from, after redirects.
    pub final_url: Url,
    pub content_type: String,
    pub body: Vec<u8>,
}

#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error("not an http(s) URL with a host and no credentials")]
    BadUrl,
    #[error("port not allowed")]
    Port,
    #[error("address not public")]
    Blocked,
    #[error("too many redirects")]
    TooManyRedirects,
    #[error("redirect without a usable Location")]
    BadRedirect,
    #[error("the redirects led nowhere known")]
    NotResolved,
    #[error("timed out")]
    Timeout,
    #[error("status {0}")]
    Status(u16),
    #[error("unexpected content type")]
    ContentType,
    #[error("body over the size cap")]
    TooLarge,
    #[error("network: {0}")]
    Network(reqwest::Error),
}

impl From<reqwest::Error> for FetchError {
    /// The resolver's refusal comes back wrapped in the client's errors: find it.
    fn from(e: reqwest::Error) -> Self {
        let mut src: Option<&(dyn std::error::Error + 'static)> = Some(&e);
        while let Some(s) = src {
            if s.is::<Refused>() {
                return FetchError::Blocked;
            }
            src = s.source();
        }
        if e.is_timeout() {
            FetchError::Timeout
        } else {
            FetchError::Network(e)
        }
    }
}

/// Whether `ip` is an address on the public internet. False for this host, private and
/// carrier-grade NAT ranges, link-local (cloud metadata), benchmarking, multicast and reserved
/// ranges, for IPv6 forms that carry one of those IPv4 addresses (mapped, compatible, NAT64),
/// and for all of the IPv6 prefixes whose far side we can't check: local-use NAT64
/// (`64:ff9b:1::/48`), 6to4 (`2002::/16`), Teredo (`2001::/32`) and discard-only (`100::/64`).
pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4_public(v4),
        IpAddr::V6(v6) => match embedded_v4(v6) {
            Some(v4) => v4_public(v4),
            None => {
                let s = v6.segments();
                let first = s[0];
                !(v6.is_unspecified()
                    || v6.is_loopback()
                    || first & 0xfe00 == 0xfc00 // unique local
                    || first & 0xffc0 == 0xfe80 // link-local
                    || first & 0xffc0 == 0xfec0 // site-local (deprecated)
                    || first & 0xff00 == 0xff00 // multicast
                    || first == 0x2002 // 6to4
                    || (first == 0x2001 && s[1] == 0) // Teredo
                    || s[..3] == [0x64, 0xff9b, 1] // local-use NAT64
                    || s[..4] == [0x100, 0, 0, 0]) // discard-only
            }
        },
    }
}

fn v4_public(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(a == 0
        || a == 10
        || (a == 100 && (64..=127).contains(&b))
        || a == 127
        || (a == 169 && b == 254)
        || (a == 172 && (16..=31).contains(&b))
        || (a == 192 && b == 0 && c == 0)
        || (a == 192 && b == 168)
        || (a == 198 && (18..=19).contains(&b))
        // Multicast, reserved and broadcast.
        || a >= 224)
}

/// The IPv4 address inside an IPv4-mapped (`::ffff:a.b.c.d`), IPv4-compatible (`::a.b.c.d`,
/// which takes in `::` and `::1`) or NAT64 (`64:ff9b::a.b.c.d`) address.
fn embedded_v4(ip: Ipv6Addr) -> Option<Ipv4Addr> {
    let s = ip.segments();
    let [.., a, b, c, d] = ip.octets();
    let mapped_or_compat = s[..5] == [0; 5] && (s[5] == 0xffff || s[5] == 0);
    let nat64 = s[..6] == [0x64, 0xff9b, 0, 0, 0, 0];
    (mapped_or_compat || nat64).then(|| Ipv4Addr::new(a, b, c, d))
}

/// The resolver's refusal of a name with a non-public answer.
#[derive(Debug)]
struct Refused;

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a non-public address")
    }
}

impl std::error::Error for Refused {}

/// The client's DNS: every name the client connects to comes through here, redirects included.
#[derive(Clone)]
struct GuardResolver {
    allowed: Arc<Vec<IpAddr>>,
    hosts: Arc<Vec<(String, IpAddr)>>,
}

impl GuardResolver {
    fn allows(&self, ip: IpAddr) -> bool {
        is_public(ip) || self.allowed.contains(&ip)
    }

    /// The name's answers, all of which must be allowed: one private answer next to public ones
    /// (rebinding) refuses the name.
    async fn lookup(
        &self,
        name: &str,
    ) -> Result<Vec<IpAddr>, Box<dyn std::error::Error + Send + Sync>> {
        let fixed: Vec<IpAddr> = self
            .hosts
            .iter()
            .filter(|(h, _)| h.eq_ignore_ascii_case(name))
            .map(|(_, ip)| *ip)
            .collect();
        let ips = if fixed.is_empty() {
            tokio::net::lookup_host((name, 0))
                .await?
                .map(|a| a.ip())
                .collect()
        } else {
            fixed
        };
        if ips.is_empty() {
            return Err("no addresses".into());
        }
        if !ips.iter().all(|&ip| self.allows(ip)) {
            return Err(Box::new(Refused));
        }
        Ok(ips)
    }
}

impl Resolve for GuardResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let this = self.clone();
        Box::pin(async move {
            let ips = this.lookup(name.as_str()).await?;
            // The port is the URL's; the client sets it.
            let addrs: Addrs = Box::new(ips.into_iter().map(|ip| SocketAddr::new(ip, 0)));
            Ok(addrs)
        })
    }
}

/// The end of a walk through redirects.
enum Walked {
    /// A response that isn't a redirect, and the URL it came from.
    Response(Url, reqwest::Response),
    /// A redirect target the caller asked to stop at, not requested.
    Stopped(Url),
}

/// Fetches pages, oEmbed answers and thumbnails for link previews; see the module doc.
pub struct Fetcher {
    client: reqwest::Client,
    resolver: GuardResolver,
    ports: Vec<u16>,
    timeout: Duration,
}

impl Fetcher {
    pub fn new(cfg: &EmbedConfig) -> Fetcher {
        let resolver = GuardResolver {
            allowed: Arc::new(cfg.allowed_ips.clone()),
            hosts: Arc::new(cfg.hosts.clone()),
        };
        let client = reqwest::Client::builder()
            .redirect(redirect::Policy::none())
            .no_proxy()
            .dns_resolver(Arc::new(resolver.clone()))
            .user_agent(USER_AGENT)
            .timeout(cfg.timeout)
            .build()
            // Only fails when the TLS backend can't start, which is a broken build.
            .expect("the link preview HTTP client builds");
        let mut ports = vec![80, 443];
        ports.extend(&cfg.extra_ports);
        Fetcher {
            client,
            resolver,
            ports,
            timeout: cfg.timeout,
        }
    }

    /// `url`'s body, if it's what `want` asks for, within its cap. Redirects are followed (at
    /// most [`MAX_REDIRECTS`]); the whole call, body included, is under the timeout.
    pub async fn get(&self, url: &str, want: Want) -> Result<Fetched, FetchError> {
        let url = Url::parse(url).map_err(|_| FetchError::BadUrl)?;
        tokio::time::timeout(self.timeout, self.get_inner(url, want))
            .await
            .map_err(|_| FetchError::Timeout)?
    }

    async fn get_inner(&self, url: Url, want: Want) -> Result<Fetched, FetchError> {
        let (final_url, mut resp) = match self.walk(url, want.accept(), |_| false).await? {
            Walked::Response(u, r) => (u, r),
            Walked::Stopped(_) => unreachable!("never asked to stop"),
        };
        if !resp.status().is_success() {
            return Err(FetchError::Status(resp.status().as_u16()));
        }
        let content_type = resp
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_owned();
        if !want.accepts(&content_type) {
            return Err(FetchError::ContentType);
        }
        let cap = want.cap();
        let truncate = want == Want::Html;
        if !truncate && resp.content_length().is_some_and(|n| n > cap as u64) {
            return Err(FetchError::TooLarge);
        }
        let mut body = Vec::new();
        while let Some(chunk) = resp.chunk().await? {
            let room = cap - body.len();
            if chunk.len() > room {
                if !truncate {
                    return Err(FetchError::TooLarge);
                }
                body.extend_from_slice(&chunk[..room]);
                break;
            }
            body.extend_from_slice(&chunk);
        }
        Ok(Fetched {
            final_url,
            content_type,
            body,
        })
    }

    /// For a provider's short link: follows `url`'s redirects (at most [`MAX_REDIRECTS`]) to the
    /// first URL [`providers::detect`] knows, and returns it without requesting it.
    pub async fn resolve_redirects(&self, url: &str) -> Result<Url, FetchError> {
        let url = Url::parse(url).map_err(|_| FetchError::BadUrl)?;
        let known = |u: &Url| matches!(providers::detect(u), Detection::Known(_));
        if known(&url) {
            return Ok(url);
        }
        let walk = self.walk(url, Want::Html.accept(), known);
        match tokio::time::timeout(self.timeout, walk)
            .await
            .map_err(|_| FetchError::Timeout)??
        {
            Walked::Stopped(u) => Ok(u),
            Walked::Response(..) => Err(FetchError::NotResolved),
        }
    }

    /// Request `url`, following redirects by hand and checking every URL before it's requested.
    /// `stop` sees each redirect target first and ends the walk there when it says so.
    async fn walk(
        &self,
        mut url: Url,
        accept: &str,
        stop: impl Fn(&Url) -> bool,
    ) -> Result<Walked, FetchError> {
        for hop in 0..=MAX_REDIRECTS {
            self.check(&url)?;
            let resp = self
                .client
                .get(url.clone())
                .header(ACCEPT, accept)
                .send()
                .await?;
            let redirect = matches!(
                resp.status(),
                StatusCode::MOVED_PERMANENTLY
                    | StatusCode::FOUND
                    | StatusCode::SEE_OTHER
                    | StatusCode::TEMPORARY_REDIRECT
                    | StatusCode::PERMANENT_REDIRECT
            );
            if !redirect {
                return Ok(Walked::Response(url, resp));
            }
            if hop == MAX_REDIRECTS {
                return Err(FetchError::TooManyRedirects);
            }
            let next = resp
                .headers()
                .get(LOCATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|loc| url.join(loc).ok())
                .ok_or(FetchError::BadRedirect)?;
            if stop(&next) {
                return Ok(Walked::Stopped(next));
            }
            url = next;
        }
        unreachable!("the last hop returns")
    }

    /// The checks the resolver can't make: scheme, credentials, port, and IP-literal hosts (the
    /// client connects to those without asking the resolver).
    fn check(&self, url: &Url) -> Result<(), FetchError> {
        if !matches!(url.scheme(), "http" | "https")
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(FetchError::BadUrl);
        }
        if !url
            .port_or_known_default()
            .is_some_and(|p| self.ports.contains(&p))
        {
            return Err(FetchError::Port);
        }
        let host = url.host_str().unwrap_or("");
        if host.is_empty() {
            return Err(FetchError::BadUrl);
        }
        // An IPv6 literal is printed in brackets. A name that would parse as an IPv4 address was
        // already turned into one by the URL parser.
        let literal = host.trim_start_matches('[').trim_end_matches(']');
        let Ok(ip) = literal.parse::<IpAddr>() else {
            return Ok(());
        };
        if self.resolver.allows(ip) {
            Ok(())
        } else {
            Err(FetchError::Blocked)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn is_public_table() {
        for ip in [
            "0.0.0.0",
            "0.1.2.3",
            "10.0.0.1",
            "100.64.0.1",
            "100.127.255.254",
            "127.0.0.1",
            "127.1.2.3",
            "169.254.169.254",
            "172.16.0.1",
            "172.31.255.255",
            "192.0.0.8",
            "192.168.1.1",
            "198.18.0.1",
            "198.19.255.255",
            "224.0.0.1",
            "239.255.255.250",
            "240.0.0.1",
            "255.255.255.255",
            "::",
            "::1",
            "fc00::1",
            "fd12:3456::1",
            "fe80::1",
            "febf::1",
            "fec0::1",
            "ff02::1",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
            "::ffff:169.254.169.254",
            "::127.0.0.1",
            "::10.0.0.1",
            "64:ff9b::10.0.0.1",
            "64:ff9b::127.0.0.1",
            // Tunnels and translators that reach IPv4 (or anything) on the far side.
            "2002::1",
            "2002:7f00:1::1",
            "2002:cb00:7109::1",
            "64:ff9b:1::a00:1",
            "64:ff9b:1:ffff::1",
            "2001::1",
            "2001:0:4136:e378:8000:63bf:3fff:fdd2",
            // Discard-only.
            "100::1",
            "100::ffff:ffff:ffff:ffff",
        ] {
            assert!(!is_public(ip.parse().unwrap()), "{ip}");
        }
        // Documentation ranges stand in for public addresses here.
        for ip in [
            "192.0.2.1",
            "198.51.100.7",
            "203.0.113.9",
            "100.63.255.255",
            "100.128.0.1",
            "172.15.255.255",
            "172.32.0.1",
            "198.17.255.255",
            "198.20.0.1",
            "223.255.255.254",
            "2001:db8::1",
            "::ffff:192.0.2.1",
            "64:ff9b::203.0.113.9",
            "2001:1::1",
            "2003::1",
            "64:ff9b:2::1",
            "100:0:0:1::1",
        ] {
            assert!(is_public(ip.parse().unwrap()), "{ip}");
        }
    }

    fn resolver(hosts: &[(&str, &str)]) -> GuardResolver {
        GuardResolver {
            allowed: Arc::new(vec![]),
            hosts: Arc::new(
                hosts
                    .iter()
                    .map(|(h, ip)| (h.to_string(), ip.parse().unwrap()))
                    .collect(),
            ),
        }
    }

    async fn answers(r: &GuardResolver, name: &str) -> Result<Vec<SocketAddr>, String> {
        match r.resolve(Name::from_str(name).unwrap()).await {
            Ok(a) => Ok(a.collect()),
            Err(e) => Err(if e.is::<Refused>() {
                "refused".into()
            } else {
                e.to_string()
            }),
        }
    }

    /// DNS rebinding: a name with a public and a private answer is refused as a whole, so the
    /// client can't be steered to the private one.
    #[tokio::test]
    async fn refuses_a_name_with_any_private_answer() {
        let r = resolver(&[
            ("mixed.test", "203.0.113.5"),
            ("mixed.test", "10.0.0.1"),
            ("v6.test", "2001:db8::5"),
            ("v6.test", "fe80::1"),
            ("public.test", "203.0.113.5"),
            ("public.test", "2001:db8::5"),
        ]);
        assert_eq!(answers(&r, "mixed.test").await.unwrap_err(), "refused");
        assert_eq!(answers(&r, "v6.test").await.unwrap_err(), "refused");
        assert_eq!(answers(&r, "MIXED.test").await.unwrap_err(), "refused");
        let ok = answers(&r, "public.test").await.unwrap();
        assert_eq!(
            ok.iter().map(|a| a.ip().to_string()).collect::<Vec<_>>(),
            ["203.0.113.5", "2001:db8::5"]
        );
    }

    #[test]
    fn content_types() {
        assert!(Want::Html.accepts("text/html; charset=utf-8"));
        assert!(Want::Html.accepts("Text/HTML"));
        assert!(!Want::Html.accepts("application/xhtml+xml"));
        assert!(!Want::Html.accepts(""));
        assert!(Want::Json.accepts("application/json; charset=utf-8"));
        assert!(Want::Json.accepts("application/json+oembed"));
        assert!(!Want::Json.accepts("text/html"));
        assert!(Want::Image.accepts("image/webp"));
        assert!(!Want::Image.accepts("text/html"));
    }
}
