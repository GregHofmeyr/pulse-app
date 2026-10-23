//! The `pulse-media` URI scheme: emoji images for the webview. The webview can't send our bearer
//! token, so the core fetches each hash once, keeps it on disk and serves it from there.
//! (Windows: `http://pulse-media.localhost/<hash>`; elsewhere `pulse-media://localhost/<hash>`.)
//! Each account on each server has its own cache: nothing fetched under one login is served
//! under another without that login's own authorised fetch. Signing out deletes it.

use std::path::{Path, PathBuf};

use pulse_protocol::ids::UserId;
use sha2::{Digest, Sha256};
use tauri::http::{Response, StatusCode, header};
use tauri::{AppHandle, Manager, UriSchemeContext, UriSchemeResponder, Wry};

use crate::api::{Api, ApiError};
use crate::commands::Core;

pub const SCHEME: &str = "pulse-media";

/// Who is asking: the signed-in session, and its cache (`None` until we know who it is: then
/// nothing is cached).
pub struct Session {
    pub api: Api,
    pub token: String,
    pub cache: Option<PathBuf>,
}

/// This account's cache on this server: `root/<sha256(server + user)>`. The server is
/// normalised first, so `HTTPS://Host:443/` and `https://host` are the same scope.
pub fn scope_dir(root: &Path, server: &str, user: &UserId) -> PathBuf {
    let server = match reqwest::Url::parse(server) {
        Ok(u) => u.as_str().trim_end_matches('/').to_string(),
        Err(_) => server.trim_end_matches('/').to_string(),
    };
    root.join(hex(&Sha256::digest(format!("{server}\n{user}"))))
}

