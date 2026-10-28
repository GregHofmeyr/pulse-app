//! An in-process stand-in for Cloudflare R2 (S3, path style): PUT, GET with `Range`, HEAD and
//! DELETE on `/{bucket}/{key}`, in memory or on disk. Every request must carry a valid SigV4
//! query presign. The verifier below is written from the AWS spec on `hmac` + `sha2`, not on
//! `rusty-s3`, so tests that presign with `rusty-s3` cross-check the signer.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use axum::response::Response;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

pub const FAKE_BUCKET: &str = "pulse-clips-test";
pub const FAKE_KEY_ID: &str = "FAKEKEYID-not-a-real-key";
pub const FAKE_SECRET: &str = "fake-secret-not-a-real-secret-0000";
/// R2's region name; the credential scope must say it.
const REGION: &str = "auto";

#[derive(Clone, Debug)]
pub enum FakeStore {
    Memory,
    /// One file per object under this directory (`<hex of key>` plus `<hex>.type`), so it
    /// survives a restart.
    Dir(PathBuf),
}

#[derive(Default)]
struct Inner {
    mem: HashMap<String, (Vec<u8>, String)>,
    requests: Vec<(Method, String)>,
}

struct Shared {
    store: FakeStore,
    inner: Mutex<Inner>,
    down: AtomicBool,
    delay_ms: std::sync::atomic::AtomicU64,
    /// Unix seconds to treat as "now"; 0 means the real clock.
    now: AtomicI64,
}

