//! Attachments (#57): files sent with messages in text channels, voice channels' chats, DMs and
//! groups. The client PUTs each file straight to storage on a signed URL; the server never
//! touches the bytes (nor decodes them: images arrive as WebP, converted by the client).

pub mod routes;
pub mod sweep;

use std::collections::HashMap;
use std::time::Duration;

use pulse_protocol::ids::{AttachmentId, ChannelId, MessageId, UserId};
use pulse_protocol::rest::{Attachment, AttachmentKind, Message, attachment_limits};
use sqlx::{SqliteConnection, SqlitePool};

use crate::error::{AppError, AppResult};

pub use attachment_limits::{MAX_BYTES, NAME_MAX_CHARS};

/// How long a signed upload or download URL works (the same as a clip's).
pub const URL_TTL: Duration = crate::clips::URL_TTL;
/// How long an upload that was never sent (and its object) is kept.
pub const PENDING_TTL_HOURS: i64 = 24;
/// Uploads one person may have started and not yet sent.
pub const OPEN_PER_USER: i64 = 50;
/// The largest width or height kept for a picture or video (anything else isn't plausible).
pub const MAX_SIDE: u32 = 16_384;
/// The longest length kept for a video or sound: a day.
pub const MAX_DURATION_MS: u32 = 24 * 60 * 60 * 1000;

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

/// An attachment's stored object, for deleting it: the row's key and which prefix it lives under.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stored {
    pub key: String,
    pub temp: bool,
}

impl Stored {
    pub fn object_key(&self) -> String {
        object_key(&self.key, self.temp)
    }
}

fn kind_of(s: &str) -> AttachmentKind {
    match s {
        "image" => AttachmentKind::Image,
        "video" => AttachmentKind::Video,
        "audio" => AttachmentKind::Audio,
        _ => AttachmentKind::File,
    }
}

/// `attachments` columns read into an [`Attachment`], in this order.
const COLS: &str =
    "id, name, content_type, kind, size, width, height, duration_ms, expires_at, purged_at";
type Row = (
    String,
    String,
    String,
    String,
    i64,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<String>,
    Option<String>,
);

fn to_attachment(r: Row, now: &str) -> AppResult<Attachment> {
    let (id, name, content_type, kind, size, width, height, duration_ms, expires_at, purged_at) = r;
    // Past its time it reads as expired at once; the sweep deletes the bytes within minutes.
    let expired = purged_at.is_some() || expires_at.as_deref().is_some_and(|e| e <= now);
    Ok(Attachment {
        id: id.parse().map_err(anyhow::Error::from)?,
        name,
        content_type,
        kind: kind_of(&kind),
        size: size as u64,
        width: width.map(|v| v as u32),
        height: height.map(|v| v as u32),
        duration_ms: duration_ms.map(|v| v as u32),
        expires_at,
        expired,
    })
}

/// Fill `attachments` (in their order) and `caption_is_fallback` on every live message in `msgs`.
pub async fn attach(db: &SqlitePool, msgs: &mut [Message]) -> AppResult<()> {
    let ids: Vec<String> = msgs
        .iter()
        .filter(|m| !m.deleted)
        .map(|m| m.id.to_string())
        .collect();
    if ids.is_empty() {
        return Ok(());
    }
    let mut q = sqlx::QueryBuilder::new(format!(
        "SELECT message_id, {COLS} FROM attachments WHERE message_id IN ("
    ));
    let mut list = q.separated(", ");
    for id in &ids {
        list.push_bind(id);
    }
    q.push(") ORDER BY message_id, position");
    let rows: Vec<(
        String,
        String,
        String,
        String,
        String,
        i64,
        Option<i64>,
        Option<i64>,
        Option<i64>,
        Option<String>,
        Option<String>,
    )> = q.build_query_as().fetch_all(db).await?;
    if rows.is_empty() {
        return Ok(());
    }
    let now = crate::db::now();
    let mut by_msg: HashMap<String, Vec<Attachment>> = HashMap::new();
    for (message_id, id, name, ct, kind, size, w, h, d, exp, purged) in rows {
        by_msg.entry(message_id).or_default().push(to_attachment(
            (id, name, ct, kind, size, w, h, d, exp, purged),
            &now,
        )?);
    }
    for m in msgs.iter_mut().filter(|m| !m.deleted) {
        if let Some(atts) = by_msg.remove(&m.id.to_string()) {
            m.caption_is_fallback = m.content == fallback_text(&atts);
            m.attachments = atts;
        }
    }
    Ok(())
}

/// Whether the first bytes are a WebP file's (`RIFF`, a size, `WEBP`). A signature check, not a
/// decode: the server never decodes anyone's image.
pub fn is_webp(head: &[u8]) -> bool {
    head.len() >= 12 && &head[0..4] == b"RIFF" && &head[8..12] == b"WEBP"
}

