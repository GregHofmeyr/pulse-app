//! Local videos for the Linux webview, over loopback HTTP. WebKitGTK's media player (GStreamer)
//! refuses video from the `asset://` scheme (FormatError, nothing loads), though images over it
//! work and the same file plays from `http://`. So on Linux the clip editor's `<video>` gets
//! `http://127.0.0.1:<port>/<token>` (`commands::video_url`); Windows keeps the asset protocol.
//!
//! - A token is 32 random bytes (64 hex digits) naming exactly one file. Tokens are given only for
//!   paths the webview may load through the asset protocol anyway (`allowed`, the asset scope),
//!   and that is checked again as each request starts: a forbidden path gets no new response (one
//!   already streaming runs on until its token is revoked).
//! - `revoke` (an editor source closed) and `revoke_all` (sign-out) end tokens: later requests
//!   get a 404 and a response in flight is cut off, even one stuck writing to a client that
//!   stopped reading. Any body write that takes over 30 s drops the client too. Nothing from the
//!   request ever names a file: only the token map does.
//! - Just `GET` and `HEAD`, one request per connection (`Connection: close`), at most 32
//!   connections at once (more are closed unanswered). The head is capped at 8 KB and must arrive
//!   within 10 s; peers that aren't loopback are dropped. `Host` must be `127.0.0.1:<port>` or
//!   `localhost:<port>` (else 421), so a DNS-rebound page can't read a video.
//! - Ranges: `bytes=a-b`, `bytes=a-` and `bytes=-n` get a 206, one past the end a 416. Several
//!   ranges at once (GStreamer never asks) get the whole file (200): ignoring Range is always allowed.

use std::{
    collections::HashMap,
    net::{Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, Weak},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{OnceCell, Semaphore, watch},
};

/// The request line and headers, at most.
const MAX_HEAD: usize = 8 * 1024;
/// How long a client has to send them.
const HEAD_TIMEOUT: Duration = Duration::from_secs(10);
/// How long one write of the body may take: a client that stops reading is dropped.
const WRITE_TIMEOUT: Duration = Duration::from_secs(30);
/// Connections served at once; one more is closed at once. The editor plays one or two videos.
const MAX_CONNS: usize = 32;
const CHUNK: usize = 64 * 1024;

/// The server's limits (tests shorten them).
#[derive(Clone, Copy)]
struct Limits {
    head_timeout: Duration,
    write_timeout: Duration,
    max_conns: usize,
}

const LIMITS: Limits = Limits {
    head_timeout: HEAD_TIMEOUT,
    write_timeout: WRITE_TIMEOUT,
    max_conns: MAX_CONNS,
};

/// Whether the webview may load this file (the asset scope).
pub type Allowed = Box<dyn Fn(&Path) -> bool + Send + Sync>;

/// The token map and the server, started on first use.
pub struct Videos {
    inner: Arc<Inner>,
}

struct Inner {
    allowed: Allowed,
    files: Mutex<Files>,
    addr: OnceCell<SocketAddr>,
    limits: Limits,
    /// One per connection being served.
    conns: Arc<Semaphore>,
}

#[derive(Default)]
struct Files {
    by_token: HashMap<String, Grant>,
    /// (owner, canonical path) → token
    by_key: HashMap<(String, PathBuf), String>,
}

#[derive(Clone)]
struct Grant {
    path: PathBuf,
    /// Set when revoked: a response in flight stops, even mid-write.
    revoked: Arc<watch::Sender<bool>>,
}

impl Videos {
    pub fn new(allowed: Allowed) -> Self {
        Self::with_limits(allowed, LIMITS)
    }

    fn with_limits(allowed: Allowed, limits: Limits) -> Self {
        Self {
            inner: Arc::new(Inner {
                allowed,
                files: Mutex::default(),
                addr: OnceCell::new(),
                limits,
                conns: Arc::new(Semaphore::new(limits.max_conns)),
            }),
        }
    }

    /// Start the server if it isn't running. `false` if it can't start.
    pub async fn start(&self) -> bool {
        match self.inner.addr.get_or_try_init(|| start(&self.inner)).await {
            Ok(_) => true,
            Err(e) => {
                tracing::warn!(error = %e, "video server unavailable");
                false
            }
        }
    }

    /// `http://127.0.0.1:<port>/<token>` for `path` on behalf of `owner` (an editor source; the
    /// same token while it lasts). Synchronous, so the caller can make it under its own lock
    /// (`Core::video_url`). `path` must be canonical, so that aliases of one file share a token.
    /// `None` if the webview may not load `path`, or the server isn't started.
    pub fn grant(&self, owner: &str, path: &Path) -> Option<String> {
        let addr = *self.inner.addr.get()?;
        if !(self.inner.allowed)(path) {
            return None;
        }
        let mut files = self.inner.files.lock().unwrap();
        let key = (owner.to_owned(), path.to_path_buf());
        let token = match files.by_key.get(&key) {
            Some(t) => t.clone(),
            None => {
                let t = hex(&rand::random::<[u8; 32]>());
                let grant = Grant {
                    path: path.to_path_buf(),
                    revoked: Arc::new(watch::Sender::new(false)),
                };
                files.by_token.insert(t.clone(), grant);
                files.by_key.insert(key, t.clone());
                t
            }
        };
        Some(format!("http://{addr}/{token}"))
    }

