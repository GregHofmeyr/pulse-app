//! Saving a clip (#204) or a quoted image (#206) to disk: the name the Save dialog suggests, and
//! the GET of the original file streamed straight to the chosen path (a clip can be 200 MiB; none
//! of it is held in memory, and none of it passes through the webview).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub use super::upload::CancellationFlag;
use super::upload::STALL_AFTER;

/// What a clip with no usable caption is saved as.
pub const FALLBACK_NAME: &str = "clip";
/// What an image with no usable caption is saved as.
pub const IMAGE_NAME: &str = "image";
/// Every clip is stored as MP4 (`clips/<hash>.mp4`); used when the link doesn't say.
pub const FALLBACK_EXT: &str = "mp4";

#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    /// The store answered with a non-2xx status (an expired link is a 403).
    #[error("the download was refused ({0})")]
    Rejected(u16),
    #[error("the download stopped moving")]
    Stalled,
    /// Stopped on purpose (#205): the UI says nothing.
    #[error("cancelled")]
    Cancelled,
    #[error("couldn't save the file: {0}")]
    File(String),
    #[error("can't reach the server: {0}")]
    Network(String),
}

/// A file name (without extension) from a clip's caption: its first line, without characters
/// Windows, macOS or Linux refuse, trimmed, at most 80 characters; "clip" when nothing is left.
pub fn suggested_stem(caption: &str) -> String {
    stem_or(caption, FALLBACK_NAME)
}

/// `suggested_stem` with another name for when nothing usable is left (an image's is "image").
pub fn stem_or(caption: &str, fallback: &str) -> String {
    const MAX_CHARS: usize = 80;
    const REFUSED: &str = r#"<>:"/\|?*"#;
    let line = caption
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    let kept: String = line
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .filter(|c| !c.is_control() && !REFUSED.contains(*c))
        .collect();
    let short: String = kept
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(MAX_CHARS)
        .collect();
    // Windows drops trailing dots and spaces; a leading dot hides the file elsewhere
    let stem = short.trim_matches(|c: char| c == '.' || c == ' ');
    if stem.is_empty() {
        fallback.to_owned()
    } else if is_reserved_on_windows(stem) {
        format!("{stem} {fallback}")
    } else {
        stem.to_owned()
    }
}

/// CON, PRN, AUX, NUL, COM1–9 and LPT1–9 name devices on Windows, with any extension.
fn is_reserved_on_windows(stem: &str) -> bool {
    let base = stem.split('.').next().unwrap_or(stem).to_ascii_uppercase();
    matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((base.starts_with("COM") || base.starts_with("LPT"))
            && base.len() == 4
            && matches!(base.as_bytes()[3], b'1'..=b'9'))
}

/// The extension of the file a clip link points at (`…/clips/<hash>.mp4?sig=…` → "mp4").
pub fn extension_of(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.path_segments()?.next_back().map(str::to_owned))
        .and_then(|last| {
            last.rsplit_once('.')
                .map(|(_, ext)| ext.to_ascii_lowercase())
        })
        .filter(|ext| {
            (1..=5).contains(&ext.len()) && ext.chars().all(|c| c.is_ascii_alphanumeric())
        })
        .unwrap_or_else(|| FALLBACK_EXT.to_owned())
}

/// How far a download is: the bytes written so far and the whole size when the server said it
/// (`None`: no `content-length`, so the UI shows it indeterminate). Sent to the UI as
/// `download://progress` with the download's key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct Progress {
    pub received: u64,
    pub total: Option<u64>,
}

/// At most ten progress reports a second (the last one, complete, always goes).
pub const PROGRESS_EVERY: Duration = Duration::from_millis(100);

/// Rate limit for progress reports: the first one at once, then at most one per [`PROGRESS_EVERY`].
#[derive(Debug, Default)]
pub struct Throttle {
    last: Option<Instant>,
}

