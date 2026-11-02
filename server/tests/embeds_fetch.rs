//! The link-preview fetcher's SSRF guards, against a fake site on loopback. `*.test` names are
//! fixed DNS answers (`EmbedConfig::hosts`); nothing here leaves the machine.

use std::net::IpAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::{Path, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use pulse_server::config::EmbedConfig;
use pulse_server::embeds::fetch::{
    FetchError, Fetcher, HTML_MAX, IMAGE_MAX, JSON_MAX, USER_AGENT, Want,
};

const LOOPBACK: IpAddr = IpAddr::V4(std::net::Ipv4Addr::LOCALHOST);
const TIKTOK: &str = "https://www.tiktok.com/@alex/video/7000000000000000001";

/// A fake site on 127.0.0.1 that counts every request it gets.
struct Fake {
    port: u16,
    hits: Arc<AtomicUsize>,
}

impl Fake {
    fn url(&self, host: &str, path: &str) -> String {
        format!("http://{host}:{}{path}", self.port)
    }
    fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }
}

fn html(body: impl Into<Body>) -> Response {
    (
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        body.into(),
    )
        .into_response()
}

fn found(to: String) -> Response {
    (StatusCode::FOUND, [(header::LOCATION, to)]).into_response()
}

async fn count(State(hits): State<Arc<AtomicUsize>>, req: Request, next: Next) -> Response {
    hits.fetch_add(1, Ordering::SeqCst);
    next.run(req).await
}

async fn fake() -> Fake {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let hits = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route(
            "/page",
            get(|| async { html("<html><head><title>Hi</title></head></html>") }),
        )
        .route(
            "/ua",
            get(|h: HeaderMap| async move {
                html(
                    h.get(header::USER_AGENT)
                        .map(|v| v.as_bytes().to_vec())
                        .unwrap_or_default(),
                )
            }),
        )
        .route(
            "/to-inner",
            get(move || async move { found(format!("http://inner.test:{port}/page")) }),
        )
        .route(
            // 127.0.0.2: loopback, but not the one address the tests allow.
            "/to-literal",
            get(move || async move { found(format!("http://127.0.0.2:{port}/page")) }),
        )
        .route(
            "/hop/{n}",
            get(|Path(n): Path<u32>| async move {
                if n == 0 {
                    html("<html></html>")
                } else {
                    found(format!("/hop/{}", n - 1))
                }
            }),
        )
        .route(
            "/slow",
            get(|| async {
                tokio::time::sleep(Duration::from_secs(3)).await;
                html("<html></html>")
            }),
        )
        .route(
            "/big.html",
            get(|| async { html(vec![b'a'; 2 * HTML_MAX]) }),
        )
        .route(
            "/big.json",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "application/json")],
                    vec![b' '; JSON_MAX + 1],
                )
            }),
        )
        .route(
            "/page.json",
            get(|| async { ([(header::CONTENT_TYPE, "application/json")], "{}") }),
        )
        .route(
            "/big.png",
            // Streamed, so no Content-Length warns the fetcher: the read itself must stop.
            get(|| async {
                let chunks = (0..(IMAGE_MAX >> 20) + 1)
                    .map(|_| Ok::<_, std::io::Error>(Bytes::from(vec![0u8; 1 << 20])));
                (
                    [(header::CONTENT_TYPE, "image/png")],
                    Body::from_stream(futures::stream::iter(chunks)),
                )
            }),
        )
        .route(
            "/small.png",
            get(|| async { ([(header::CONTENT_TYPE, "image/png")], vec![1u8; 64]) }),
        )
        .route(
            // 10 chunks 150 ms apart: each one well inside the timeout, the whole body not.
            "/drip",
            get(|| async {
                let chunks = futures::stream::unfold(0u32, |n| async move {
                    if n == 10 {
                        return None;
                    }
                    tokio::time::sleep(Duration::from_millis(150)).await;
                    Some((
                        Ok::<_, std::io::Error>(Bytes::from_static(b"<p>drip</p>")),
                        n + 1,
                    ))
                });
                html(Body::from_stream(chunks))
            }),
        )
        .route(
            // A redirect after 250 ms, `n` times: every hop inside the timeout, the chain not.
            "/slow-hop/{n}",
            get(|Path(n): Path<u32>| async move {
                tokio::time::sleep(Duration::from_millis(250)).await;
                if n == 0 {
                    html("<html></html>")
                } else {
                    found(format!("/slow-hop/{}", n - 1))
                }
            }),
        )
        .route("/short", get(|| async { found(TIKTOK.into()) }))
        .route("/short-chain", get(|| async { found("/short".into()) }))
        .layer(middleware::from_fn_with_state(hits.clone(), count));
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Fake { port, hits }
}

/// Loopback allowed (it's where the fake is), `site.test` on it and `inner.test` on a private
/// address.
fn cfg(f: &Fake) -> EmbedConfig {
    EmbedConfig {
        allowed_ips: vec![LOOPBACK],
        extra_ports: vec![f.port],
        hosts: vec![
            ("site.test".into(), LOOPBACK),
            ("inner.test".into(), "10.0.0.1".parse().unwrap()),
        ],
        timeout: Duration::from_secs(2),
        ..EmbedConfig::default()
    }
}