    /// `owner`'s tokens stop working: a request for one is a 404 from now on, and a response in
    /// flight ends.
    pub fn revoke(&self, owner: &str) {
        let files = &mut *self.inner.files.lock().unwrap();
        files.by_key.retain(|(o, _), t| {
            if o != owner {
                return true;
            }
            if let Some(g) = files.by_token.remove(t) {
                g.revoked.send_replace(true);
            }
            false
        });
    }

    /// How many tokens `owner` has.
    #[cfg(test)]
    pub(crate) fn tokens(&self, owner: &str) -> usize {
        let files = self.inner.files.lock().unwrap();
        files.by_key.keys().filter(|(o, _)| o == owner).count()
    }

    /// Every token stops working (sign-out).
    pub fn revoke_all(&self) {
        let mut files = self.inner.files.lock().unwrap();
        files.by_key.clear();
        for (_, g) in files.by_token.drain() {
            g.revoked.send_replace(true);
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Bind `127.0.0.1:0` and serve from a task of its own. The task holds the `Videos` weakly: once
/// it is dropped the next connection ends the task (and closes the port); until then it waits.
async fn start(inner: &Arc<Inner>) -> std::io::Result<SocketAddr> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    let addr = listener.local_addr()?;
    let inner = Arc::downgrade(inner);
    tokio::spawn(accept(listener, inner));
    Ok(addr)
}

async fn accept(listener: TcpListener, inner: Weak<Inner>) {
    loop {
        let (conn, peer) = match listener.accept().await {
            Ok(c) => c,
            // e.g. out of file descriptors: wait for some to free up rather than spin
            Err(_) => {
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        let Some(inner) = inner.upgrade() else {
            return;
        };
        // bound to loopback anyway
        if !peer.ip().is_loopback() {
            continue;
        }
        // full: this one is dropped (closed) at once
        let Ok(permit) = inner.conns.clone().try_acquire_owned() else {
            continue;
        };
        tokio::spawn(async move {
            serve(inner, conn).await;
            drop(permit);
        });
    }
}

/// What a client asked for, once its head has been read and parsed.
struct Request {
    method: String,
    target: String,
    host: Option<String>,
    range: Option<String>,
}

enum Head {
    Ok(Request),
    Malformed,
    TooLarge,
    /// Closed, or the read failed, before a whole head arrived.
    Gone,
}

/// The request line and headers (the body, if any, is never read).
async fn read_head(conn: &mut TcpStream) -> Head {
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    let end = loop {
        let n = match conn.read(&mut chunk).await {
            Ok(0) | Err(_) => return Head::Gone,
            Ok(n) => n,
        };
        // the terminator may straddle two reads
        let from = buf.len().saturating_sub(3);
        buf.extend_from_slice(&chunk[..n]);
        if let Some(i) = buf[from..].windows(4).position(|w| w == b"\r\n\r\n") {
            break from + i;
        }
        if buf.len() > MAX_HEAD {
            return Head::TooLarge;
        }
    };
    if end > MAX_HEAD {
        return Head::TooLarge;
    }
    let Ok(head) = std::str::from_utf8(&buf[..end]) else {
        return Head::Malformed;
    };
    parse_head(head).map_or(Head::Malformed, Head::Ok)
}

fn parse_head(head: &str) -> Option<Request> {
    let mut lines = head.split("\r\n");
    let mut line = lines.next()?.split(' ');
    let (method, target, version) = (line.next()?, line.next()?, line.next()?);
    if method.is_empty() || line.next().is_some() || !version.starts_with("HTTP/1.") {
        return None;
    }
    let (mut host, mut range) = (None, None);
    for l in lines {
        let (name, value) = l.split_once(':')?;
        if name.eq_ignore_ascii_case("host") {
            // two is malformed (RFC 9112 3.2)
            if host.replace(value.trim().to_owned()).is_some() {
                return None;
            }
        } else if name.eq_ignore_ascii_case("range") {
            range = Some(value.trim().to_owned());
        }
    }
    Some(Request {
        method: method.to_owned(),
        target: target.to_owned(),
        host,
        range,
    })
}

async fn serve(inner: Arc<Inner>, mut conn: TcpStream) {
    let read = tokio::time::timeout(inner.limits.head_timeout, read_head(&mut conn)).await;
    let req = match read {
        Ok(Head::Ok(req)) => req,
        Ok(Head::Malformed) => return empty(&mut conn, "400 Bad Request", &[]).await,
        Ok(Head::TooLarge) => {
            return empty(&mut conn, "431 Request Header Fields Too Large", &[]).await;
        }
        // stuck or gone: no reply
        Ok(Head::Gone) | Err(_) => return,
    };
    // DNS rebinding: a page on another name that resolves to 127.0.0.1 can reach this port, but
    // its requests carry its own name
    let port = conn.local_addr().map_or(0, |a| a.port());
    match req.host.as_deref() {
        None => return empty(&mut conn, "400 Bad Request", &[]).await,
        Some(host) if !is_our_host(host, port) => {
            return empty(&mut conn, "421 Misdirected Request", &[]).await;
        }
        Some(_) => {}
    }
    let head_only = match req.method.as_str() {
        "GET" => false,
        "HEAD" => true,
        _ => {
            let allow = [("Allow", "GET, HEAD".to_owned())];
            return empty(&mut conn, "405 Method Not Allowed", &allow).await;
        }
    };
    let Some((grant, mut file, size)) = open(&inner, &req.target).await else {
        return empty(&mut conn, "404 Not Found", &[]).await;
    };
    let kind = ("Content-Type", content_type(&grant.path).to_owned());
    let ranges = ("Accept-Ranges", "bytes".to_owned());
    let (status, start, len, mut headers) = match range(req.range.as_deref(), size) {
        Range::Whole => ("200 OK", 0, size, vec![]),
        Range::Part(a, b) => (
            "206 Partial Content",
            a,
            b - a + 1,
            vec![("Content-Range", format!("bytes {a}-{b}/{size}"))],
        ),
        Range::Unsatisfiable => {
            let at = [("Content-Range", format!("bytes */{size}")), ranges];
            return empty(&mut conn, "416 Range Not Satisfiable", &at).await;
        }
    };
    headers.extend([kind, ranges, ("Content-Length", len.to_string())]);
    let write_timeout = inner.limits.write_timeout;
    let head = head(status, &headers);
    let sent = tokio::time::timeout(write_timeout, conn.write_all(head.as_bytes())).await;
    if !matches!(sent, Ok(Ok(()))) || head_only {
        let _ = conn.shutdown().await;
        return;
    }
    if file.seek(std::io::SeekFrom::Start(start)).await.is_err() {
        return;
    }
    // Each write has `write_timeout`, and revoking ends the copy even mid-write (a client that
    // stops reading would otherwise hold it in `write_all`): either way the connection is dropped.
    let body = async {
        let mut left = len;
        let mut buf = vec![0u8; CHUNK];
        while left > 0 {
            let want = buf.len().min(usize::try_from(left).unwrap_or(usize::MAX));
            let n = match file.read(&mut buf[..want]).await {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            let wrote = tokio::time::timeout(write_timeout, conn.write_all(&buf[..n])).await;
            if !matches!(wrote, Ok(Ok(()))) {
                return false;
            }
            left -= n as u64;
        }
        true
    };
    let mut revoked = grant.revoked.subscribe();
    let finished = tokio::select! {
        finished = body => finished,
        _ = revoked.wait_for(|r| *r) => false,
    };
    if finished {
        let _ = conn.shutdown().await;
    }
}

/// `127.0.0.1:<port>` or `localhost:<port>`, with this server's port.
fn is_our_host(host: &str, port: u16) -> bool {
    host.rsplit_once(':').is_some_and(|(name, p)| {
        (name == "127.0.0.1" || name.eq_ignore_ascii_case("localhost")) && p == port.to_string()
    })
}

/// The file `/<token>` names, if the token is live and the webview may still load the file.
async fn open(inner: &Inner, target: &str) -> Option<(Grant, tokio::fs::File, u64)> {
    let token = target
        .strip_prefix('/')
        .filter(|t| t.len() == 64 && t.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))?;
    let grant = inner.files.lock().unwrap().by_token.get(token).cloned()?;
    // the scope is asked outside our lock (it takes its own)
    if !(inner.allowed)(&grant.path) {
        return None;
    }
    let file = tokio::fs::File::open(&grant.path).await.ok()?;
    let meta = file.metadata().await.ok()?;
    meta.is_file().then_some((grant, file, meta.len()))
}

#[derive(Debug, PartialEq)]
enum Range {
    Whole,
    /// First and last byte, inclusive.
    Part(u64, u64),
    Unsatisfiable,
}

/// The `Range` header against a file of `size` bytes. A malformed one, or several ranges, is
/// ignored (the whole file).
fn range(header: Option<&str>, size: u64) -> Range {
    let Some(spec) = header.and_then(|h| {
        let (unit, spec) = h.split_once('=')?;
        unit.trim()
            .eq_ignore_ascii_case("bytes")
            .then_some(spec.trim())
    }) else {
        return Range::Whole;
    };
    if spec.contains(',') {
        return Range::Whole;
    }
    let Some((a, b)) = spec.split_once('-') else {
        return Range::Whole;
    };
    let (a, b) = (a.trim(), b.trim());
    if a.is_empty() {
        // the last n bytes
        return match number(b) {
            Some(0) => Range::Unsatisfiable,
            Some(_) if size == 0 => Range::Unsatisfiable,
            Some(n) => Range::Part(size - n.min(size), size - 1),
            None => Range::Whole,
        };
    }
    let Some(first) = number(a) else {
        return Range::Whole;
    };
    let last = match b {
        "" => None,
        b => match number(b) {
            Some(last) if last >= first => Some(last),
            _ => return Range::Whole,
        },
    };
    if first >= size {
        return Range::Unsatisfiable;
    }
    Range::Part(first, last.map_or(size - 1, |l| l.min(size - 1)))
}

/// A range bound: digits only (`u64::from_str` would also take a leading `+`).
fn number(s: &str) -> Option<u64> {
    (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
        .then(|| s.parse().ok())
        .flatten()
}

fn head(status: &str, headers: &[(&str, String)]) -> String {
    let mut out = format!("HTTP/1.1 {status}\r\n");
    for (name, value) in headers {
        out.push_str(&format!("{name}: {value}\r\n"));
    }
    out.push_str("Cache-Control: no-store\r\nConnection: close\r\n\r\n");
    out
}

/// A reply with no body, and the connection closed.
async fn empty(conn: &mut TcpStream, status: &str, headers: &[(&str, String)]) {
    let mut headers = headers.to_vec();
    headers.push(("Content-Length", "0".to_owned()));
    let _ = conn.write_all(head(status, &headers).as_bytes()).await;
    let _ = conn.shutdown().await;
    // Closing with the rest of the request unread resets the connection, which can throw away
    // the reply before the client reads it: take what's left first (a little, briefly).
    let mut sink = [0u8; 4096];
    let drain = async {
        for _ in 0..16 {
            if matches!(conn.read(&mut sink).await, Ok(0) | Err(_)) {
                break;
            }
        }
    };
    let _ = tokio::time::timeout(Duration::from_secs(1), drain).await;
}

/// What a file is served as, by its extension.
pub fn content_type(path: &Path) -> &'static str {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase());
    match ext.as_deref() {
        Some("mp4" | "m4v") => "video/mp4",
        Some("webm") => "video/webm",
        Some("mkv") => "video/x-matroska",
        Some("mov") => "video/quicktime",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tokio::io::AsyncReadExt;

    const BYTES: usize = 200_000;

    /// A video file of BYTES bytes (byte i is i % 251, so any slice is recognisable).
    fn video(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        let data: Vec<u8> = (0..BYTES).map(|i| (i % 251) as u8).collect();
        std::fs::File::create(&path)
            .unwrap()
            .write_all(&data)
            .unwrap();
        path
    }

    fn slice(a: usize, b: usize) -> Vec<u8> {
        (a..=b).map(|i| (i % 251) as u8).collect()
    }

    /// The webview may load everything in `allowed`.
    fn videos(allowed: Arc<Mutex<Vec<PathBuf>>>) -> Videos {
        videos_with(
            allowed,
            Limits {
                head_timeout: Duration::from_millis(300),
                ..LIMITS
            },
        )
    }

    fn videos_with(allowed: Arc<Mutex<Vec<PathBuf>>>, limits: Limits) -> Videos {
        Videos::with_limits(
            Box::new(move |p| allowed.lock().unwrap().iter().any(|a| a == p)),
            limits,
        )
    }

    struct Reply {
        status: u16,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    }

    impl Reply {
        fn header(&self, name: &str) -> Option<&str> {
            self.headers
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.as_str())
        }
    }

    /// `http://127.0.0.1:<port>/<token>` → (`127.0.0.1:<port>`, `/<token>`)
    fn split(url: &str) -> (String, String) {
        let rest = url.strip_prefix("http://").unwrap();
        let (host, path) = rest.split_once('/').unwrap();
        (host.to_owned(), format!("/{path}"))
    }

    /// Send `raw` as is and read the whole reply (the server closes after one).
    async fn send(host: &str, raw: &[u8]) -> Reply {
        let mut conn = TcpStream::connect(host).await.unwrap();
        conn.write_all(raw).await.unwrap();
        let mut all = Vec::new();
        conn.read_to_end(&mut all).await.unwrap();
        let end = all.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        let head = String::from_utf8(all[..end].to_vec()).unwrap();
        let mut lines = head.split("\r\n");
        let status = lines.next().unwrap().split(' ').nth(1).unwrap();
        Reply {
            status: status.parse().unwrap(),
            headers: lines
                .map(|l| {
                    let (n, v) = l.split_once(':').unwrap();
                    (n.to_owned(), v.trim().to_owned())
                })
                .collect(),
            body: all[end + 4..].to_vec(),
        }
    }

    async fn request(url: &str, method: &str, range: Option<&str>) -> Reply {
        let (host, path) = split(url);
        let range = range.map(|r| format!("Range: {r}\r\n")).unwrap_or_default();
        let raw = format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\n{range}\r\n");
        send(&host, raw.as_bytes()).await
    }

    /// The editor source the tests' tokens are for.
    const OWNER: &str = "a";

    /// Start the server and grant `path` to `OWNER`.
    async fn url_for(v: &Videos, path: &Path) -> Option<String> {
        assert!(v.start().await);
        v.grant(OWNER, path)
    }

    async fn setup() -> (tempfile::TempDir, PathBuf, Videos, String) {
        let dir = tempfile::tempdir().unwrap();
        let path = video(dir.path(), "replay.mp4");
        let v = videos(Arc::new(Mutex::new(vec![path.clone()])));
        let url = url_for(&v, &path).await.unwrap();
        (dir, path, v, url)
    }

    #[tokio::test]
    async fn the_url_is_loopback_with_a_64_hex_digit_token_kept_per_path() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (video(dir.path(), "a.mp4"), video(dir.path(), "b.mp4"));
        let v = videos(Arc::new(Mutex::new(vec![a.clone(), b.clone()])));
        let url = url_for(&v, &a).await.unwrap();
        let (host, path) = split(&url);
        assert!(host.starts_with("127.0.0.1:"));
        let token = path.strip_prefix('/').unwrap();
        assert!(token.len() == 64 && token.bytes().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(url_for(&v, &a).await.unwrap(), url);
        let other = url_for(&v, &b).await.unwrap();
        assert_ne!(other, url);
        assert_eq!(split(&other).0, host, "one server");
    }

    #[tokio::test]
    async fn each_owner_has_its_own_token_and_revoking_ends_only_its_own() {
        let (_dir, path, v, url) = setup().await;
        let theirs = v.grant("b", &path).unwrap();
        assert_ne!(theirs, url);
        v.revoke("b");
        assert_eq!(request(&theirs, "GET", None).await.status, 404);
        assert_eq!(request(&url, "GET", None).await.status, 200);
    }

    #[tokio::test]
    async fn no_token_before_the_server_starts() {
        let dir = tempfile::tempdir().unwrap();
        let path = video(dir.path(), "replay.mp4");
        let v = videos(Arc::new(Mutex::new(vec![path.clone()])));
        assert_eq!(v.grant(OWNER, &path), None);
    }

    #[tokio::test]
    async fn a_path_the_webview_may_not_load_gets_no_url() {
        let dir = tempfile::tempdir().unwrap();
        let path = video(dir.path(), "replay.mp4");
        let v = videos(Arc::new(Mutex::new(vec![])));
        assert_eq!(url_for(&v, &path).await, None);
    }

    #[tokio::test]
    async fn get_serves_the_whole_file() {
        let (_dir, _path, _v, url) = setup().await;
        let r = request(&url, "GET", None).await;
        assert_eq!(r.status, 200);
        assert_eq!(r.header("content-length"), Some("200000"));
        assert_eq!(r.header("content-type"), Some("video/mp4"));
        assert_eq!(r.header("accept-ranges"), Some("bytes"));
        assert_eq!(r.header("connection"), Some("close"));
        assert_eq!(r.body, slice(0, BYTES - 1));
    }

    #[tokio::test]
    async fn a_range_gets_206_with_just_those_bytes() {
        let (_dir, _path, _v, url) = setup().await;
        let r = request(&url, "GET", Some("bytes=100-1099")).await;
        assert_eq!(r.status, 206);
        assert_eq!(r.header("content-range"), Some("bytes 100-1099/200000"));
        assert_eq!(r.header("content-length"), Some("1000"));
        assert_eq!(r.header("accept-ranges"), Some("bytes"));
        assert_eq!(r.body, slice(100, 1099));
    }

    #[tokio::test]
    async fn an_open_ended_range_runs_to_the_end() {
        let (_dir, _path, _v, url) = setup().await;
        let r = request(&url, "GET", Some("bytes=150000-")).await;
        assert_eq!(r.status, 206);
        assert_eq!(
            r.header("content-range"),
            Some("bytes 150000-199999/200000")
        );
        assert_eq!(r.header("content-length"), Some("50000"));
        assert_eq!(r.body, slice(150_000, BYTES - 1));
    }

    #[tokio::test]
    async fn a_suffix_range_is_the_last_n_bytes() {
        let (_dir, _path, _v, url) = setup().await;
        let r = request(&url, "GET", Some("bytes=-500")).await;
        assert_eq!(r.status, 206);
        assert_eq!(
            r.header("content-range"),
            Some("bytes 199500-199999/200000")
        );
        assert_eq!(r.body, slice(199_500, BYTES - 1));
        // longer than the file: all of it
        let r = request(&url, "GET", Some("bytes=-900000")).await;
        assert_eq!(r.header("content-range"), Some("bytes 0-199999/200000"));
        assert_eq!(r.body.len(), BYTES);
    }

    #[tokio::test]
    async fn a_range_past_the_end_is_cut_at_the_end() {
        let (_dir, _path, _v, url) = setup().await;
        let r = request(&url, "GET", Some("bytes=199990-300000")).await;
        assert_eq!(r.status, 206);
        assert_eq!(
            r.header("content-range"),
            Some("bytes 199990-199999/200000")
        );
        assert_eq!(r.body, slice(199_990, BYTES - 1));
    }

    #[tokio::test]
    async fn a_range_starting_past_the_end_is_416() {
        let (_dir, _path, _v, url) = setup().await;
        for range in ["bytes=200000-", "bytes=500000-600000", "bytes=-0"] {
            let r = request(&url, "GET", Some(range)).await;
            assert_eq!(r.status, 416, "{range}");
            assert_eq!(r.header("content-range"), Some("bytes */200000"), "{range}");
            assert!(r.body.is_empty(), "{range}");
        }
    }

    #[tokio::test]
    async fn several_ranges_or_a_malformed_one_get_the_whole_file() {
        let (_dir, _path, _v, url) = setup().await;
        for range in [
            "bytes=0-9,20-29",
            "bytes=9-0",
            "bytes=x-y",
            "items=0-9",
            "bytes=",
            // only digits: Rust's u64 parse would take a leading +
            "bytes=+100-1099",
            "bytes=100-+1099",
            "bytes=-+500",
        ] {
            let r = request(&url, "GET", Some(range)).await;
            assert_eq!(r.status, 200, "{range}");
            assert_eq!(r.body.len(), BYTES, "{range}");
        }
    }

    #[tokio::test]
    async fn head_has_the_headers_and_no_body() {
        let (_dir, _path, _v, url) = setup().await;
        let r = request(&url, "HEAD", None).await;
        assert_eq!(r.status, 200);
        assert_eq!(r.header("content-length"), Some("200000"));
        assert!(r.body.is_empty());
        let r = request(&url, "HEAD", Some("bytes=10-19")).await;
        assert_eq!(r.status, 206);
        assert_eq!(r.header("content-range"), Some("bytes 10-19/200000"));
        assert_eq!(r.header("content-length"), Some("10"));
        assert!(r.body.is_empty());
    }

    #[tokio::test]
    async fn other_methods_are_405() {
        let (_dir, _path, _v, url) = setup().await;
        for method in ["POST", "PUT", "DELETE", "OPTIONS"] {
            let r = request(&url, method, None).await;
            assert_eq!(r.status, 405, "{method}");
            assert_eq!(r.header("allow"), Some("GET, HEAD"), "{method}");
            assert!(r.body.is_empty());
        }
    }

    #[tokio::test]
    async fn anything_but_a_known_token_is_404() {
        let (_dir, path, _v, url) = setup().await;
        let (host, token) = split(&url);
        let unknown = format!("/{}", "ab".repeat(32));
        let file = path.to_string_lossy().into_owned();
        for target in [
            "/".to_owned(),
            unknown,
            format!("{token}/"),
            format!("{token}?x=1"),
            token[..20].to_owned(),
            token.to_uppercase(),
            file,
            "/../../etc/passwd".to_owned(),
        ] {
            let raw = format!("GET {target} HTTP/1.1\r\nHost: {host}\r\n\r\n");
            let r = send(&host, raw.as_bytes()).await;
            assert_eq!(r.status, 404, "{target}");
            assert!(r.body.is_empty(), "{target}");
        }
    }

    #[tokio::test]
    async fn a_revoked_path_is_404_and_gets_a_new_token_next_time() {
        let (_dir, path, v, url) = setup().await;
        v.revoke(OWNER);
        assert_eq!(request(&url, "GET", None).await.status, 404);
        let again = url_for(&v, &path).await.unwrap();
        assert_ne!(again, url);
        assert_eq!(request(&again, "GET", None).await.status, 200);
    }

    #[tokio::test]
    async fn revoke_all_ends_every_token() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (video(dir.path(), "a.mp4"), video(dir.path(), "b.webm"));
        let v = videos(Arc::new(Mutex::new(vec![a.clone(), b.clone()])));
        let (ua, ub) = (
            url_for(&v, &a).await.unwrap(),
            url_for(&v, &b).await.unwrap(),
        );
        v.revoke_all();
        assert_eq!(request(&ua, "GET", None).await.status, 404);
        assert_eq!(request(&ub, "GET", None).await.status, 404);
    }

    #[tokio::test]
    async fn a_path_taken_off_the_scope_is_404_from_then_on() {
        let dir = tempfile::tempdir().unwrap();
        let path = video(dir.path(), "replay.mp4");
        let allowed = Arc::new(Mutex::new(vec![path.clone()]));
        let v = videos(allowed.clone());
        let url = url_for(&v, &path).await.unwrap();
        allowed.lock().unwrap().clear();
        assert_eq!(request(&url, "GET", None).await.status, 404);
        assert_eq!(url_for(&v, &path).await, None);
    }

    #[tokio::test]
    async fn a_deleted_file_is_404() {
        let (_dir, path, _v, url) = setup().await;
        std::fs::remove_file(&path).unwrap();
        assert_eq!(request(&url, "GET", None).await.status, 404);
    }

    #[tokio::test]
    async fn revoking_stops_a_response_in_flight() {
        let dir = tempfile::tempdir().unwrap();
        // big enough that the socket buffers can't take it all before we revoke
        let path = dir.path().join("big.mp4");
        std::fs::write(&path, vec![7u8; 64 * 1024 * 1024]).unwrap();
        let v = videos(Arc::new(Mutex::new(vec![path.clone()])));
        let url = url_for(&v, &path).await.unwrap();
        let (host, target) = split(&url);
        let mut conn = TcpStream::connect(&host).await.unwrap();
        let raw = format!("GET {target} HTTP/1.1\r\nHost: {host}\r\n\r\n");
        conn.write_all(raw.as_bytes()).await.unwrap();
        let mut first = [0u8; 1024];
        let _ = conn.read(&mut first).await.unwrap();
        v.revoke(OWNER);
        let mut rest = Vec::new();
        tokio::time::timeout(Duration::from_secs(10), conn.read_to_end(&mut rest))
            .await
            .expect("the server closed the connection")
            .unwrap();
        assert!(rest.len() < 64 * 1024 * 1024 - 1024);
    }

    #[tokio::test]
    async fn an_empty_file_is_served_empty_and_any_range_is_416() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.mkv");
        std::fs::write(&path, b"").unwrap();
        let v = videos(Arc::new(Mutex::new(vec![path.clone()])));
        let url = url_for(&v, &path).await.unwrap();
        let r = request(&url, "GET", None).await;
        assert_eq!((r.status, r.header("content-length")), (200, Some("0")));
        assert_eq!(r.header("content-type"), Some("video/x-matroska"));
        let r = request(&url, "GET", Some("bytes=0-")).await;
        assert_eq!(
            (r.status, r.header("content-range")),
            (416, Some("bytes */0"))
        );
    }

    #[tokio::test]
    async fn an_oversized_head_is_431() {
        let (_dir, _path, _v, url) = setup().await;
        let (host, target) = split(&url);
        let raw = format!(
            "GET {target} HTTP/1.1\r\nHost: {host}\r\nX-Pad: {}\r\n\r\n",
            "a".repeat(MAX_HEAD)
        );
        let r = send(&host, raw.as_bytes()).await;
        assert_eq!(r.status, 431);
        assert!(r.body.is_empty());
    }

    /// DNS rebinding: a page on some other name that resolves to 127.0.0.1 sends its own Host.
    #[tokio::test]
    async fn only_a_loopback_host_with_our_port_is_served() {
        let (_dir, _path, _v, url) = setup().await;
        let (host, target) = split(&url);
        let port = host.rsplit_once(':').unwrap().1.to_owned();
        let ok = format!("localhost:{port}");
        let r = send(
            &host,
            format!("GET {target} HTTP/1.1\r\nhost: {ok}\r\n\r\n").as_bytes(),
        )
        .await;
        assert_eq!(r.status, 200, "{ok}");
        for bad in [
            format!("evil.example:{port}"),
            "127.0.0.1:1".to_owned(),
            "127.0.0.1".to_owned(),
            format!("127.0.0.2:{port}"),
            format!("[::1]:{port}"),
            format!("127.0.0.1:{port}.evil.example"),
            String::new(),
        ] {
            let raw = format!("GET {target} HTTP/1.1\r\nHost: {bad}\r\n\r\n");
            let r = send(&host, raw.as_bytes()).await;
            assert_eq!(r.status, 421, "{bad:?}");
            assert!(r.body.is_empty(), "{bad:?}");
        }
        // none at all, or two
        for raw in [
            format!("GET {target} HTTP/1.1\r\n\r\n"),
            format!("GET {target} HTTP/1.1\r\nHost: {host}\r\nHost: {host}\r\n\r\n"),
        ] {
            assert_eq!(send(&host, raw.as_bytes()).await.status, 400, "{raw:?}");
        }
    }

    #[tokio::test]
    async fn a_malformed_request_is_400() {
        let (_dir, _path, _v, url) = setup().await;
        let (host, target) = split(&url);
        for raw in [
            format!("GET {target}\r\n\r\n"),
            format!("GET {target} SPDY/3\r\n\r\n"),
            "\r\n\r\n".to_owned(),
            format!("GET {target} HTTP/1.1\r\nno colon here\r\n\r\n"),
        ] {
            let r = send(&host, raw.as_bytes()).await;
            assert_eq!(r.status, 400, "{raw:?}");
        }
    }

    #[tokio::test]
    async fn a_client_that_never_finishes_its_head_is_dropped() {
        let (_dir, _path, _v, url) = setup().await;
        let (host, target) = split(&url);
        let mut conn = TcpStream::connect(&host).await.unwrap();
        conn.write_all(format!("GET {target} HTTP/1.1\r\n").as_bytes())
            .await
            .unwrap();
        let mut rest = Vec::new();
        // closed without a reply once the (test's 300 ms) head timeout passes
        tokio::time::timeout(Duration::from_secs(5), conn.read_to_end(&mut rest))
            .await
            .expect("the server dropped the connection")
            .unwrap();
        assert!(rest.is_empty());
    }

    /// Every connection being served is back (its permit returned), within a few seconds.
    async fn all_let_go(v: &Videos) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while v.inner.conns.available_permits() < v.inner.limits.max_conns {
            assert!(
                tokio::time::Instant::now() < deadline,
                "a connection is still being served"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// A GET of a file far bigger than the socket buffers, from a client that then reads nothing:
    /// the server is soon stuck writing.
    async fn stalled(v: &Videos, path: &Path) -> TcpStream {
        let url = url_for(v, path).await.unwrap();
        let (host, target) = split(&url);
        let mut conn = TcpStream::connect(&host).await.unwrap();
        let raw = format!("GET {target} HTTP/1.1\r\nHost: {host}\r\n\r\n");
        conn.write_all(raw.as_bytes()).await.unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(
            v.inner.conns.available_permits(),
            v.inner.limits.max_conns - 1,
            "still being served"
        );
        conn
    }

    fn big(dir: &Path) -> PathBuf {
        let path = dir.join("big.mp4");
        std::fs::write(&path, vec![7u8; 64 * 1024 * 1024]).unwrap();
        path
    }

    #[tokio::test]
    async fn revoking_ends_a_response_stuck_on_a_client_that_stopped_reading() {
        let dir = tempfile::tempdir().unwrap();
        let path = big(dir.path());
        let v = videos(Arc::new(Mutex::new(vec![path.clone()])));
        for revoke in [&(|v: &Videos| v.revoke(OWNER)) as &dyn Fn(&Videos), &|v| {
            v.revoke_all()
        }] {
            let _conn = stalled(&v, &path).await;
            revoke(&v);
            all_let_go(&v).await;
        }
    }

    #[tokio::test]
    async fn a_client_that_stops_reading_is_dropped_after_the_write_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let path = big(dir.path());
        let limits = Limits {
            write_timeout: Duration::from_millis(500),
            ..LIMITS
        };
        let v = videos_with(Arc::new(Mutex::new(vec![path.clone()])), limits);
        let _conn = stalled(&v, &path).await;
        all_let_go(&v).await;
    }

    #[tokio::test]
    async fn connections_past_the_cap_are_closed_and_the_rest_still_served() {
        let dir = tempfile::tempdir().unwrap();
        let path = video(dir.path(), "replay.mp4");
        let limits = Limits {
            head_timeout: Duration::from_secs(30),
            max_conns: 2,
            ..LIMITS
        };
        let v = videos_with(Arc::new(Mutex::new(vec![path.clone()])), limits);
        let url = url_for(&v, &path).await.unwrap();
        let (host, _) = split(&url);
        // two that never send a head hold both places
        let idle = [
            TcpStream::connect(&host).await.unwrap(),
            TcpStream::connect(&host).await.unwrap(),
        ];
        let mut extra = TcpStream::connect(&host).await.unwrap();
        let mut rest = Vec::new();
        tokio::time::timeout(Duration::from_secs(5), extra.read_to_end(&mut rest))
            .await
            .expect("the one past the cap is closed at once")
            .ok();
        assert!(rest.is_empty(), "no reply");
        drop(idle);
        all_let_go(&v).await;
        assert_eq!(request(&url, "GET", None).await.status, 200);
    }

    #[test]
    fn content_types_follow_the_extension() {
        for (name, want) in [
            ("a.mp4", "video/mp4"),
            ("a.M4V", "video/mp4"),
            ("a.webm", "video/webm"),
            ("a.mkv", "video/x-matroska"),
            ("a.mov", "video/quicktime"),
            ("a.avi", "application/octet-stream"),
            ("noext", "application/octet-stream"),
        ] {
            assert_eq!(content_type(Path::new(name)), want, "{name}");
        }
    }
}