impl Throttle {
    /// Whether a report at `now` goes out (and, if so, it counts as the latest).
    pub fn due(&mut self, now: Instant) -> bool {
        let due = self
            .last
            .is_none_or(|t| now.saturating_duration_since(t) >= PROGRESS_EVERY);
        if due {
            self.last = Some(now);
        }
        due
    }
}

/// GET `url` into `dest`: written to a sibling first and moved over `dest` only when complete, so
/// a failed download leaves no half file (and an existing file as it was).
pub async fn save_url(http: &reqwest::Client, url: &str, dest: &Path) -> Result<(), DownloadError> {
    save_request(http.get(url), dest).await
}

/// `save_url` for a request the caller builds (an image needs the session's token, #206).
pub async fn save_request(req: reqwest::RequestBuilder, dest: &Path) -> Result<(), DownloadError> {
    save_watched(req, dest, |_| {}, &CancellationFlag::new()).await
}

/// [`save_request`] with progress (#205): `on_progress` gets one report as the body starts, then
/// at most one per [`PROGRESS_EVERY`], and the complete one before the file takes its place.
/// `cancel` stops the network read at once and deletes the partial file
/// ([`DownloadError::Cancelled`]); an existing file at `dest` stays as it was.
pub async fn save_watched(
    req: reqwest::RequestBuilder,
    dest: &Path,
    on_progress: impl FnMut(Progress) + Send,
    cancel: &CancellationFlag,
) -> Result<(), DownloadError> {
    let part = partial_path(dest);
    let fetched = fetch_to(req, &part, on_progress, cancel).await;
    let done = match fetched {
        Ok(()) if cancel.is_cancelled() => Err(DownloadError::Cancelled),
        Ok(()) => tokio::fs::rename(&part, dest)
            .await
            .map_err(|e| DownloadError::File(e.to_string())),
        Err(e) => Err(e),
    };
    if done.is_err() {
        let _ = tokio::fs::remove_file(&part).await;
    }
    done
}

async fn fetch_to(
    req: reqwest::RequestBuilder,
    part: &Path,
    mut on_progress: impl FnMut(Progress) + Send,
    cancel: &CancellationFlag,
) -> Result<(), DownloadError> {
    use futures::StreamExt;
    use tokio::io::AsyncWriteExt;
    // the link is signed: keep it out of error text the UI shows
    let net = |e: reqwest::Error| DownloadError::Network(e.without_url().to_string());
    let file_err = |e: std::io::Error| DownloadError::File(e.to_string());
    let res = tokio::select! {
        biased;
        () = cancel.cancelled() => return Err(DownloadError::Cancelled),
        res = req.send() => res.map_err(net)?,
    };
    if !res.status().is_success() {
        return Err(DownloadError::Rejected(res.status().as_u16()));
    }
    let total = res.content_length();
    let mut file = tokio::fs::File::create(part).await.map_err(file_err)?;
    let mut body = res.bytes_stream();
    let mut received = 0u64;
    let mut throttle = Throttle::default();
    let mut reported = None;
    let mut report = |received: u64, throttle: &mut Throttle, force: bool| {
        if reported != Some(received) && (throttle.due(Instant::now()) || force) {
            reported = Some(received);
            on_progress(Progress { received, total });
        }
    };
    report(0, &mut throttle, true);
    let read = loop {
        // dropping the body (cancel, stall, error) closes the connection: the read stops there
        let next = tokio::select! {
            biased;
            () = cancel.cancelled() => break Err(DownloadError::Cancelled),
            next = tokio::time::timeout(STALL_AFTER, body.next()) => next,
        };
        match next {
            Err(_) => break Err(DownloadError::Stalled),
            Ok(None) => break Ok(()),
            Ok(Some(Err(e))) => break Err(net(e)),
            Ok(Some(Ok(chunk))) => {
                if let Err(e) = file.write_all(&chunk).await {
                    break Err(file_err(e));
                }
                received += chunk.len() as u64;
                report(received, &mut throttle, false);
            }
        }
    };
    drop(body);
    // finish any write in flight and close the file before it is moved or deleted (Windows
    // refuses to delete an open file)
    let flushed = file.flush().await.map_err(file_err);
    drop(file);
    read?;
    flushed?;
    report(received, &mut throttle, true);
    Ok(())
}

