//! The `pulse-media` URI scheme: emoji images for the webview. The webview can't send our bearer
//! token, so the core fetches each hash once, keeps it on disk and serves it from there.
//! (Windows: `http://pulse-media.localhost/<hash>`; elsewhere `pulse-media://localhost/<hash>`.)

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use tauri::http::{Response, StatusCode, header};
use tauri::{AppHandle, Manager, UriSchemeContext, UriSchemeResponder, Wry};

use crate::api::{Api, ApiError};
use crate::commands::Core;

pub const SCHEME: &str = "pulse-media";

/// The hash in a `/<hash>` request path.
pub fn parse_path(path: &str) -> Option<&str> {
    let hash = path.strip_prefix('/')?;
    let hex = hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    (hash.len() == 64 && hex).then_some(hash)
}

/// `root/media/ab/abcdef….webp`, sharded like the server's store.
pub fn cache_path(root: &Path, hash: &str) -> PathBuf {
    root.join("media")
        .join(&hash[..2])
        .join(format!("{hash}.webp"))
}

/// Content-addressed: the bytes must hash to their name.
pub fn verify(hash: &str, bytes: &[u8]) -> bool {
    let digest = Sha256::digest(bytes);
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    hex == hash
}

/// The registered handler: the work runs on the async runtime, never the UI thread.
pub fn handle(
    ctx: UriSchemeContext<'_, Wry>,
    req: tauri::http::Request<Vec<u8>>,
    responder: UriSchemeResponder,
) {
    let app = ctx.app_handle().clone();
    let path = req.uri().path().to_string();
    tauri::async_runtime::spawn(async move {
        responder.respond(respond(&app, &path).await);
    });
}

async fn respond(app: &AppHandle, path: &str) -> Response<Vec<u8>> {
    // the window exists a moment before Core does (setup builds it first)
    let Some(core) = app.try_state::<Core>() else {
        return status(StatusCode::SERVICE_UNAVAILABLE);
    };
    let session = core.current().ok();
    let signed_in = session.is_some();
    let root = match app.path().app_cache_dir() {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, "media: no cache directory");
            return status(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };
    match serve(session, &root, path).await {
        Ok(bytes) => Response::builder()
            .header(header::CONTENT_TYPE, "image/webp")
            .header(header::CACHE_CONTROL, "max-age=31536000, immutable")
            .body(bytes)
            .expect("static headers"),
        Err(code) => {
            if code == StatusCode::UNAUTHORIZED && signed_in {
                // the server refused our token: the session is dead, same as any other 401
                let _ = core.check::<()>(app, Err(ApiError::Unauthorized));
            }
            status(code)
        }
    }
}

fn status(code: StatusCode) -> Response<Vec<u8>> {
    Response::builder()
        .status(code)
        .body(Vec::new())
        .expect("status only")
}

/// The image behind `path`: from the cache, else fetched, checked and cached. Signed out serves
/// nothing, not even what's cached.
async fn serve(
    session: Option<(Api, String)>,
    root: &Path,
    path: &str,
) -> Result<Vec<u8>, StatusCode> {
    let hash = parse_path(path).ok_or(StatusCode::NOT_FOUND)?;
    let (api, token) = session.ok_or(StatusCode::UNAUTHORIZED)?;
    let file = cache_path(root, hash);
    // checked on the way out too: a cache file that went bad is fetched again
    if let Ok(bytes) = tokio::fs::read(&file).await
        && verify(hash, &bytes)
    {
        return Ok(bytes);
    }
    let bytes = match api.media(&token, hash).await {
        Ok(b) => b,
        Err(ApiError::Unauthorized) => return Err(StatusCode::UNAUTHORIZED),
        Err(ApiError::Rejected(_)) => return Err(StatusCode::NOT_FOUND),
        Err(_) => return Err(StatusCode::BAD_GATEWAY),
    };
    if !verify(hash, &bytes) {
        tracing::warn!(hash, "media: the server's bytes don't match their hash");
        return Err(StatusCode::BAD_GATEWAY);
    }
    if let Err(e) = store(&file, &bytes).await {
        tracing::warn!(error = %e, "media: caching failed");
    }
    Ok(bytes)
}