#[tokio::test]
async fn refuses_private_loopback_link_local_and_metadata_addresses() {
    let f = fake().await;
    // As in production: no address is allowed that isn't public.
    let fetcher = Fetcher::new(&EmbedConfig {
        allowed_ips: vec![],
        ..cfg(&f)
    });
    for host in [
        "127.0.0.1",
        "[::1]",
        "10.0.0.1",
        "169.254.169.254",
        "100.64.0.1",
        "[::ffff:127.0.0.1]",
        "site.test",
    ] {
        let r = fetcher.get(&f.url(host, "/page"), Want::Html).await;
        assert!(matches!(r, Err(FetchError::Blocked)), "{host}: {r:?}");
    }
    assert_eq!(f.hits(), 0);
}

/// Hosts the URL parser reads as IPv4 (a bare number, hex, a short form) are loopback here,
/// and refused like the dotted form, before any request.
#[tokio::test]
async fn refuses_loopback_written_as_a_number_or_short_form() {
    let f = fake().await;
    let fetcher = Fetcher::new(&EmbedConfig {
        allowed_ips: vec![],
        ..cfg(&f)
    });
    for host in ["2130706433", "0x7f.1", "127.1"] {
        let url = f.url(host, "/page");
        let r = fetcher.get(&url, Want::Html).await;
        assert!(matches!(r, Err(FetchError::Blocked)), "{url}: {r:?}");
        let r = fetcher.resolve_redirects(&url).await;
        assert!(matches!(r, Err(FetchError::Blocked)), "{url}: {r:?}");
    }
    assert_eq!(f.hits(), 0);
}

#[tokio::test]
async fn refuses_other_ports_and_schemes() {
    let f = fake().await;
    let fetcher = Fetcher::new(&cfg(&f));
    let r = fetcher.get("http://site.test:8080/page", Want::Html).await;
    assert!(matches!(r, Err(FetchError::Port)), "{r:?}");
    for url in [
        f.url("site.test", "/page").replace("http:", "ftp:"),
        "file:///etc/hosts".into(),
        f.url("alex:pw@site.test", "/page"),
        f.url("alex@site.test", "/page"),
        "not a url".into(),
    ] {
        let r = fetcher.get(&url, Want::Html).await;
        assert!(matches!(r, Err(FetchError::BadUrl)), "{url}: {r:?}");
    }
    assert_eq!(f.hits(), 0);
    // The same site through an allowed URL works, so the refusals above were the guards.
    fetcher
        .get(&f.url("site.test", "/page"), Want::Html)
        .await
        .unwrap();
    assert_eq!(f.hits(), 1);
}

#[tokio::test]
async fn rechecks_every_redirect() {
    let f = fake().await;
    let fetcher = Fetcher::new(&cfg(&f));
    for path in ["/to-inner", "/to-literal"] {
        let r = fetcher.get(&f.url("site.test", path), Want::Html).await;
        assert!(matches!(r, Err(FetchError::Blocked)), "{path}: {r:?}");
    }
    // Only the first hops got through.
    assert_eq!(f.hits(), 2);
}

#[tokio::test]
async fn stops_after_five_redirects() {
    let f = fake().await;
    let fetcher = Fetcher::new(&cfg(&f));
    let ok = fetcher
        .get(&f.url("site.test", "/hop/5"), Want::Html)
        .await
        .unwrap();
    assert_eq!(ok.final_url.path(), "/hop/0");
    assert_eq!(f.hits(), 6);
    let r = fetcher.get(&f.url("site.test", "/hop/6"), Want::Html).await;
    assert!(matches!(r, Err(FetchError::TooManyRedirects)), "{r:?}");
    assert_eq!(f.hits(), 12);
}

#[tokio::test]
async fn times_out() {
    let f = fake().await;
    let fetcher = Fetcher::new(&EmbedConfig {
        timeout: Duration::from_millis(300),
        ..cfg(&f)
    });
    let t = Instant::now();
    let r = fetcher.get(&f.url("site.test", "/slow"), Want::Html).await;
    assert!(matches!(r, Err(FetchError::Timeout)), "{r:?}");
    assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
}

/// The timeout is over the whole fetch: a body that keeps dripping, each piece in time, is cut
/// off at it.
#[tokio::test]
async fn the_timeout_covers_a_dripping_body() {
    let f = fake().await;
    let fetcher = Fetcher::new(&EmbedConfig {
        timeout: Duration::from_millis(600),
        ..cfg(&f)
    });
    let t = Instant::now();
    let r = fetcher.get(&f.url("site.test", "/drip"), Want::Html).await;
    assert!(matches!(r, Err(FetchError::Timeout)), "{r:?}");
    assert!(
        t.elapsed() < Duration::from_millis(1200),
        "{:?}",
        t.elapsed()
    );
}