/// The downloads running now, by key (`clip:<message>`, `image:<hash>`, `file:<attachment>`): one
/// at a time per key, and each one's cancel switch. Cloning shares the map.
#[derive(Clone, Default)]
pub struct Downloads(Arc<Mutex<HashMap<String, CancellationFlag>>>);

/// A download that holds its key until it is dropped.
pub struct Running {
    key: String,
    flag: CancellationFlag,
    all: Downloads,
}

impl Downloads {
    /// Claim `key` for a new download; `None` while one with that key runs.
    pub fn begin(&self, key: impl Into<String>) -> Option<Running> {
        let key = key.into();
        let mut map = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if map.contains_key(&key) {
            return None;
        }
        let flag = CancellationFlag::new();
        map.insert(key.clone(), flag.clone());
        Some(Running {
            key,
            flag,
            all: self.clone(),
        })
    }

    /// Stop the download with `key`; `false` when none runs.
    pub fn cancel(&self, key: &str) -> bool {
        let map = self.0.lock().unwrap_or_else(|e| e.into_inner());
        map.get(key).inspect(|f| f.cancel()).is_some()
    }
}

impl Running {
    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn flag(&self) -> &CancellationFlag {
        &self.flag
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        let mut map = self.all.0.lock().unwrap_or_else(|e| e.into_inner());
        map.remove(&self.key);
    }
}

