//! Local videos for the Linux webview, over loopback HTTP. WebKitGTK's media player (GStreamer)
//! refuses video from the `asset://` scheme (FormatError, nothing loads), though images over it
//! work and the same file plays from `http://`. So on Linux the clip editor's `<video>` gets
//! `http://127.0.0.1:<port>/<token>` (`commands::video_url`); Windows keeps the asset protocol.
//!
//! - A token is 32 random bytes (64 hex digits) naming exactly one file. Tokens are given only for
//!   paths the webview may load through the asset protocol anyway (`allowed`, the asset scope),
//!   and that is checked again on every request: a forbidden path stops working at once.
//! - `revoke` (an editor source closed) and `revoke_all` (sign-out) end tokens, and stop a
//!   response in flight. Nothing from the request ever names a file: only the token map does.
//! - Just `GET` and `HEAD`, one request per connection (`Connection: close`). The head is capped
//!   at 8 KB and must arrive within 10 s; peers that aren't loopback are dropped.
//! - Ranges: `bytes=a-b`, `bytes=a-` and `bytes=-n` get a 206, one past the end a 416. Several
//!   ranges at once (GStreamer never asks) get the whole file (200): ignoring Range is always allowed.

use std::{
    collections::HashMap,
    net::{Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::OnceCell,
};

/// The request line and headers, at most.
const MAX_HEAD: usize = 8 * 1024;
/// How long a client has to send them.
const HEAD_TIMEOUT: Duration = Duration::from_secs(10);
const CHUNK: usize = 64 * 1024;

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
    head_timeout: Duration,
}

#[derive(Default)]
struct Files {
    by_token: HashMap<String, Grant>,
    by_path: HashMap<PathBuf, String>,
}

#[derive(Clone)]
struct Grant {
    path: PathBuf,
    /// Cleared when revoked: a response in flight stops at its next chunk.
    live: Arc<AtomicBool>,
}

impl Videos {
    pub fn new(allowed: Allowed) -> Self {
        Self::with_head_timeout(allowed, HEAD_TIMEOUT)
    }

    fn with_head_timeout(allowed: Allowed, head_timeout: Duration) -> Self {
        Self {
            inner: Arc::new(Inner {
                allowed,
                files: Mutex::default(),
                addr: OnceCell::new(),
                head_timeout,
            }),
        }
    }

    /// `http://127.0.0.1:<port>/<token>` for `path` (the same token while it lasts), starting the
    /// server if needed. `None` if the webview may not load `path`, or the server can't start.
    pub async fn url(&self, path: &Path) -> Option<String> {
        if !(self.inner.allowed)(path) {
            return None;
        }
        let addr = match self.inner.addr.get_or_try_init(|| start(&self.inner)).await {
            Ok(addr) => *addr,
            Err(e) => {
                tracing::warn!(error = %e, "video server unavailable");
                return None;
            }
        };
        let mut files = self.inner.files.lock().unwrap();
        let token = match files.by_path.get(path) {
            Some(t) => t.clone(),
            None => {
                let t = hex(&rand::random::<[u8; 32]>());
                let grant = Grant {
                    path: path.to_path_buf(),
                    live: Arc::new(AtomicBool::new(true)),
                };
                files.by_token.insert(t.clone(), grant);
                files.by_path.insert(path.to_path_buf(), t.clone());
                t
            }
        };
        Some(format!("http://{addr}/{token}"))
    }

    /// `path`'s token stops working (a request for it is a 404 from now on).
    pub fn revoke(&self, path: &Path) {
        let mut files = self.inner.files.lock().unwrap();
        if let Some(t) = files.by_path.remove(path)
            && let Some(g) = files.by_token.remove(&t)
        {
            g.live.store(false, Ordering::SeqCst);
        }
    }

    #[cfg(test)]
    pub(crate) fn has_token(&self, path: &Path) -> bool {
        self.inner.files.lock().unwrap().by_path.contains_key(path)
    }