/// Written aside, then renamed: a reader never sees half a file.
async fn store(file: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = file.parent().expect("sharded path");
    tokio::fs::create_dir_all(dir).await?;
    let tmp = dir.join(format!(".{}.tmp", rand::random::<u64>()));
    tokio::fs::write(&tmp, bytes).await?;
    let renamed = tokio::fs::rename(&tmp, file).await;
    if renamed.is_err() {
        let _ = tokio::fs::remove_file(&tmp).await;
    }
    renamed
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use pulse_server::testing;

    const HASH: &str = "ab0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcd";

    #[test]
    fn parses_only_hex_hashes() {
        assert_eq!(parse_path(&format!("/{HASH}")), Some(HASH));
        assert_eq!(parse_path(HASH), None, "the path starts with a slash");
        assert_eq!(parse_path(&format!("/{}", HASH.to_uppercase())), None);
        assert_eq!(parse_path(&format!("/{}", &HASH[1..])), None);
        assert_eq!(parse_path(&format!("/{HASH}0")), None);
        assert_eq!(parse_path(&format!("/../{}", &HASH[3..])), None);
        assert_eq!(parse_path(&format!("/{HASH}/x")), None);
        assert_eq!(parse_path("/"), None);
    }

    #[test]
    fn cache_path_is_sharded() {
        let root = Path::new("cache");
        assert_eq!(
            cache_path(root, HASH),
            root.join("media").join("ab").join(format!("{HASH}.webp"))
        );
    }

    #[test]
    fn rejects_bytes_that_dont_match_hash() {
        // sha256("abc")
        let abc = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert!(verify(abc, b"abc"));
        assert!(!verify(abc, b"abd"));
        assert!(!verify(HASH, b"abc"));
    }

    /// 1×1 transparent PNG.
    pub(crate) const PNG: &[u8] = &[
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f,
        0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0x64,
        0x60, 0xf8, 0x5f, 0x0f, 0x00, 0x02, 0x87, 0x01, 0x80, 0xeb, 0x47, 0xba, 0x92, 0x00, 0x00,
        0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ];

    #[tokio::test]
    async fn signed_out_serves_nothing_even_when_cached() {
        let dir = tempfile::tempdir().unwrap();
        let file = cache_path(dir.path(), HASH);
        store(&file, b"cached").await.unwrap();
        assert_eq!(
            serve(None, dir.path(), &format!("/{HASH}")).await,
            Err(StatusCode::UNAUTHORIZED)
        );
        assert_eq!(
            serve(None, dir.path(), "/nope").await,
            Err(StatusCode::NOT_FOUND)
        );
    }

    #[tokio::test]
    async fn fetches_once_then_serves_from_the_cache() {
        let app = testing::spawn().await;
        let (_, token) = testing::register(&app, "alex").await;
        let api = Api::new(&format!("http://{}", app.addr));
        let info = api.upload_emoji_media(&token, PNG.to_vec()).await.unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = format!("/{}", info.hash);

        let first = serve(Some((api, token.clone())), dir.path(), &path)
            .await
            .unwrap();
        assert!(verify(&info.hash, &first));
        assert!(cache_path(dir.path(), &info.hash).exists());

        // the server is gone: the cache answers
        let offline = Api::new("http://localhost:9");
        let again = serve(Some((offline.clone(), token)), dir.path(), &path)
            .await
            .unwrap();
        assert_eq!(again, first);

        // a hash nobody stored, with nothing reachable: a gateway error, not a crash
        assert_eq!(
            serve(Some((offline, "t".into())), dir.path(), &format!("/{HASH}")).await,
            Err(StatusCode::BAD_GATEWAY)
        );
    }

    #[tokio::test]
    async fn unknown_hash_is_404_and_a_dead_token_401() {
        let app = testing::spawn().await;
        let (_, token) = testing::register(&app, "alex").await;
        let api = Api::new(&format!("http://{}", app.addr));
        let dir = tempfile::tempdir().unwrap();
        let path = format!("/{HASH}");
        assert_eq!(
            serve(Some((api.clone(), token)), dir.path(), &path).await,
            Err(StatusCode::NOT_FOUND)
        );
        assert_eq!(
            serve(Some((api, "garbage".into())), dir.path(), &path).await,
            Err(StatusCode::UNAUTHORIZED)
        );
    }

    #[tokio::test]
    async fn a_bad_cache_file_is_fetched_again() {
        let app = testing::spawn().await;
        let (_, token) = testing::register(&app, "alex").await;
        let api = Api::new(&format!("http://{}", app.addr));
        let info = api.upload_emoji_media(&token, PNG.to_vec()).await.unwrap();
        let dir = tempfile::tempdir().unwrap();
        let file = cache_path(dir.path(), &info.hash);
        store(&file, b"truncated").await.unwrap();
        let bytes = serve(Some((api, token)), dir.path(), &format!("/{}", info.hash))
            .await
            .unwrap();
        assert!(verify(&info.hash, &bytes));
        assert_eq!(std::fs::read(&file).unwrap(), bytes);
    }
}