pub struct FakeS3 {
    addr: SocketAddr,
    shared: Arc<Shared>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for FakeS3 {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl FakeS3 {
    /// Listen on an ephemeral 127.0.0.1 port.
    pub async fn start(store: FakeStore) -> FakeS3 {
        Self::start_on("127.0.0.1:0".parse().unwrap(), store).await
    }

    pub async fn start_on(addr: SocketAddr, store: FakeStore) -> FakeS3 {
        if let FakeStore::Dir(d) = &store {
            std::fs::create_dir_all(d).unwrap();
        }
        let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let shared = Arc::new(Shared {
            store,
            inner: Mutex::default(),
            down: AtomicBool::new(false),
            delay_ms: std::sync::atomic::AtomicU64::new(0),
            now: AtomicI64::new(0),
        });
        let app = axum::Router::new()
            .fallback(handle)
            .with_state(shared.clone());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        FakeS3 { addr, shared, task }
    }

    /// `http://127.0.0.1:<port>`: the endpoint to hand to a path-style S3 client.
    pub fn endpoint(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// The body and content type stored under `key`.
    pub fn object(&self, key: &str) -> Option<(Vec<u8>, String)> {
        self.shared.get(key)
    }

    /// Store an object directly, without a signature.
    pub fn put_raw(&self, key: &str, bytes: Vec<u8>) {
        self.shared.put(key, bytes, "application/octet-stream");
    }

    pub fn keys(&self) -> Vec<String> {
        self.shared.keys()
    }

    /// Every request that passed the signature check (method, object key), oldest first,
    /// whatever its outcome (a GET of a missing key is in here too).
    pub fn requests(&self) -> Vec<(Method, String)> {
        self.shared.inner.lock().unwrap().requests.clone()
    }

    /// While down, every request is answered 503 (broken storage).
    pub fn set_down(&self, down: bool) {
        self.shared.down.store(down, Ordering::SeqCst);
    }

    /// Hold every request this long before answering (slow storage).
    pub fn set_delay(&self, d: std::time::Duration) {
        self.shared
            .delay_ms
            .store(d.as_millis() as u64, Ordering::SeqCst);
    }

    /// Fix the verifier's clock at this unix time (`None`: the real clock again).
    pub fn set_now(&self, unix: Option<i64>) {
        self.shared.now.store(unix.unwrap_or(0), Ordering::SeqCst);
    }
}

impl Shared {
    fn now(&self) -> i64 {
        match self.now.load(Ordering::SeqCst) {
            0 => chrono::Utc::now().timestamp(),
            t => t,
        }
    }

    fn file(&self, dir: &std::path::Path, key: &str) -> PathBuf {
        dir.join(hex::encode(key))
    }

    fn put(&self, key: &str, bytes: Vec<u8>, content_type: &str) {
        match &self.store {
            FakeStore::Memory => {
                self.inner
                    .lock()
                    .unwrap()
                    .mem
                    .insert(key.to_string(), (bytes, content_type.to_string()));
            }
            FakeStore::Dir(d) => {
                let f = self.file(d, key);
                std::fs::write(f.with_extension("type"), content_type).unwrap();
                std::fs::write(f, bytes).unwrap();
            }
        }
    }

    fn get(&self, key: &str) -> Option<(Vec<u8>, String)> {
        match &self.store {
            FakeStore::Memory => self.inner.lock().unwrap().mem.get(key).cloned(),
            FakeStore::Dir(d) => {
                let f = self.file(d, key);
                let bytes = std::fs::read(&f).ok()?;
                let ct = std::fs::read_to_string(f.with_extension("type")).ok()?;
                Some((bytes, ct))
            }
        }
    }

    fn delete(&self, key: &str) {
        match &self.store {
            FakeStore::Memory => {
                self.inner.lock().unwrap().mem.remove(key);
            }
            FakeStore::Dir(d) => {
                let f = self.file(d, key);
                let _ = std::fs::remove_file(f.with_extension("type"));
                let _ = std::fs::remove_file(f);
            }
        }
    }

    fn keys(&self) -> Vec<String> {
        let mut keys: Vec<String> = match &self.store {
            FakeStore::Memory => self.inner.lock().unwrap().mem.keys().cloned().collect(),
            FakeStore::Dir(d) => std::fs::read_dir(d)
                .unwrap()
                .filter_map(|e| {
                    let name = e.ok()?.file_name().into_string().ok()?;
                    String::from_utf8(hex::decode(name).ok()?).ok()
                })
                .collect(),
        };
        keys.sort();
        keys
    }
}

fn xml_error(status: StatusCode, code: &str, message: &str) -> Response {
    let body = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Error><Code>{code}</Code><Message>{message}</Message></Error>"
    );
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/xml")
        .body(Body::from(body))
        .unwrap()
}

fn empty(status: StatusCode) -> Response {
    Response::builder()
        .status(status)
        .body(Body::empty())
        .unwrap()
}

async fn handle(State(s): State<Arc<Shared>>, req: Request<Body>) -> Response {
    let delay = s.delay_ms.load(Ordering::SeqCst);
    if delay > 0 {
        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
    }
    if s.down.load(Ordering::SeqCst) {
        return xml_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "ServiceUnavailable",
            "down",
        );
    }
    let (parts, body) = req.into_parts();
    let mut headers: Vec<(String, String)> = Vec::new();
    for (name, value) in &parts.headers {
        let value = String::from_utf8_lossy(value.as_bytes()).into_owned();
        match headers.iter_mut().find(|(n, _)| n == name.as_str()) {
            Some((_, v)) => {
                v.push(',');
                v.push_str(&value);
            }
            None => headers.push((name.as_str().to_string(), value)),
        }
    }
    let creds = Creds {
        key_id: FAKE_KEY_ID,
        secret: FAKE_SECRET,
        region: REGION,
    };
    let query = parts.uri.query().unwrap_or("");
    if let Err((code, msg)) = verify(
        parts.method.as_str(),
        parts.uri.path(),
        query,
        &headers,
        &creds,
        s.now(),
    ) {
        return xml_error(StatusCode::FORBIDDEN, code, msg);
    }

    let path = percent_decode(parts.uri.path());
    let Some(key) = path
        .strip_prefix('/')
        .and_then(|p| p.strip_prefix(FAKE_BUCKET))
        .and_then(|p| p.strip_prefix('/'))
        .filter(|k| !k.is_empty())
    else {
        return xml_error(StatusCode::NOT_FOUND, "NoSuchBucket", "unknown bucket");
    };
    s.inner
        .lock()
        .unwrap()
        .requests
        .push((parts.method.clone(), key.to_string()));

    match parts.method {
        Method::PUT => {
            let Ok(bytes) = axum::body::to_bytes(body, usize::MAX).await else {
                return xml_error(StatusCode::BAD_REQUEST, "IncompleteBody", "body");
            };
            let ct = header_str(&parts.headers, header::CONTENT_TYPE)
                .unwrap_or("application/octet-stream");
            s.put(key, bytes.to_vec(), ct);
            empty(StatusCode::OK)
        }
        Method::GET | Method::HEAD => {
            let Some((bytes, ct)) = s.get(key) else {
                return xml_error(StatusCode::NOT_FOUND, "NoSuchKey", "no such key");
            };
            let len = bytes.len();
            let mut res = Response::builder()
                .header(header::ACCEPT_RANGES, "bytes")
                .header(header::CONTENT_TYPE, ct);
            let (status, slice) = match header_str(&parts.headers, header::RANGE) {
                None => (StatusCode::OK, 0..len),
                Some(r) => match parse_range(r, len) {
                    Some(range) => {
                        res = res.header(
                            header::CONTENT_RANGE,
                            format!("bytes {}-{}/{len}", range.start, range.end - 1),
                        );
                        (StatusCode::PARTIAL_CONTENT, range)
                    }
                    None => {
                        return xml_error(
                            StatusCode::RANGE_NOT_SATISFIABLE,
                            "InvalidRange",
                            "range",
                        );
                    }
                },
            };
            let res = res
                .status(status)
                .header(header::CONTENT_LENGTH, slice.len());
            if parts.method == Method::HEAD {
                res.body(Body::empty()).unwrap()
            } else {
                res.body(Body::from(bytes[slice].to_vec())).unwrap()
            }
        }
        Method::DELETE => {
            s.delete(key);
            empty(StatusCode::NO_CONTENT)
        }
        _ => xml_error(StatusCode::METHOD_NOT_ALLOWED, "MethodNotAllowed", "method"),
    }
}

fn header_str(h: &HeaderMap, name: header::HeaderName) -> Option<&str> {
    h.get(name)?.to_str().ok()
}

/// `bytes=a-b` or `bytes=a-` against an object of `len` bytes, as a half-open range.
fn parse_range(value: &str, len: usize) -> Option<std::ops::Range<usize>> {
    let (a, b) = value.strip_prefix("bytes=")?.split_once('-')?;
    let start: usize = a.parse().ok()?;
    let end = match b {
        "" => len,
        b => b.parse::<usize>().ok()?.checked_add(1)?.min(len),
    };
    (start < len && start < end).then_some(start..end)
}

// ---- SigV4 query-presign verifier ------------------------------------------------------------

pub(super) struct Creds<'a> {
    pub key_id: &'a str,
    pub secret: &'a str,
    pub region: &'a str,
}