    /// Every token stops working (sign-out).
    pub fn revoke_all(&self) {
        let mut files = self.inner.files.lock().unwrap();
        files.by_path.clear();
        for (_, g) in files.by_token.drain() {
            g.live.store(false, Ordering::SeqCst);
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Bind `127.0.0.1:0` and accept until the `Videos` is dropped.
async fn start(inner: &Arc<Inner>) -> std::io::Result<SocketAddr> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    let addr = listener.local_addr()?;
    let inner = Arc::downgrade(inner);
    tokio::spawn(accept(listener, inner));
    Ok(addr)
}

async fn accept(listener: TcpListener, inner: Weak<Inner>) {
    loop {
        let Ok((conn, peer)) = listener.accept().await else {
            continue;
        };
        let Some(inner) = inner.upgrade() else {
            return;
        };
        // bound to loopback anyway
        if !peer.ip().is_loopback() {
            continue;
        }
        tokio::spawn(serve(inner, conn));
    }
}

/// What a client asked for, once its head has been read and parsed.
struct Request {
    method: String,
    target: String,
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
    let mut range = None;
    for l in lines {
        let (name, value) = l.split_once(':')?;
        if name.eq_ignore_ascii_case("range") {
            range = Some(value.trim().to_owned());
        }
    }
    Some(Request {
        method: method.to_owned(),
        target: target.to_owned(),
        range,
    })
}

async fn serve(inner: Arc<Inner>, mut conn: TcpStream) {
    let read = tokio::time::timeout(inner.head_timeout, read_head(&mut conn)).await;
    let req = match read {
        Ok(Head::Ok(req)) => req,
        Ok(Head::Malformed) => return empty(&mut conn, "400 Bad Request", &[]).await,
        Ok(Head::TooLarge) => {
            return empty(&mut conn, "431 Request Header Fields Too Large", &[]).await;
        }
        // stuck or gone: no reply
        Ok(Head::Gone) | Err(_) => return,
    };
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
    if conn
        .write_all(head(status, &headers).as_bytes())
        .await
        .is_err()
        || head_only
    {
        let _ = conn.shutdown().await;
        return;
    }
    if file.seek(std::io::SeekFrom::Start(start)).await.is_err() {
        return;
    }
    let mut left = len;
    let mut buf = vec![0u8; CHUNK];
    while left > 0 && grant.live.load(Ordering::SeqCst) {
        let want = buf.len().min(usize::try_from(left).unwrap_or(usize::MAX));
        let n = match file.read(&mut buf[..want]).await {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        if conn.write_all(&buf[..n]).await.is_err() {
            return;
        }
        left -= n as u64;
    }
    let _ = conn.shutdown().await;
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
        return match b.parse::<u64>() {
            Ok(0) => Range::Unsatisfiable,
            Ok(_) if size == 0 => Range::Unsatisfiable,
            Ok(n) => Range::Part(size - n.min(size), size - 1),
            Err(_) => Range::Whole,
        };
    }
    let Ok(first) = a.parse::<u64>() else {
        return Range::Whole;
    };
    let last = match b {
        "" => None,
        b => match b.parse::<u64>() {
            Ok(last) if last >= first => Some(last),
            _ => return Range::Whole,
        },
    };
    if first >= size {
        return Range::Unsatisfiable;
    }
    Range::Part(first, last.map_or(size - 1, |l| l.min(size - 1)))
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
        Videos::with_head_timeout(
            Box::new(move |p| allowed.lock().unwrap().iter().any(|a| a == p)),
            Duration::from_millis(300),
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

    async fn setup() -> (tempfile::TempDir, PathBuf, Videos, String) {
        let dir = tempfile::tempdir().unwrap();
        let path = video(dir.path(), "replay.mp4");
        let v = videos(Arc::new(Mutex::new(vec![path.clone()])));
        let url = v.url(&path).await.unwrap();
        (dir, path, v, url)
    }

    #[tokio::test]
    async fn the_url_is_loopback_with_a_64_hex_digit_token_kept_per_path() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (video(dir.path(), "a.mp4"), video(dir.path(), "b.mp4"));
        let v = videos(Arc::new(Mutex::new(vec![a.clone(), b.clone()])));
        let url = v.url(&a).await.unwrap();
        let (host, path) = split(&url);
        assert!(host.starts_with("127.0.0.1:"));
        let token = path.strip_prefix('/').unwrap();
        assert!(token.len() == 64 && token.bytes().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(v.url(&a).await.unwrap(), url);
        let other = v.url(&b).await.unwrap();
        assert_ne!(other, url);
        assert_eq!(split(&other).0, host, "one server");
    }

    #[tokio::test]
    async fn a_path_the_webview_may_not_load_gets_no_url() {
        let dir = tempfile::tempdir().unwrap();
        let path = video(dir.path(), "replay.mp4");
        let v = videos(Arc::new(Mutex::new(vec![])));
        assert_eq!(v.url(&path).await, None);
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
        v.revoke(&path);
        assert_eq!(request(&url, "GET", None).await.status, 404);
        let again = v.url(&path).await.unwrap();
        assert_ne!(again, url);
        assert_eq!(request(&again, "GET", None).await.status, 200);
    }

    #[tokio::test]
    async fn revoke_all_ends_every_token() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (video(dir.path(), "a.mp4"), video(dir.path(), "b.webm"));
        let v = videos(Arc::new(Mutex::new(vec![a.clone(), b.clone()])));
        let (ua, ub) = (v.url(&a).await.unwrap(), v.url(&b).await.unwrap());
        v.revoke_all();
        assert_eq!(request(&ua, "GET", None).await.status, 404);
        assert_eq!(request(&ub, "GET", None).await.status, 404);
    }

    #[tokio::test]
    async fn a_path_taken_off_the_scope_stops_working_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = video(dir.path(), "replay.mp4");
        let allowed = Arc::new(Mutex::new(vec![path.clone()]));
        let v = videos(allowed.clone());
        let url = v.url(&path).await.unwrap();
        allowed.lock().unwrap().clear();
        assert_eq!(request(&url, "GET", None).await.status, 404);
        assert_eq!(v.url(&path).await, None);
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
        let url = v.url(&path).await.unwrap();
        let (host, target) = split(&url);
        let mut conn = TcpStream::connect(&host).await.unwrap();
        let raw = format!("GET {target} HTTP/1.1\r\nHost: {host}\r\n\r\n");
        conn.write_all(raw.as_bytes()).await.unwrap();
        let mut first = [0u8; 1024];
        let _ = conn.read(&mut first).await.unwrap();
        v.revoke(&path);
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
        let url = v.url(&path).await.unwrap();
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