/// `.<name>.part` beside `dest`.
fn partial_path(dest: &Path) -> PathBuf {
    let name = dest
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    dest.with_file_name(format!(".{name}.part"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_caption_is_the_name() {
        assert_eq!(suggested_stem("what a shot"), "what a shot");
        assert_eq!(suggested_stem("  ace  "), "ace");
    }

    #[test]
    fn no_caption_is_clip() {
        assert_eq!(suggested_stem(""), "clip");
        assert_eq!(suggested_stem("   \n "), "clip");
        assert_eq!(suggested_stem("???"), "clip");
        assert_eq!(suggested_stem(" ... "), "clip");
    }

    #[test]
    fn characters_file_systems_refuse_go() {
        assert_eq!(suggested_stem(r#"a/b\c:d*e?f"g<h>i|j"#), "abcdefghij");
        assert_eq!(suggested_stem("tab\there"), "tab here");
        assert_eq!(suggested_stem("clutch :fire: 1v3"), "clutch fire 1v3");
    }

    #[test]
    fn only_the_first_line_and_spaces_collapse() {
        assert_eq!(suggested_stem("first line\nsecond line"), "first line");
        assert_eq!(suggested_stem("\n\nlater   line  "), "later line");
    }

    #[test]
    fn no_trailing_dots_or_leading_dots() {
        // Windows drops trailing dots and spaces; a leading dot hides the file on Linux and macOS
        assert_eq!(suggested_stem("gg..."), "gg");
        assert_eq!(suggested_stem(".hidden"), "hidden");
    }

    #[test]
    fn windows_reserved_names_are_kept_usable() {
        assert_eq!(suggested_stem("CON"), "CON clip");
        assert_eq!(suggested_stem("lpt1"), "lpt1 clip");
        assert_eq!(suggested_stem("console"), "console");
    }

    #[test]
    fn long_captions_are_cut_on_a_character() {
        let long = "é".repeat(200);
        let stem = suggested_stem(&long);
        assert_eq!(stem.chars().count(), 80);
        assert_eq!(
            suggested_stem(&format!("{} tail", "a".repeat(79))),
            "a".repeat(79)
        );
    }

    #[test]
    fn an_image_falls_back_to_image() {
        // saving a quoted image (#206): the caption, or "image" when there is none
        assert_eq!(stem_or("the look", IMAGE_NAME), "the look");
        assert_eq!(stem_or("  ", IMAGE_NAME), "image");
        assert_eq!(stem_or("nul", IMAGE_NAME), "nul image");
    }

    #[test]
    fn the_extension_comes_from_the_link() {
        assert_eq!(
            extension_of("https://r2.example.com/b/clips/abc.mp4?X-Amz-Signature=1"),
            "mp4"
        );
        assert_eq!(extension_of("http://127.0.0.1:7895/clips/k.webm"), "webm");
        assert_eq!(extension_of("http://127.0.0.1:7895/clips/K.MP4#t=1"), "mp4");
    }

    #[test]
    fn a_link_without_one_is_mp4() {
        assert_eq!(extension_of("https://r2.example.com/b/clips/abc"), "mp4");
        assert_eq!(
            extension_of("https://r2.example.com/b.d/clips/abc?x=1.mov"),
            "mp4"
        );
        assert_eq!(extension_of("not a url"), "mp4");
        assert_eq!(extension_of("https://e.com/a.toolongext"), "mp4");
    }

    /// One HTTP/1.1 response per connection, for `save_url`.
    async fn serve_once(status: &'static str, body: Vec<u8>) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut s, _) = l.accept().await.unwrap();
            let mut buf = [0u8; 4096];
            let _ = s.read(&mut buf).await;
            let head = format!(
                "HTTP/1.1 {status}\r\ncontent-length: {}\r\ncontent-type: video/mp4\r\nconnection: close\r\n\r\n",
                body.len()
            );
            s.write_all(head.as_bytes()).await.unwrap();
            s.write_all(&body).await.unwrap();
            s.shutdown().await.unwrap();
        });
        format!("http://{addr}/clips/abc.mp4?sig=1")
    }

    fn leftovers(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[tokio::test]
    async fn saves_the_bytes_to_the_chosen_file() {
        let bytes: Vec<u8> = (0..3 * 1024 * 1024).map(|i| (i * 31 % 251) as u8).collect();
        let url = serve_once("200 OK", bytes.clone()).await;
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("what a shot.mp4");
        save_url(&reqwest::Client::new(), &url, &dest)
            .await
            .unwrap();
        assert!(
            std::fs::read(&dest).unwrap() == bytes,
            "the saved bytes are the served ones"
        );
        assert_eq!(leftovers(dir.path()), vec!["what a shot.mp4"]);
    }

    #[tokio::test]
    async fn saves_any_request_the_caller_builds() {
        // an image (#206) is fetched with the session's token: the caller builds the request
        let url = serve_once("200 OK", b"webp bytes".to_vec()).await;
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("the look.webp");
        let req = reqwest::Client::new().get(&url).bearer_auth("t");
        save_request(req, &dest).await.unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"webp bytes");
        assert_eq!(leftovers(dir.path()), vec!["the look.webp"]);
    }

    #[tokio::test]
    async fn replaces_a_file_the_dialog_agreed_to_overwrite() {
        let url = serve_once("200 OK", b"new".to_vec()).await;
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("ace.mp4");
        std::fs::write(&dest, b"old and longer").unwrap();
        save_url(&reqwest::Client::new(), &url, &dest)
            .await
            .unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"new");
    }

    #[tokio::test]
    async fn a_refused_download_leaves_nothing_behind() {
        let url = serve_once("403 Forbidden", b"<Error/>".to_vec()).await;
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("ace.mp4");
        std::fs::write(&dest, b"keep me").unwrap();
        let err = save_url(&reqwest::Client::new(), &url, &dest)
            .await
            .unwrap_err();
        assert!(matches!(err, DownloadError::Rejected(403)), "{err:?}");
        assert_eq!(std::fs::read(&dest).unwrap(), b"keep me");
        assert_eq!(leftovers(dir.path()), vec!["ace.mp4"]);
    }

    #[tokio::test]
    async fn an_unreachable_server_is_a_network_error() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("ace.mp4");
        let err = save_url(
            &reqwest::Client::new(),
            "http://127.0.0.1:9/clips/a.mp4",
            &dest,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, DownloadError::Network(_)), "{err:?}");
        assert!(leftovers(dir.path()).is_empty());
    }

    #[test]
    fn progress_goes_out_at_most_ten_times_a_second() {
        let t0 = Instant::now();
        let ms = |n: u64| t0 + Duration::from_millis(n);
        let mut t = Throttle::default();
        assert!(t.due(ms(0)), "the first report goes at once");
        assert!(!t.due(ms(1)));
        assert!(!t.due(ms(99)));
        assert!(t.due(ms(100)));
        assert!(!t.due(ms(150)));
        assert!(t.due(ms(260)));
        let sent = (0..1000).filter(|i| t.due(ms(1000 + i))).count();
        assert_eq!(
            sent, 10,
            "one second of chunks every millisecond is ten reports"
        );
    }

    /// A response written in `chunks`, each after `gap`; with `length` false it has no
    /// content-length (chunked).
    async fn serve_slowly(chunks: Vec<Vec<u8>>, gap: Duration, length: bool) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut s, _) = l.accept().await.unwrap();
            let mut buf = [0u8; 4096];
            let _ = s.read(&mut buf).await;
            let size: usize = chunks.iter().map(Vec::len).sum();
            let head = if length {
                format!("HTTP/1.1 200 OK\r\ncontent-length: {size}\r\nconnection: close\r\n\r\n")
            } else {
                "HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\nconnection: close\r\n\r\n".into()
            };
            s.write_all(head.as_bytes()).await.unwrap();
            for c in chunks {
                tokio::time::sleep(gap).await;
                if length {
                    s.write_all(&c).await.unwrap();
                } else {
                    s.write_all(format!("{:x}\r\n", c.len()).as_bytes())
                        .await
                        .unwrap();
                    s.write_all(&c).await.unwrap();
                    s.write_all(b"\r\n").await.unwrap();
                }
                s.flush().await.unwrap();
            }
            if !length {
                s.write_all(b"0\r\n\r\n").await.unwrap();
            }
            s.shutdown().await.unwrap();
        });
        format!("http://{addr}/clips/abc.mp4?sig=1")
    }

    async fn watched(url: &str, dest: &Path) -> (Result<(), DownloadError>, Vec<Progress>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        let res = save_watched(
            reqwest::Client::new().get(url),
            dest,
            move |p| log.lock().unwrap().push(p),
            &CancellationFlag::new(),
        )
        .await;
        let seen = seen.lock().unwrap().clone();
        (res, seen)
    }

    #[tokio::test]
    async fn progress_is_throttled_and_ends_complete() {
        // 60 chunks over about 600 ms: a report per chunk would be 60
        let chunks: Vec<Vec<u8>> = (0..60).map(|i| vec![i as u8; 1024]).collect();
        let url = serve_slowly(chunks, Duration::from_millis(10), true).await;
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("ace.mp4");
        let started = Instant::now();
        let (res, seen) = watched(&url, &dest).await;
        res.unwrap();
        let elapsed = started.elapsed().as_millis() as usize;
        let total = Some(60 * 1024);
        assert_eq!(seen.first(), Some(&Progress { received: 0, total }));
        assert_eq!(
            seen.last(),
            Some(&Progress {
                received: 60 * 1024,
                total
            })
        );
        assert!(
            seen.len() <= elapsed / 100 + 2,
            "{} reports in {elapsed} ms",
            seen.len()
        );
        assert!(seen.len() >= 3, "it moves in between: {seen:?}");
        assert!(seen.windows(2).all(|w| w[0].received < w[1].received));
        assert_eq!(std::fs::read(&dest).unwrap().len(), 60 * 1024);
    }

    #[tokio::test]
    async fn an_unknown_length_reports_no_total() {
        let chunks: Vec<Vec<u8>> = (0..5).map(|_| vec![7u8; 2000]).collect();
        let url = serve_slowly(chunks, Duration::from_millis(5), false).await;
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("ace.mp4");
        let (res, seen) = watched(&url, &dest).await;
        res.unwrap();
        assert!(seen.iter().all(|p| p.total.is_none()), "{seen:?}");
        assert_eq!(seen.last().unwrap().received, 10_000);
        assert_eq!(std::fs::read(&dest).unwrap(), vec![7u8; 10_000]);
    }

    #[tokio::test]
    async fn cancelling_stops_the_read_and_deletes_the_partial_file() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        let (closed_tx, closed_rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let (mut s, _) = l.accept().await.unwrap();
            let mut buf = [0u8; 4096];
            let _ = s.read(&mut buf).await;
            let head = "HTTP/1.1 200 OK\r\ncontent-length: 104857600\r\nconnection: close\r\n\r\n";
            s.write_all(head.as_bytes()).await.unwrap();
            s.write_all(&vec![1u8; 64 * 1024]).await.unwrap();
            s.flush().await.unwrap();
            // then nothing more: the client has to hang up for this read to end
            let _ = closed_tx.send(s.read(&mut buf).await.map(|n| n == 0).unwrap_or(true));
        });
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("ace.mp4");
        std::fs::write(&dest, b"keep me").unwrap();
        let cancel = CancellationFlag::new();
        let stop = cancel.clone();
        let part = partial_path(&dest);
        // once the first bytes are on disk and the server has gone quiet, the user cancels
        tokio::spawn(async move {
            while std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0) < 64 * 1024 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            stop.cancel();
        });
        let err = save_watched(
            reqwest::Client::new().get(format!("http://{addr}/clips/a.mp4")),
            &dest,
            |_| {},
            &cancel,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, DownloadError::Cancelled), "{err:?}");
        assert_eq!(std::fs::read(&dest).unwrap(), b"keep me");
        assert_eq!(leftovers(dir.path()), vec!["ace.mp4"], "no partial file");
        let closed = tokio::time::timeout(Duration::from_secs(5), closed_rx)
            .await
            .expect("the connection closes")
            .unwrap();
        assert!(closed, "the client hung up");
    }

    #[tokio::test]
    async fn cancelled_before_it_starts_saves_nothing() {
        let url = serve_once("200 OK", b"x".to_vec()).await;
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("ace.mp4");
        let cancel = CancellationFlag::new();
        cancel.cancel();
        let err = save_watched(reqwest::Client::new().get(&url), &dest, |_| {}, &cancel)
            .await
            .unwrap_err();
        assert!(matches!(err, DownloadError::Cancelled), "{err:?}");
        assert!(leftovers(dir.path()).is_empty());
    }

    #[test]
    fn one_download_per_key_at_a_time() {
        let all = Downloads::default();
        let first = all.begin("clip:1").expect("free");
        assert!(all.begin("clip:1").is_none(), "a second one doesn't start");
        let other = all
            .begin("clip:2")
            .expect("another clip is its own download");
        assert!(all.cancel("clip:1"));
        assert!(first.flag().is_cancelled());
        assert!(!other.flag().is_cancelled());
        assert!(!all.cancel("clip:3"), "nothing to stop");
        drop(first);
        assert!(all.begin("clip:1").is_some(), "free again once it ends");
    }

    #[tokio::test]
    async fn a_folder_that_cannot_be_written_is_a_file_error() {
        let url = serve_once("200 OK", b"x".to_vec()).await;
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("missing").join("ace.mp4");
        let err = save_url(&reqwest::Client::new(), &url, &dest)
            .await
            .unwrap_err();
        assert!(matches!(err, DownloadError::File(_)), "{err:?}");
    }
}