type AuthError = (&'static str, &'static str);

fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("hmac takes any key length");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

/// AWS's URI encoding: everything but `A-Za-z0-9-_.~` as `%XX` (and `/` kept for paths).
fn aws_encode(s: &str, keep_slash: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        if b.is_ascii_alphanumeric()
            || matches!(b, b'-' | b'_' | b'.' | b'~')
            || (keep_slash && b == b'/')
        {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            out.push(v);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Check a presigned request as of unix time `now`. `path` and `query` are as sent (encoded);
/// `headers` have lowercase names.
pub(super) fn verify(
    method: &str,
    path: &str,
    query: &str,
    headers: &[(String, String)],
    creds: &Creds,
    now: i64,
) -> Result<(), AuthError> {
    const BAD: AuthError = (
        "AccessDenied",
        "Invalid or missing presigned query parameters",
    );
    let pairs: Vec<(String, String)> = query
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (percent_decode(k), percent_decode(v))
        })
        .collect();
    let param = |name: &str| {
        pairs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    };

    if param("X-Amz-Algorithm") != Some("AWS4-HMAC-SHA256") {
        return Err(BAD);
    }
    let date = param("X-Amz-Date").ok_or(BAD)?;
    let expires: i64 = param("X-Amz-Expires")
        .and_then(|e| e.parse().ok())
        .ok_or(BAD)?;
    // S3 and R2 refuse a lifetime outside 1 second to 7 days.
    if !(1..=604_800).contains(&expires) {
        return Err((
            "AuthorizationQueryParametersError",
            "X-Amz-Expires must be between 1 and 604800 seconds",
        ));
    }
    let signed = param("X-Amz-SignedHeaders").ok_or(BAD)?;
    let given = param("X-Amz-Signature").ok_or(BAD)?;
    let credential = param("X-Amz-Credential").ok_or(BAD)?;

    let day = date.get(..8).ok_or(BAD)?;
    let scope = format!("{day}/{}/s3/aws4_request", creds.region);
    if credential != format!("{}/{scope}", creds.key_id) {
        return Err(("AccessDenied", "Unknown access key or wrong scope"));
    }
    let signed_at = chrono::NaiveDateTime::parse_from_str(date, "%Y%m%dT%H%M%SZ")
        .map_err(|_| BAD)?
        .and_utc()
        .timestamp();
    if now >= signed_at + expires {
        return Err(("AccessDenied", "Request has expired"));
    }
    if signed_at > now + 900 {
        return Err(("AccessDenied", "Request is not valid yet"));
    }

    // Canonical request.
    let mut enc: Vec<(String, String)> = pairs
        .iter()
        .filter(|(k, _)| k != "X-Amz-Signature")
        .map(|(k, v)| (aws_encode(k, false), aws_encode(v, false)))
        .collect();
    enc.sort();
    let canonical_query = enc
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&");
    let mut names: Vec<&str> = signed.split(';').collect();
    names.sort_unstable();
    if !names.contains(&"host") {
        return Err(BAD);
    }
    let mut canonical_headers = String::new();
    for n in &names {
        let (_, v) = headers
            .iter()
            .find(|(h, _)| h == n)
            .ok_or(("SignatureDoesNotMatch", "A signed header is missing"))?;
        let v = v.split_whitespace().collect::<Vec<_>>().join(" ");
        canonical_headers.push_str(&format!("{n}:{v}\n"));
    }
    let canonical = format!(
        "{method}\n{}\n{canonical_query}\n{canonical_headers}\n{}\nUNSIGNED-PAYLOAD",
        aws_encode(&percent_decode(path), true),
        names.join(";"),
    );
    let to_sign = format!(
        "AWS4-HMAC-SHA256\n{date}\n{scope}\n{}",
        hex::encode(Sha256::digest(canonical.as_bytes()))
    );

    let mut key = hmac_sha256(format!("AWS4{}", creds.secret).as_bytes(), day.as_bytes());
    for part in [creds.region, "s3", "aws4_request"] {
        key = hmac_sha256(&key, part.as_bytes());
    }
    let expected = hex::encode(hmac_sha256(&key, to_sign.as_bytes()));
    if expected != given {
        return Err((
            "SignatureDoesNotMatch",
            "The request signature does not match",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use reqwest::StatusCode;
    use rusty_s3::actions::S3Action;
    use rusty_s3::{Bucket, Credentials, UrlStyle};

    fn signer(fake: &FakeS3) -> (Bucket, Credentials) {
        let bucket = Bucket::new(
            fake.endpoint().parse().unwrap(),
            UrlStyle::Path,
            FAKE_BUCKET,
            "auto",
        )
        .unwrap();
        (bucket, Credentials::new(FAKE_KEY_ID, FAKE_SECRET))
    }

    fn now() -> i64 {
        chrono::Utc::now().timestamp()
    }

    #[test]
    fn verifier_accepts_the_aws_documented_example() {
        // https://docs.aws.amazon.com/AmazonS3/latest/API/sigv4-query-string-auth.html
        let query = "X-Amz-Algorithm=AWS4-HMAC-SHA256\
            &X-Amz-Credential=AKIAIOSFODNN7EXAMPLE%2F20130524%2Fus-east-1%2Fs3%2Faws4_request\
            &X-Amz-Date=20130524T000000Z&X-Amz-Expires=86400&X-Amz-SignedHeaders=host\
            &X-Amz-Signature=aeeed9bbccd4d02ee5c0109b86d86835f995330da4c265957d157751f604d404";
        let creds = Creds {
            key_id: "AKIAIOSFODNN7EXAMPLE",
            secret: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            region: "us-east-1",
        };
        let headers = vec![(
            "host".to_string(),
            "examplebucket.s3.amazonaws.com".to_string(),
        )];
        let at = chrono::NaiveDate::from_ymd_opt(2013, 5, 24)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp();
        assert_eq!(
            verify("GET", "/test.txt", query, &headers, &creds, at + 100),
            Ok(())
        );
        // The same URL a day later is expired.
        assert_eq!(
            verify("GET", "/test.txt", query, &headers, &creds, at + 86_400)
                .unwrap_err()
                .0,
            "AccessDenied"
        );
        // The same signature on another method or path is not valid.
        assert!(verify("PUT", "/test.txt", query, &headers, &creds, at + 100).is_err());
        assert!(verify("GET", "/other.txt", query, &headers, &creds, at + 100).is_err());
    }

    #[tokio::test]
    async fn expired_or_tampered_signatures_are_refused() {
        let fake = FakeS3::start(FakeStore::Memory).await;
        fake.put_raw("clips/a.mp4", b"hello".to_vec());
        let (bucket, creds) = signer(&fake);
        let http = reqwest::Client::new();

        let url = bucket
            .get_object(Some(&creds), "clips/a.mp4")
            .sign(Duration::from_secs(3600));
        let r = http.get(url.clone()).send().await.unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(r.bytes().await.unwrap().as_ref(), b"hello");

        // Flip one character of the signature.
        let mut s = url.to_string();
        let i = s.rfind("X-Amz-Signature=").unwrap() + "X-Amz-Signature=".len();
        let c = if &s[i..=i] == "0" { "1" } else { "0" };
        s.replace_range(i..=i, c);
        let r = http.get(&s).send().await.unwrap();
        assert_eq!(r.status(), StatusCode::FORBIDDEN);
        assert!(
            r.text()
                .await
                .unwrap()
                .contains("<Code>SignatureDoesNotMatch</Code>")
        );

        // A longer lifetime than the one signed.
        let longer = url
            .to_string()
            .replace("X-Amz-Expires=3600", "X-Amz-Expires=7200");
        assert_ne!(longer, url.to_string());
        let r = http.get(&longer).send().await.unwrap();
        assert_eq!(r.status(), StatusCode::FORBIDDEN);
        assert!(
            r.text()
                .await
                .unwrap()
                .contains("<Code>SignatureDoesNotMatch</Code>")
        );

        // A different key under the same signature.
        let other = url.to_string().replace("a.mp4", "b.mp4");
        assert_eq!(
            http.get(&other).send().await.unwrap().status(),
            StatusCode::FORBIDDEN
        );

        // An unsigned request.
        let bare = format!("{}/{FAKE_BUCKET}/clips/a.mp4", fake.endpoint());
        assert_eq!(
            http.get(&bare).send().await.unwrap().status(),
            StatusCode::FORBIDDEN
        );

        // Once the (fixed) clock passes the expiry.
        fake.set_now(Some(now() + 3601));
        let r = http.get(url).send().await.unwrap();
        assert_eq!(r.status(), StatusCode::FORBIDDEN);
        assert!(
            r.text()
                .await
                .unwrap()
                .contains("<Code>AccessDenied</Code>")
        );
    }

    #[tokio::test]
    async fn a_put_must_match_its_signed_length() {
        let fake = FakeS3::start(FakeStore::Memory).await;
        let (bucket, creds) = signer(&fake);
        let http = reqwest::Client::new();

        let mut put = bucket.put_object(Some(&creds), "clips/p.mp4");
        put.headers_mut().insert("content-length", "5");
        put.headers_mut().insert("content-type", "video/mp4");
        let url = put.sign(Duration::from_secs(3600));

        let r = http
            .put(url.clone())
            .header("content-type", "video/mp4")
            .body(b"123456".to_vec())
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::FORBIDDEN);
        assert!(fake.keys().is_empty(), "nothing stored");

        // A content-type other than the signed one.
        let r = http
            .put(url.clone())
            .header("content-type", "image/png")
            .body(b"12345".to_vec())
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::FORBIDDEN);
        assert!(fake.keys().is_empty(), "nothing stored");

        let r = http
            .put(url)
            .header("content-type", "video/mp4")
            .body(b"12345".to_vec())
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(
            fake.object("clips/p.mp4"),
            Some((b"12345".to_vec(), "video/mp4".to_string()))
        );
    }

    #[tokio::test]
    async fn ranges_are_served() {
        let fake = FakeS3::start(FakeStore::Memory).await;
        fake.put_raw("clips/r.mp4", b"0123456789".to_vec());
        let (bucket, creds) = signer(&fake);
        let http = reqwest::Client::new();
        let ttl = Duration::from_secs(3600);
        let get = bucket.get_object(Some(&creds), "clips/r.mp4").sign(ttl);

        let r = http
            .get(get.clone())
            .header("range", "bytes=2-4")
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(r.headers()["accept-ranges"], "bytes");
        assert_eq!(r.headers()["content-range"], "bytes 2-4/10");
        assert_eq!(r.bytes().await.unwrap().as_ref(), b"234");

        let r = http
            .get(get.clone())
            .header("range", "bytes=7-")
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(r.bytes().await.unwrap().as_ref(), b"789");

        let r = http
            .get(get.clone())
            .header("range", "bytes=10-")
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::RANGE_NOT_SATISFIABLE);

        let r = http.get(get).send().await.unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(r.bytes().await.unwrap().as_ref(), b"0123456789");

        let head = bucket.head_object(Some(&creds), "clips/r.mp4").sign(ttl);
        let r = http.head(head.clone()).send().await.unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(r.headers()["content-length"], "10");

        let del = bucket.delete_object(Some(&creds), "clips/r.mp4").sign(ttl);
        assert_eq!(
            http.delete(del.clone()).send().await.unwrap().status(),
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            http.head(head).send().await.unwrap().status(),
            StatusCode::NOT_FOUND
        );
        // Deleting a missing key is still 204, like S3.
        assert_eq!(
            http.delete(del).send().await.unwrap().status(),
            StatusCode::NO_CONTENT
        );
    }

    #[tokio::test]
    async fn down_answers_503_and_requests_are_logged() {
        let fake = FakeS3::start(FakeStore::Memory).await;
        fake.put_raw("k", b"x".to_vec());
        let (bucket, creds) = signer(&fake);
        let http = reqwest::Client::new();
        let url = bucket
            .get_object(Some(&creds), "k")
            .sign(Duration::from_secs(60));
        fake.set_down(true);
        assert_eq!(
            http.get(url.clone()).send().await.unwrap().status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
        fake.set_down(false);
        assert_eq!(http.get(url).send().await.unwrap().status(), StatusCode::OK);
        // A missing key is a 404, and still a request that passed the signature check.
        let missing = bucket
            .get_object(Some(&creds), "nope")
            .sign(Duration::from_secs(60));
        assert_eq!(
            http.get(missing).send().await.unwrap().status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            fake.requests(),
            vec![
                (reqwest::Method::GET, "k".to_string()),
                (reqwest::Method::GET, "nope".to_string())
            ]
        );
    }

    #[tokio::test]
    async fn expires_must_be_one_second_to_seven_days() {
        let fake = FakeS3::start(FakeStore::Memory).await;
        fake.put_raw("k", b"x".to_vec());
        let (bucket, creds) = signer(&fake);
        let http = reqwest::Client::new();
        let get = |secs| {
            bucket
                .get_object(Some(&creds), "k")
                .sign(Duration::from_secs(secs))
        };
        assert_eq!(
            http.get(get(604_800)).send().await.unwrap().status(),
            StatusCode::OK
        );
        assert_eq!(
            http.get(get(604_801)).send().await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
        // Hand-built: the signature is irrelevant, the range is checked first.
        let zero = get(60)
            .to_string()
            .replace("X-Amz-Expires=60", "X-Amz-Expires=0");
        let r = http.get(&zero).send().await.unwrap();
        assert_eq!(r.status(), StatusCode::FORBIDDEN);
        assert!(r.text().await.unwrap().contains("X-Amz-Expires must be"));
    }

    #[tokio::test]
    async fn dir_store_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let first = FakeS3::start(FakeStore::Dir(dir.path().to_path_buf())).await;
        let (bucket, creds) = signer(&first);
        let http = reqwest::Client::new();
        let mut put = bucket.put_object(Some(&creds), "clips/d.mp4");
        put.headers_mut().insert("content-length", "3");
        put.headers_mut().insert("content-type", "video/mp4");
        let r = http
            .put(put.sign(Duration::from_secs(60)))
            .header("content-type", "video/mp4")
            .body(b"abc".to_vec())
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        drop(first);

        let second = FakeS3::start(FakeStore::Dir(dir.path().to_path_buf())).await;
        assert_eq!(second.keys(), vec!["clips/d.mp4".to_string()]);
        let (bucket, creds) = signer(&second);
        let r = http
            .get(
                bucket
                    .get_object(Some(&creds), "clips/d.mp4")
                    .sign(Duration::from_secs(60)),
            )
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(r.headers()["content-type"], "video/mp4");
        assert_eq!(r.bytes().await.unwrap().as_ref(), b"abc");
    }
}