/// ...and over every redirect hop together, not each one alone.
#[tokio::test]
async fn the_timeout_covers_a_chain_of_slow_redirects() {
    let f = fake().await;
    let fetcher = Fetcher::new(&EmbedConfig {
        timeout: Duration::from_millis(600),
        ..cfg(&f)
    });
    // One slow hop alone is fine.
    fetcher
        .get(&f.url("site.test", "/slow-hop/1"), Want::Html)
        .await
        .unwrap();
    for (what, r) in [
        (
            "get",
            fetcher
                .get(&f.url("site.test", "/slow-hop/4"), Want::Html)
                .await
                .map(|_| ()),
        ),
        (
            "resolve_redirects",
            fetcher
                .resolve_redirects(&f.url("site.test", "/slow-hop/4"))
                .await
                .map(|_| ()),
        ),
    ] {
        assert!(matches!(r, Err(FetchError::Timeout)), "{what}: {r:?}");
    }
    let t = Instant::now();
    let r = fetcher
        .get(&f.url("site.test", "/slow-hop/4"), Want::Html)
        .await;
    assert!(matches!(r, Err(FetchError::Timeout)), "{r:?}");
    assert!(
        t.elapsed() < Duration::from_millis(1000),
        "{:?}",
        t.elapsed()
    );
}

#[tokio::test]
async fn caps_html_and_refuses_oversized_images() {
    let f = fake().await;
    let fetcher = Fetcher::new(&cfg(&f));
    // A page over the cap is cut short, not refused.
    let page = fetcher
        .get(&f.url("site.test", "/big.html"), Want::Html)
        .await
        .unwrap();
    assert_eq!(page.body.len(), HTML_MAX);
    assert!(page.content_type.starts_with("text/html"));
    let r = fetcher
        .get(&f.url("site.test", "/big.png"), Want::Image)
        .await;
    assert!(matches!(r, Err(FetchError::TooLarge)), "{r:?}");
    let r = fetcher
        .get(&f.url("site.test", "/big.json"), Want::Json)
        .await;
    assert!(matches!(r, Err(FetchError::TooLarge)), "{r:?}");
    let small = fetcher
        .get(&f.url("site.test", "/small.png"), Want::Image)
        .await
        .unwrap();
    assert_eq!(small.body, vec![1u8; 64]);
}

#[tokio::test]
async fn html_only_for_page_fetches() {
    let f = fake().await;
    let fetcher = Fetcher::new(&cfg(&f));
    let r = fetcher
        .get(&f.url("site.test", "/small.png"), Want::Html)
        .await;
    assert!(matches!(r, Err(FetchError::ContentType)), "{r:?}");
    let r = fetcher.get(&f.url("site.test", "/page"), Want::Image).await;
    assert!(matches!(r, Err(FetchError::ContentType)), "{r:?}");
    let r = fetcher.get(&f.url("site.test", "/page"), Want::Json).await;
    assert!(matches!(r, Err(FetchError::ContentType)), "{r:?}");
    let json = fetcher
        .get(&f.url("site.test", "/page.json"), Want::Json)
        .await
        .unwrap();
    assert_eq!(json.body, b"{}");
    let r = fetcher
        .get(&f.url("site.test", "/nowhere"), Want::Html)
        .await;
    assert!(matches!(r, Err(FetchError::Status(404))), "{r:?}");
}

#[tokio::test]
async fn sends_the_honest_user_agent() {
    let f = fake().await;
    let fetcher = Fetcher::new(&cfg(&f));
    let r = fetcher
        .get(&f.url("site.test", "/ua"), Want::Html)
        .await
        .unwrap();
    assert_eq!(String::from_utf8(r.body).unwrap(), USER_AGENT);
    assert_eq!(USER_AGENT, "PulseBot/1.0 (+https://pulsechat.co.za)");
}

#[tokio::test]
async fn resolve_redirects_stops_at_the_first_known_link() {
    let f = fake().await;
    let fetcher = Fetcher::new(&cfg(&f));
    // Two hops, and the TikTok link itself is never requested (it would leave the machine).
    let u = fetcher
        .resolve_redirects(&f.url("site.test", "/short-chain"))
        .await
        .unwrap();
    assert_eq!(u.as_str(), TIKTOK);
    assert_eq!(f.hits(), 2);
    // A link that's already known needs no request at all.
    let u = fetcher.resolve_redirects(TIKTOK).await.unwrap();
    assert_eq!(u.as_str(), TIKTOK);
    assert_eq!(f.hits(), 2);
    // A page that doesn't redirect to anything known.
    let r = fetcher
        .resolve_redirects(&f.url("site.test", "/hop/1"))
        .await;
    assert!(matches!(r, Err(FetchError::NotResolved)), "{r:?}");
    // The guards hold here too.
    let r = fetcher
        .resolve_redirects(&f.url("inner.test", "/short"))
        .await;
    assert!(matches!(r, Err(FetchError::Blocked)), "{r:?}");
}
