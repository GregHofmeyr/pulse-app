//! The author's avatar beside a message notification (#73). Windows toasts and Linux notification
//! services both want a local file, and neither reliably reads WebP, so the cached avatar is
//! written once as a small PNG in this account's media cache (deleted with it on sign-out).
//! Anything that goes wrong (no avatar, signed out, server slow or down, a bad image) means a
//! notification without one: never a late or lost notification.

use std::path::{Path, PathBuf};
use std::time::Duration;

use image::ImageFormat;

use crate::media::{self, Session};

/// The largest side kept: Windows shows the logo at 48 px (96 px at 200 %), Linux services less.
pub const SIDE: u32 = 128;
/// How long a notification waits for an avatar not yet cached, well inside the page's 4 s
/// per-conversation window (lib/notify.ts), so a slow fetch never reorders two notifications.
pub const FETCH_WAIT: Duration = Duration::from_millis(1500);

/// `scope/notify/<hash>.png`; `hash` was checked by [media::is_hash].
pub fn png_path(scope: &Path, hash: &str) -> PathBuf {
    scope.join("notify").join(format!("{hash}.png"))
}

/// The image (an animated one's first frame) as a PNG no bigger than [SIDE] a side.
pub fn to_png(bytes: &[u8]) -> Option<Vec<u8>> {
    let img = image::load_from_memory(bytes).ok()?;
    let img = if img.width() > SIDE || img.height() > SIDE {
        img.thumbnail(SIDE, SIDE)
    } else {
        img
    };
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, ImageFormat::Png).ok()?;
    Some(out.into_inner())
}

/// The avatar `hash` as a local PNG for a notification, or `None` (no image then).
pub async fn local_png(session: Option<Session>, hash: &str) -> Option<PathBuf> {
    if !media::is_hash(hash) {
        return None;
    }
    let scope = session.as_ref()?.cache.clone()?;
    let file = png_path(&scope, hash);
    if tokio::fs::try_exists(&file).await.unwrap_or(false) {
        return Some(file);
    }
    let webp = tokio::time::timeout(FETCH_WAIT, media::serve(session, &format!("/{hash}")))
        .await
        .ok()?
        .ok()?;
    let png = tokio::task::spawn_blocking(move || to_png(&webp))
        .await
        .ok()??;
    if let Err(e) = media::store(&file, &png).await {
        tracing::debug!(error = %e, "notifications: avatar not cached");
        return None;
    }
    Some(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pulse_server::testing;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    use crate::api::Api;

    fn webp(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba([200, 40, 90, 255]));
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut out, ImageFormat::WebP)
            .unwrap();
        out.into_inner()
    }

    fn session(api: &Api, token: &str, cache: Option<&Path>) -> Option<Session> {
        Some(Session {
            api: api.clone(),
            token: token.into(),
            cache: cache.map(Path::to_path_buf),
            alive: Arc::new(AtomicBool::new(true)),
        })
    }

    #[test]
    fn a_webp_becomes_a_small_png() {
        let png = to_png(&webp(512, 256)).unwrap();
        assert_eq!(image::guess_format(&png).unwrap(), ImageFormat::Png);
        let img = image::load_from_memory(&png).unwrap();
        assert_eq!((img.width(), img.height()), (SIDE, SIDE / 2));
        // small ones keep their size
        let img = image::load_from_memory(&to_png(&webp(40, 40)).unwrap()).unwrap();
        assert_eq!((img.width(), img.height()), (40, 40));
    }

    #[test]
    fn not_an_image_is_none() {
        assert_eq!(to_png(b"not an image"), None);
    }

    #[tokio::test]
    async fn the_avatar_is_fetched_once_as_a_png_in_the_accounts_cache() {
        let app = testing::spawn().await;
        let (_, token) = testing::register(&app, "alex").await;
        let api = Api::new(&format!("http://{}", app.addr));
        let info = api
            .upload_emoji_media(&token, webp(300, 300))
            .await
            .unwrap();
        let dir = tempfile::tempdir().unwrap();

        let file = local_png(session(&api, &token, Some(dir.path())), &info.hash)
            .await
            .unwrap();
        assert_eq!(file, png_path(dir.path(), &info.hash));
        let img = image::open(&file).unwrap();
        assert_eq!((img.width(), img.height()), (SIDE, SIDE));

        // the server is gone: the PNG made before still serves
        let offline = Api::new("http://localhost:9");
        assert_eq!(
            local_png(session(&offline, &token, Some(dir.path())), &info.hash).await,
            Some(file)
        );
    }

    #[tokio::test]
    async fn without_an_avatar_the_notification_goes_without_one() {
        let dir = tempfile::tempdir().unwrap();
        let offline = Api::new("http://localhost:9");
        let hash = "ab0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcd";
        // signed out, no cache yet, not a hash, unreachable server: all None, none an error
        assert_eq!(local_png(None, hash).await, None);
        assert_eq!(local_png(session(&offline, "t", None), hash).await, None);
        assert_eq!(
            local_png(session(&offline, "t", Some(dir.path())), "../x").await,
            None
        );
        assert_eq!(
            local_png(session(&offline, "t", Some(dir.path())), hash).await,
            None
        );
        assert!(!png_path(dir.path(), hash).exists());
    }
}