/// A media hash: sha256 as 64 lowercase hex digits.
pub fn is_hash(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// The hash in a `/<hash>` request path.
pub fn parse_path(path: &str) -> Option<&str> {
    path.strip_prefix('/').filter(|h| is_hash(h))
}

/// `scope/ab/abcdef….webp`, sharded like the server's store.
pub fn cache_path(scope: &Path, hash: &str) -> PathBuf {
    scope.join(&hash[..2]).join(format!("{hash}.webp"))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Content-addressed: the bytes must hash to their name.
pub fn verify(hash: &str, bytes: &[u8]) -> bool {
    hex(&Sha256::digest(bytes)) == hash
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
    let session = core.media_session();
    let signed_in = session.is_some();
    match serve(session, path).await {
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

/// The image behind `path`: from this session's cache, else fetched, checked and cached. Signed
/// out serves nothing, not even what's cached.
pub async fn serve(session: Option<Session>, path: &str) -> Result<Vec<u8>, StatusCode> {
    let hash = parse_path(path).ok_or(StatusCode::NOT_FOUND)?;
    let Session { api, token, cache } = session.ok_or(StatusCode::UNAUTHORIZED)?;
    let file = cache.map(|scope| cache_path(&scope, hash));
    // checked on the way out too: a cache file that went bad is fetched again
    if let Some(file) = &file
        && let Ok(bytes) = tokio::fs::read(file).await
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
    if let Some(file) = &file
        && let Err(e) = store(file, &bytes).await
    {
        tracing::warn!(error = %e, "media: caching failed");
    }
    Ok(bytes)
}

/// Signed out: the webview drops its HTTP and memory caches, so an image this session loaded
/// (served `immutable`) isn't shown again without the core being asked. Only the caches:
/// `clear_all_browsing_data` would also wipe localStorage, i.e. this machine's audio and other
/// settings. Asynchronous: it returns before the webview has finished.
pub fn clear_webview_cache(app: &AppHandle) {
    let Some(win) = app.get_webview_window("main") else {
        return;
    };
    #[cfg(target_os = "linux")]
    let sent = win.with_webview(|wv| {
        use webkit2gtk::{
            WebContextExt, WebViewExt, WebsiteDataManagerExtManual, WebsiteDataTypes, gio, glib,
        };
        let Some(data) = wv.inner().context().and_then(|c| c.website_data_manager()) else {
            tracing::warn!("webview cache: no data manager");
            return;
        };
        data.clear(
            WebsiteDataTypes::MEMORY_CACHE | WebsiteDataTypes::DISK_CACHE,
            glib::TimeSpan::from_seconds(0),
            None::<&gio::Cancellable>,
            |r| match r {
                Ok(()) => tracing::info!("webview cache cleared"),
                Err(e) => tracing::warn!(error = %e, "webview cache not cleared"),
            },
        );
    });
    #[cfg(windows)]
    let sent = win.with_webview(|wv| {
        use webview2_com::ClearBrowsingDataCompletedHandler;
        use webview2_com::Microsoft::Web::WebView2::Win32::{
            COREWEBVIEW2_BROWSING_DATA_KINDS_DISK_CACHE, ICoreWebView2_13, ICoreWebView2Profile2,
        };
        use windows_core::Interface;
        let done = ClearBrowsingDataCompletedHandler::create(Box::new(|r| {
            match r {
                Ok(()) => tracing::info!("webview cache cleared"),
                Err(e) => tracing::warn!(error = %e, "webview cache not cleared"),
            }
            Ok(())
        }));
        // SAFETY: plain COM calls on the webview's own thread (with_webview runs us there).
        let asked = unsafe {
            wv.controller()
                .CoreWebView2()
                .and_then(|core| core.cast::<ICoreWebView2_13>())
                .and_then(|v13| v13.Profile())
                .and_then(|p| p.cast::<ICoreWebView2Profile2>())
                .and_then(|p| {
                    p.ClearBrowsingData(COREWEBVIEW2_BROWSING_DATA_KINDS_DISK_CACHE, &done)
                })
        };
        if let Err(e) = asked {
            // ICoreWebView2Profile2 needs WebView2 Runtime 108+
            tracing::warn!(error = %e, "webview cache not cleared");
        }
    });
    #[cfg(not(any(target_os = "linux", windows)))]
    let sent: tauri::Result<()> = Ok(());
    if let Err(e) = sent {
        tracing::warn!(error = %e, "webview cache: webview unavailable");
    }
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

    fn session(api: &Api, token: &str, cache: Option<&Path>) -> Option<Session> {
        Some(Session {
            api: api.clone(),
            token: token.into(),
            cache: cache.map(Path::to_path_buf),
        })
    }

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
        let scope = Path::new("scope");
        assert_eq!(
            cache_path(scope, HASH),
            scope.join("ab").join(format!("{HASH}.webp"))
        );
    }

    #[test]
    fn each_account_on_each_server_has_its_own_scope() {
        let root = Path::new("media");
        let alex: UserId = "01J0000000000000000000000A".parse().unwrap();
        let sam: UserId = "01J0000000000000000000000B".parse().unwrap();
        let here = scope_dir(root, "https://pulse.example.com", &alex);
        assert_eq!(here.parent(), Some(root));
        assert!(is_hash(&here.file_name().unwrap().to_string_lossy()));
        assert_ne!(here, scope_dir(root, "https://pulse.example.com", &sam));
        assert_ne!(here, scope_dir(root, "https://other.example.com", &alex));
        assert_ne!(
            here,
            scope_dir(root, "https://pulse.example.com:8443", &alex)
        );
        assert_ne!(here, scope_dir(root, "https://pulse.example.com/b", &alex));
        for same in [
            "https://pulse.example.com/",
            "HTTPS://Pulse.Example.COM",
            "https://pulse.example.com:443",
        ] {
            assert_eq!(here, scope_dir(root, same, &alex), "{same}");
        }
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
            serve(None, &format!("/{HASH}")).await,
            Err(StatusCode::UNAUTHORIZED)
        );
        assert_eq!(serve(None, "/nope").await, Err(StatusCode::NOT_FOUND));
    }

    #[tokio::test]
    async fn fetches_once_then_serves_from_the_cache() {
        let app = testing::spawn().await;
        let (_, token) = testing::register(&app, "alex").await;
        let api = Api::new(&format!("http://{}", app.addr));
        let info = api.upload_emoji_media(&token, PNG.to_vec()).await.unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = format!("/{}", info.hash);

        let first = serve(session(&api, &token, Some(dir.path())), &path)
            .await
            .unwrap();
        assert!(verify(&info.hash, &first));
        assert!(cache_path(dir.path(), &info.hash).exists());

        // the server is gone: the cache answers
        let offline = Api::new("http://localhost:9");
        let again = serve(session(&offline, &token, Some(dir.path())), &path)
            .await
            .unwrap();
        assert_eq!(again, first);

        // a hash nobody stored, with nothing reachable: a gateway error, not a crash
        assert_eq!(
            serve(
                session(&offline, "t", Some(dir.path())),
                &format!("/{HASH}")
            )
            .await,
            Err(StatusCode::BAD_GATEWAY)
        );
    }

    #[tokio::test]
    async fn another_account_or_server_fetches_for_itself() {
        let app = testing::spawn().await;
        let base = format!("http://{}", app.addr);
        let (alex_id, alex) = testing::register(&app, "alex").await;
        let (sam_id, sam) = testing::register(&app, "sam").await;
        let api = Api::new(&base);
        let info = api.upload_emoji_media(&alex, PNG.to_vec()).await.unwrap();
        let path = format!("/{}", info.hash);
        let root = tempfile::tempdir().unwrap();
        let alex_here = scope_dir(root.path(), &base, &alex_id);
        let sam_here = scope_dir(root.path(), &base, &sam_id);
        let alex_elsewhere = scope_dir(root.path(), "https://other.example.com", &alex_id);
        serve(session(&api, &alex, Some(&alex_here)), &path)
            .await
            .unwrap();

        // sam's cache doesn't have it: with the server gone there is nothing to serve
        let offline = Api::new("http://localhost:9");
        assert_eq!(
            serve(session(&offline, &sam, Some(&sam_here)), &path).await,
            Err(StatusCode::BAD_GATEWAY)
        );
        // ...and the fetch is sam's own, checked by the server
        assert_eq!(
            serve(session(&api, "garbage", Some(&sam_here)), &path).await,
            Err(StatusCode::UNAUTHORIZED)
        );
        assert!(!cache_path(&sam_here, &info.hash).exists());
        let bytes = serve(session(&api, &sam, Some(&sam_here)), &path)
            .await
            .unwrap();
        assert!(verify(&info.hash, &bytes));
        assert!(cache_path(&sam_here, &info.hash).exists());

        // the same account on another server starts empty too
        assert_eq!(
            serve(session(&offline, &alex, Some(&alex_elsewhere)), &path).await,
            Err(StatusCode::BAD_GATEWAY)
        );
    }

    #[tokio::test]
    async fn before_we_know_who_it_is_it_still_serves() {
        let app = testing::spawn().await;
        let (_, token) = testing::register(&app, "alex").await;
        let api = Api::new(&format!("http://{}", app.addr));
        let info = api.upload_emoji_media(&token, PNG.to_vec()).await.unwrap();
        let bytes = serve(session(&api, &token, None), &format!("/{}", info.hash))
            .await
            .unwrap();
        assert!(verify(&info.hash, &bytes));
    }

    #[tokio::test]
    async fn unknown_hash_is_404_and_a_dead_token_401() {
        let app = testing::spawn().await;
        let (_, token) = testing::register(&app, "alex").await;
        let api = Api::new(&format!("http://{}", app.addr));
        let dir = tempfile::tempdir().unwrap();
        let path = format!("/{HASH}");
        assert_eq!(
            serve(session(&api, &token, Some(dir.path())), &path).await,
            Err(StatusCode::NOT_FOUND)
        );
        assert_eq!(
            serve(session(&api, "garbage", Some(dir.path())), &path).await,
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
        let bytes = serve(
            session(&api, &token, Some(dir.path())),
            &format!("/{}", info.hash),
        )
        .await
        .unwrap();
        assert!(verify(&info.hash, &bytes));
        assert_eq!(std::fs::read(&file).unwrap(), bytes);
    }
}