/// Check the uploads a send names, before its transaction: at most [`PER_MESSAGE`], each mine,
/// unsent, started for this channel less than [`PENDING_TTL_HOURS`] ago, and stored whole (its size
/// as declared; an image a WebP file). Anything else of anyone's is a 404, like a missing one.
pub async fn claim(
    s: &crate::AppState,
    me: UserId,
    channel: ChannelId,
    ids: &[AttachmentId],
) -> AppResult<Vec<Attachment>> {
    if ids.len() > attachment_limits::PER_MESSAGE {
        return Err(AppError::BadRequest(format!(
            "Up to {} files per message.",
            attachment_limits::PER_MESSAGE
        )));
    }
    let mut seen = std::collections::HashSet::new();
    if !ids.iter().all(|id| seen.insert(*id)) {
        return Err(AppError::BadRequest("A file is attached twice.".into()));
    }
    let r2 = storage(s)?;
    let since = crate::db::stamp(chrono::Utc::now() - chrono::Duration::hours(PENDING_TTL_HOURS));
    let now = crate::db::now();
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        let row: Option<(String, Option<String>, Option<String>, String)> = sqlx::query_as(
            "SELECT key, channel_id, upload_id, created_at FROM attachments
             WHERE id = ? AND user_id = ? AND message_id IS NULL",
        )
        .bind(id.to_string())
        .bind(me.to_string())
        .fetch_optional(&s.db)
        .await?;
        let Some((key, ch, upload_id, created)) = row else {
            return Err(AppError::NotFound);
        };
        if ch.as_deref() != Some(channel.to_string().as_str()) || created < since {
            return Err(AppError::NotFound);
        }
        let att: Row = sqlx::query_as(&format!("SELECT {COLS} FROM attachments WHERE id = ?"))
            .bind(id.to_string())
            .fetch_one(&s.db)
            .await?;
        let att = to_attachment(att, &now)?;
        // A big file's parts are joined by completing its upload first.
        if upload_id.is_some() {
            return Err(AppError::Conflict(
                "The upload didn't arrive. Try again.".into(),
            ));
        }
        let stored = Stored {
            key,
            temp: att.expires_at.is_some(),
        };
        let size = r2
            .head(&stored.object_key())
            .await?
            .ok_or_else(|| AppError::Conflict("The upload didn't arrive. Try again.".into()))?;
        if size != att.size {
            return Err(AppError::Conflict(
                "The upload didn't match. Try again.".into(),
            ));
        }
        if att.kind == AttachmentKind::Image {
            let head = r2
                .get_range(&stored.object_key(), 12)
                .await?
                .unwrap_or_default();
            if !is_webp(&head) {
                return Err(AppError::Conflict(
                    "That image didn't upload correctly. Try again.".into(),
                ));
            }
        }
        out.push(att);
    }
    Ok(out)
}

/// Inside the send's transaction: tie the claimed uploads to `message`, in order. One sent (or
/// cancelled) meanwhile is a 404 and the send rolls back.
pub async fn bind(
    tx: &mut SqliteConnection,
    me: UserId,
    message: MessageId,
    ids: &[AttachmentId],
) -> AppResult<()> {
    for (position, id) in ids.iter().enumerate() {
        let done = sqlx::query(
            "UPDATE attachments SET message_id = ?, position = ?
             WHERE id = ? AND user_id = ? AND message_id IS NULL",
        )
        .bind(message.to_string())
        .bind(position as i64)
        .bind(id.to_string())
        .bind(me.to_string())
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if done == 0 {
            return Err(AppError::NotFound);
        }
    }
    Ok(())
}

/// The objects of one message's files (for deleting the message).
pub async fn stored_of_message(
    c: &mut SqliteConnection,
    message: MessageId,
) -> AppResult<Vec<Stored>> {
    let rows: Vec<(String, Option<String>)> =
        sqlx::query_as("SELECT key, expires_at FROM attachments WHERE message_id = ?")
            .bind(message.to_string())
            .fetch_all(c)
            .await?;
    Ok(rows
        .into_iter()
        .map(|(key, exp)| Stored {
            key,
            temp: exp.is_some(),
        })
        .collect())
}

/// Every object of files started in `channels`, sent or not: what purging those channels (or a
/// whole server: #184) must delete. Call in the purge's transaction, before the rows lose their
/// channel; then [`remove_objects`] after the commit.
pub async fn stored_in_channels(
    c: &mut SqliteConnection,
    channels: &[ChannelId],
) -> AppResult<Vec<Stored>> {
    if channels.is_empty() {
        return Ok(vec![]);
    }
    let mut q =
        sqlx::QueryBuilder::new("SELECT key, expires_at FROM attachments WHERE channel_id IN (");
    let mut list = q.separated(", ");
    for ch in channels {
        list.push_bind(ch.to_string());
    }
    q.push(")");
    let rows: Vec<(String, Option<String>)> = q.build_query_as().fetch_all(c).await?;
    Ok(rows
        .into_iter()
        .map(|(key, exp)| Stored {
            key,
            temp: exp.is_some(),
        })
        .collect())
}

/// Delete each object, then its row. A failure is logged and its row kept: the sweep finds rows
/// whose message is gone or deleted (and unsent ones past their time) and tries again.
pub async fn remove_objects(s: &crate::AppState, stored: Vec<Stored>) {
    let Some(r2) = &s.r2 else { return };
    for st in stored {
        if let Err(e) = r2.delete(&st.object_key()).await {
            tracing::warn!(error = ?e, "couldn't delete a file's object; the sweeper will retry");
            continue;
        }
        if let Err(e) = sqlx::query("DELETE FROM attachments WHERE key = ?")
            .bind(&st.key)
            .execute(&s.db)
            .await
        {
            tracing::warn!(error = ?e, "couldn't delete a file's row");
        }
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
