//! Saving a clip to disk (#204): the name the Save dialog suggests, and the GET of the clip's
//! original file streamed straight to the chosen path (a clip can be 200 MiB; none of it is held
//! in memory, and none of it passes through the webview).

use std::path::{Path, PathBuf};

use super::upload::STALL_AFTER;

/// What a clip with no usable caption is saved as.
pub const FALLBACK_NAME: &str = "clip";
/// Every clip is stored as MP4 (`clips/<hash>.mp4`); used when the link doesn't say.
pub const FALLBACK_EXT: &str = "mp4";

#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    /// The store answered with a non-2xx status (an expired link is a 403).
    #[error("the download was refused ({0})")]
    Rejected(u16),
    #[error("the download stopped moving")]
    Stalled,
    #[error("couldn't save the clip: {0}")]
    File(String),
    #[error("can't reach the clip server: {0}")]
    Network(String),
}

/// A file name (without extension) from a clip's caption: its first line, without characters
/// Windows, macOS or Linux refuse, trimmed, at most 80 characters; "clip" when nothing is left.
pub fn suggested_stem(caption: &str) -> String {
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
        FALLBACK_NAME.to_owned()
    } else if is_reserved_on_windows(stem) {
        format!("{stem} {FALLBACK_NAME}")
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

/// GET `url` into `dest`: written to a sibling first and moved over `dest` only when complete, so
/// a failed download leaves no half file (and an existing file as it was).
pub async fn save_url(http: &reqwest::Client, url: &str, dest: &Path) -> Result<(), DownloadError> {
    let part = partial_path(dest);
    let fetched = fetch_to(http, url, &part).await;
    let done = match fetched {
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

async fn fetch_to(http: &reqwest::Client, url: &str, part: &Path) -> Result<(), DownloadError> {
    use futures::StreamExt;
    use tokio::io::AsyncWriteExt;
    // the link is signed: keep it out of error text the UI shows
    let net = |e: reqwest::Error| DownloadError::Network(e.without_url().to_string());
    let file_err = |e: std::io::Error| DownloadError::File(e.to_string());
    let res = http.get(url).send().await.map_err(net)?;
    if !res.status().is_success() {
        return Err(DownloadError::Rejected(res.status().as_u16()));
    }
    let mut file = tokio::fs::File::create(part).await.map_err(file_err)?;
    let mut body = res.bytes_stream();
    loop {
        match tokio::time::timeout(STALL_AFTER, body.next()).await {
            Err(_) => return Err(DownloadError::Stalled),
            Ok(None) => break,
            Ok(Some(Err(e))) => return Err(net(e)),
            Ok(Some(Ok(chunk))) => file.write_all(&chunk).await.map_err(file_err)?,
        }
    }
    file.flush().await.map_err(file_err)
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
