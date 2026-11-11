//! Attachments (#57): files sent with messages in text channels, voice channels' chats, DMs and
//! groups. The client PUTs each file straight to storage on a signed URL; the server never
//! touches the bytes (nor decodes them: images arrive as WebP, converted by the client).

pub mod routes;

use std::time::Duration;

use pulse_protocol::rest::{Attachment, AttachmentKind, attachment_limits};

use crate::error::{AppError, AppResult};

pub use attachment_limits::{MAX_BYTES, NAME_MAX_CHARS};

/// How long a signed upload or download URL works (the same as a clip's).
pub const URL_TTL: Duration = crate::clips::URL_TTL;
/// How long an upload that was never sent (and its object) is kept.
pub const PENDING_TTL_HOURS: i64 = 24;
/// Uploads one person may have started and not yet sent.
pub const OPEN_PER_USER: i64 = 50;

/// The longest extension [`clean_name`] keeps when it has to shorten a name. Anything longer
/// isn't an extension worth saving.
const EXT_MAX_CHARS: usize = 16;

/// The R2 object of an attachment's `key`: separate prefixes, so storage can be reported by kind
/// and a lifecycle rule can back up the expiry of temporary sends.
pub fn object_key(key: &str, temp: bool) -> String {
    if temp {
        format!("temp/{key}")
    } else {
        format!("attachments/{key}")
    }
}

/// The storage, or the 503 that says it isn't set up.
pub fn storage(s: &crate::AppState) -> AppResult<&crate::media::r2::R2> {
    s.r2.as_ref()
        .ok_or_else(|| AppError::Unavailable("File storage isn't set up on this server.".into()))
}

/// The content type an upload is stored with, and its kind, from the type the client declared
/// (parameters and case ignored). Media types on the allow-list keep theirs; anything else is a
/// plain file (`application/octet-stream`), so a stored object never claims to be, say, HTML.
/// An image that isn't WebP is a 400: the client converts every image before uploading it.
pub fn stored_type(declared: &str) -> AppResult<(&'static str, AttachmentKind)> {
    const MEDIA: &[(&str, AttachmentKind)] = &[
        ("image/webp", AttachmentKind::Image),
        ("video/mp4", AttachmentKind::Video),
        ("video/webm", AttachmentKind::Video),
        ("video/quicktime", AttachmentKind::Video),
        ("audio/mpeg", AttachmentKind::Audio),
        ("audio/ogg", AttachmentKind::Audio),
        ("audio/wav", AttachmentKind::Audio),
        ("audio/flac", AttachmentKind::Audio),
        ("audio/aac", AttachmentKind::Audio),
        ("audio/mp4", AttachmentKind::Audio),
        ("audio/webm", AttachmentKind::Audio),
    ];
    let essence = declared
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if let Some(&(t, kind)) = MEDIA.iter().find(|(t, _)| *t == essence) {
        return Ok((t, kind));
    }
    if essence.starts_with("image/") {
        return Err(AppError::BadRequest("Images must be WebP.".into()));
    }
    Ok(("application/octet-stream", AttachmentKind::File))
}

/// How a kind is stored in `attachments.kind`.
pub fn kind_str(kind: AttachmentKind) -> &'static str {
    match kind {
        AttachmentKind::Image => "image",
        AttachmentKind::Video => "video",
        AttachmentKind::Audio => "audio",
        AttachmentKind::File => "file",
    }
}

/// Characters that reorder text: an override can make `exe.pdf` look like `fdp.exe`.
fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// A file name as shown and saved: the last path segment only, with no control or bidi
/// characters, no leading dots (no hidden files) and at most [`NAME_MAX_CHARS`] characters,
/// keeping the extension when it has to cut. `None` when nothing is left.
pub fn clean_name(name: &str) -> Option<String> {
    let last = name.rsplit(['/', '\\']).next().unwrap_or_default();
    let kept: String = last
        .chars()
        .filter(|c| !c.is_control() && !is_bidi_control(*c))
        .collect();
    let name = kept.trim().trim_start_matches('.').trim_start();
    if name.is_empty() {
        return None;
    }
    if name.chars().count() <= NAME_MAX_CHARS {
        return Some(name.to_string());
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((stem, ext))
            if !stem.is_empty() && !ext.is_empty() && ext.chars().count() <= EXT_MAX_CHARS =>
        {
            (stem, Some(ext))
        }
        _ => (name, None),
    };
    Some(match ext {
        Some(ext) => {
            let room = NAME_MAX_CHARS - ext.chars().count() - 1;
            let stem: String = stem.chars().take(room).collect();
            format!("{}.{ext}", stem.trim_end())
        }
        None => name.chars().take(NAME_MAX_CHARS).collect(),
    })
}

/// The stand-in `content` of a message sent with files and no caption, so older clients (and
/// notifications) never show a blank message: one file by name, else how many.
pub fn fallback_text(atts: &[Attachment]) -> String {
    match atts {
        [] => String::new(),
        [one] => format!("📎 {}", one.name),
        _ if atts.iter().all(|a| a.kind == AttachmentKind::Image) => {
            format!("📎 {} images", atts.len())
        }
        _ => format!("📎 {} files", atts.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pulse_protocol::ids::AttachmentId;

    fn att(name: &str, kind: AttachmentKind) -> Attachment {
        Attachment {
            id: AttachmentId::new(),
            name: name.into(),
            content_type: "application/octet-stream".into(),
            kind,
            size: 1,
            width: None,
            height: None,
            duration_ms: None,
            expires_at: None,
            expired: false,
        }
    }

    #[test]
    fn fallback_text_names_one_file_and_counts_several() {
        use AttachmentKind::*;
        assert_eq!(fallback_text(&[]), "");
        assert_eq!(fallback_text(&[att("report.pdf", File)]), "📎 report.pdf");
        assert_eq!(fallback_text(&[att("cat.webp", Image)]), "📎 cat.webp");
        let images = [att("a", Image), att("b", Image), att("c", Image)];
        assert_eq!(fallback_text(&images), "📎 3 images");
        assert_eq!(
            fallback_text(&[att("a", Image), att("b", File)]),
            "📎 2 files"
        );
        assert_eq!(
            fallback_text(&[att("a", Video), att("b", Audio)]),
            "📎 2 files"
        );
    }

    #[test]
    fn stored_type_keeps_allowed_media_and_flattens_the_rest() {
        use AttachmentKind::*;
        let ok = |t| stored_type(t).unwrap();
        assert_eq!(ok("image/webp"), ("image/webp", Image));
        assert_eq!(ok("Video/MP4"), ("video/mp4", Video));
        assert_eq!(ok("video/quicktime"), ("video/quicktime", Video));
        assert_eq!(ok("audio/ogg; codecs=opus"), ("audio/ogg", Audio));
        assert_eq!(ok("audio/flac"), ("audio/flac", Audio));
        for t in [
            "text/html",
            "application/pdf",
            "",
            "video/x-matroska",
            "imagewebp",
        ] {
            assert_eq!(ok(t), ("application/octet-stream", File), "{t}");
        }
        for t in ["image/png", "image/svg+xml", " IMAGE/JPEG "] {
            match stored_type(t) {
                Err(AppError::BadRequest(m)) => assert_eq!(m, "Images must be WebP."),
                other => panic!("{t}: {other:?}"),
            }
        }
    }

    #[test]
    fn object_keys_are_namespaced_by_lifetime() {
        assert_eq!(object_key("ab", false), "attachments/ab");
        assert_eq!(object_key("ab", true), "temp/ab");
    }

    #[test]
    fn clean_name_drops_paths_controls_and_overrides() {
        assert_eq!(
            clean_name("../../a\u{202e}gpj.exe").as_deref(),
            Some("agpj.exe")
        );
        assert_eq!(clean_name("C:\\Users\\x\\b.txt").as_deref(), Some("b.txt"));
        assert_eq!(clean_name("  ..hidden \n").as_deref(), Some("hidden"));
        assert_eq!(clean_name("a\u{2067}b\u{0}c").as_deref(), Some("abc"));
        for bad in ["", "  ", "dir/", "..", "\u{202e}"] {
            assert_eq!(clean_name(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn clean_name_cuts_long_names_keeping_the_extension() {
        let long = format!("{}.txt", "n".repeat(296));
        let cut = clean_name(&long).unwrap();
        assert_eq!(cut.chars().count(), NAME_MAX_CHARS);
        assert!(cut.ends_with(".txt"), "{cut}");
        // counted in characters, not bytes
        let wide = format!("{}.zip", "é".repeat(250));
        let cut = clean_name(&wide).unwrap();
        assert_eq!(cut.chars().count(), NAME_MAX_CHARS);
        assert!(cut.ends_with(".zip"));
        // no extension worth keeping: a plain cut
        let plain = "x".repeat(300);
        assert_eq!(clean_name(&plain).unwrap().chars().count(), NAME_MAX_CHARS);
        let exactly = "y".repeat(NAME_MAX_CHARS);
        assert_eq!(clean_name(&exactly).as_deref(), Some(exactly.as_str()));
    }
}
