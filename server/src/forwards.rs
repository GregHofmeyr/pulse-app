//! Forwarding a message (#302): "Forward" posts a message you can see into another chat you can post in, with your
//! comment if you write one, as a new message of yours that says where the original was said, by whom and when, and
//! what it said. Forwarding a forward forwards its original.
//!
//! The original's text (and GIF) are kept as they were when forwarded: a later edit doesn't change what was forwarded.
//! Its files are not copied: a forward shows the original's, by reference (their signed links come through the
//! forward, `GET /messages/{forward}/attachments/urls`, for anyone who can see the forward), and its link previews are
//! read from it too. Once the original is deleted (or its chat is) the forward shows nothing of it, and its viewers are
//! told. The forward names a server channel's channel, never a DM or group.

use axum::extract::{Path, State};
use axum::routing::post;
use axum::{Json, Router};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ChannelId, MessageId, UserId};
use pulse_protocol::rest::{
    Attachment, Channel, ChannelKind, ForwardMessageRequest, Forwarded, Gif, Message, MessageKind,
};
use sqlx::{SqliteConnection, SqlitePool};

use crate::AppState;
use crate::access::{channel_for, load_channel};
use crate::auth::AuthUser;
use crate::error::{AppError, AppResult};
use crate::messages::routes::{Outgoing as Post, caption, deliver, load};
use crate::messages::{mentions, nonce};
use crate::servers::routes::is_server_member;

/// The forward's own stored content when its forwarder wrote no comment. Read back, it says more ([`stand_in`]).
pub const STAND_IN: &str = "↪ Forwarded a message";

/// The longest piece of the original a comment-less forward's text shows, in characters.
const STAND_IN_MAX: usize = 100;

/// How a forward with no comment reads as text (notifications, previews, older clients; #415): "↪ Forwarded: " and
/// the original's first line, "↪ Forwarded a GIF", or [`STAND_IN`] (files only, or once the original is gone: then it
/// says nothing of it). Worked out on read, never stored, so a deleted original's words go with it.
pub fn stand_in(view: &Forwarded) -> String {
    if view.unavailable {
        return STAND_IN.to_owned();
    }
    if let Some(line) = view.content.lines().map(str::trim).find(|l| !l.is_empty()) {
        let mut cut: String = line.chars().take(STAND_IN_MAX).collect();
        if line.chars().nth(STAND_IN_MAX).is_some() {
            cut.push('…');
        }
        return format!("↪ Forwarded: {cut}");
    }
    if view.gif.is_some() {
        return "↪ Forwarded a GIF".to_owned();
    }
    STAND_IN.to_owned()
}

/// How many of an original's forwards are told at a time when it is deleted ([`tell_deleted`]).
pub const TELL_MAX: i64 = 200;

pub fn router() -> Router<AppState> {
    Router::new().route("/messages/{id}/forward", post(forward))
}

/// A forward's row in `message_forwards` (all but the forward's own id).
#[derive(Clone, Debug)]
pub(crate) struct Stored {
    pub source_id: MessageId,
    pub source_channel_id: ChannelId,
    pub source_author_id: Option<UserId>,
    pub source_created_at: String,
    pub content: String,
    pub gif: Option<Gif>,
    pub has_comment: bool,
}

/// A forward on its way in: its row, and how it reads (`view`, with the original's files in the send's attachments).
pub(crate) struct Outgoing {
    pub stored: Stored,
    pub view: Forwarded,
}

/// Inside the send's transaction: the forward's row.
pub(crate) async fn store(
    tx: &mut SqliteConnection,
    message: MessageId,
    f: &Stored,
) -> AppResult<()> {
    let gif = f
        .gif
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(anyhow::Error::from)?;
    sqlx::query(
        "INSERT INTO message_forwards (message_id, source_id, source_channel_id, source_author_id, source_created_at,
         content, gif, has_comment) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(message.to_string())
    .bind(f.source_id.to_string())
    .bind(f.source_channel_id.to_string())
    .bind(f.source_author_id.map(|a| a.to_string()))
    .bind(&f.source_created_at)
    .bind(&f.content)
    .bind(&gif)
    .bind(f.has_comment)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

fn parse<T: std::str::FromStr>(s: &str) -> AppResult<T>
where
    T::Err: std::error::Error + Send + Sync + 'static,
{
    s.parse::<T>().map_err(|e| AppError::Internal(e.into()))
}

type Row = (
    String,
    String,
    String,
    Option<String>,
    String,
    String,
    Option<String>,
    bool,
);

fn stored_from(r: Row) -> AppResult<(MessageId, Stored)> {
    let (message_id, source, channel, author, created, content, gif, has_comment) = r;
    Ok((
        parse(&message_id)?,
        Stored {
            source_id: parse(&source)?,
            source_channel_id: parse(&channel)?,
            source_author_id: author.as_deref().map(parse).transpose()?,
            source_created_at: created,
            content,
            gif: gif.and_then(|g| serde_json::from_str(&g).ok()),
            has_comment,
        },
    ))
}

/// How a forward of `f` posted in `here` reads now, and the original's files: unavailable (nothing of it) once the
/// original or its chat is gone. A server channel's name comes along only when `here` is in that same server (#80):
/// servers are private, so a forward in a DM, a group or another server doesn't name a channel its readers may not be
/// in (the forwarder chose to share the words, not the server's insides). A reader who is in it sees the name from
/// their own copy of the channel.
async fn view(
    db: &SqlitePool,
    f: &Stored,
    here: ChannelId,
) -> AppResult<(Forwarded, Vec<Attachment>)> {
    let live: Option<(Option<String>,)> =
        sqlx::query_as("SELECT thread_root_id FROM messages WHERE id = ? AND deleted_at IS NULL")
            .bind(f.source_id.to_string())
            .fetch_optional(db)
            .await?;
    let ch = load_channel(db, f.source_channel_id).await?;
    let here_server = if here == f.source_channel_id {
        ch.as_ref().and_then(|c| c.server_id)
    } else {
        load_channel(db, here).await?.and_then(|c| c.server_id)
    };
    let (channel_kind, channel_name, server_id) = match &ch {
        Some(c) => (
            c.kind,
            c.server_id
                .filter(|s| here_server == Some(*s))
                .and(c.name.clone()),
            c.server_id,
        ),
        None => (ChannelKind::Text, None, None),
    };
    let mut out = Forwarded {
        message_id: f.source_id,
        channel_id: f.source_channel_id,
        channel_kind,
        channel_name,
        server_id,
        thread_root_id: None,
        author_id: f.source_author_id,
        created_at: f.source_created_at.clone(),
        content: String::new(),
        gif: None,
        embeds: vec![],
        unavailable: true,
    };
    let (Some((thread,)), Some(_)) = (live, ch) else {
        return Ok((out, vec![]));
    };
    out.thread_root_id = thread.as_deref().map(parse).transpose()?;
    out.content = f.content.clone();
    out.gif = f.gif.clone();
    out.unavailable = false;
    // The original's previews and files, read as a message of its own.
    let mut original = [skeleton(f)];
    crate::embeds::attach(db, &mut original).await?;
    crate::attachments::attach(db, &mut original).await?;
    let [original] = original;
    out.embeds = original.embeds;
    Ok((out, original.attachments))
}

/// The original as a bare message, for reading its previews and files.
fn skeleton(f: &Stored) -> Message {
    Message {
        id: f.source_id,
        channel_id: f.source_channel_id,
        author_id: f.source_author_id,
        kind: MessageKind::Normal,
        content: f.content.clone(),
        reply_to_id: None,
        created_at: f.source_created_at.clone(),
        edited_at: None,
        deleted: false,
        mentions: vec![],
        reactions: vec![],
        thread_root_id: None,
        quote: None,
        qoty_card: None,
        clip: None,
        build: None,
        embeds: vec![],
        embeds_hidden: false,
        call: None,
        pin: None,
        pin_line: None,
        wheel: None,
        poll: None,
        poll_ended: None,
        stage_share_line: None,
        gif: None,
        attachments: vec![],
        caption_is_fallback: false,
        achievement: None,
        deadlock: None,
        plan: None,
        plan_reminder: None,
        sports: None,
        f1: None,
        forwarded: None,
        group_line: None,
        member_line: None,
        game_invite: None,
        server_invite: None,
    }
}

/// Fill `forwarded` on every live forward in `msgs`, with the original's files as its attachments and, when its
/// forwarder wrote no comment, its content marked as the stand-in.
pub async fn attach(db: &SqlitePool, msgs: &mut [Message]) -> AppResult<()> {
    let ids: Vec<String> = msgs
        .iter()
        .filter(|m| !m.deleted && m.kind == MessageKind::Normal)
        .map(|m| m.id.to_string())
        .collect();
    if ids.is_empty() {
        return Ok(());
    }
    let mut q = sqlx::QueryBuilder::new(
        "SELECT message_id, source_id, source_channel_id, source_author_id, source_created_at, content, gif,
                has_comment FROM message_forwards WHERE message_id IN (",
    );
    let mut list = q.separated(", ");
    for id in &ids {
        list.push_bind(id);
    }
    q.push(")");
    let rows: Vec<Row> = q.build_query_as().fetch_all(db).await?;
    for row in rows {
        let (id, f) = stored_from(row)?;
        if let Some(m) = msgs.iter_mut().find(|m| m.id == id) {
            let (fwd, files) = view(db, &f, m.channel_id).await?;
            if !f.has_comment {
                m.content = stand_in(&fwd);
            }
            m.forwarded = Some(Box::new(fwd));
            m.attachments = files;
            m.caption_is_fallback = !f.has_comment;
        }
    }
    Ok(())
}

/// Whose files a message's file links are: a live forward's are its original's. `None`: its own.
pub(crate) async fn files_of(db: &SqlitePool, message: MessageId) -> AppResult<Option<MessageId>> {
    let source: Option<String> =
        sqlx::query_scalar("SELECT source_id FROM message_forwards WHERE message_id = ?")
            .bind(message.to_string())
            .fetch_optional(db)
            .await?;
    source.as_deref().map(parse).transpose()
}

/// After an original's delete (under its `in_order` guard): each of its forwards' viewers hear it now reads as
/// unavailable. The first [`TELL_MAX`] are told here; any more go on in a task, [`TELL_MAX`] at a time, each batch under
/// its own ordering guard (taken once the delete's is let go), so a much-forwarded message never holds the lock long and
/// every forward is still told (#415). A failure is logged (the delete has landed).
pub(crate) async fn tell_deleted(s: &AppState, original: MessageId) {
    let Some(mut after) = tell_batch(s, original, None).await else {
        return;
    };
    let s = s.clone();
    tokio::spawn(async move {
        loop {
            let _order = s.hub.in_order().await;
            match tell_batch(&s, original, Some(&after)).await {
                Some(last) => after = last,
                None => break,
            }
        }
    });
}

/// One batch of [`tell_deleted`]: up to [`TELL_MAX`] of the original's live forwards after `after` (by id), each told
/// with a MessageUpdated. The last id when the batch was full (there may be more), else `None`.
async fn tell_batch(s: &AppState, original: MessageId, after: Option<&str>) -> Option<String> {
    let ids: Vec<String> = match sqlx::query_scalar(
        "SELECT f.message_id FROM message_forwards f JOIN messages m ON m.id = f.message_id
         WHERE f.source_id = ? AND m.deleted_at IS NULL AND f.message_id > ?
         ORDER BY f.message_id LIMIT ?",
    )
    .bind(original.to_string())
    .bind(after.unwrap_or(""))
    .bind(TELL_MAX)
    .fetch_all(&s.db)
    .await
    {
        Ok(ids) => ids,
        Err(e) => {
            tracing::error!(error = ?e, %original, "couldn't find a deleted message's forwards");
            return None;
        }
    };
    let more = (ids.len() as i64 == TELL_MAX)
        .then(|| ids.last().cloned())
        .flatten();
    for id in ids {
        let Ok(id) = id.parse::<MessageId>() else {
            continue;
        };
        match load(&s.db, None, id).await {
            Ok(Some(message)) => {
                s.hub
                    .publish(&s.db, Event::MessageUpdated { message })
                    .await
            }
            Ok(None) => {}
            Err(e) => tracing::error!(error = ?e, %id, "couldn't load a forward to tell"),
        }
    }
    more
}

/// The chat a forward goes to, with a send's checks: there, `me` may post in it (a server's: a member), and it takes
/// plain messages (not a quotes or clips channel).
async fn target(s: &AppState, me: UserId, id: ChannelId) -> AppResult<Channel> {
    let ch = channel_for(&s.db, me, id).await?;
    crate::access::require_open(&ch)?; // archived (#303): read-only
    if !ch.kind.has_chat() && !ch.kind.is_private() {
        return Err(AppError::BadRequest(
            "messages can't be forwarded there".into(),
        ));
    }
    if let Some(server) = ch.server_id
        && !is_server_member(&s.db, server, me).await?
    {
        return Err(AppError::Forbidden);
    }
    if matches!(
        ch.kind,
        ChannelKind::Quotes | ChannelKind::Clips | ChannelKind::Builds
    ) {
        return Err(AppError::BadRequest(
            "messages can't be forwarded to a quotes, clips or builds channel".into(),
        ));
    }
    Ok(ch)
}

/// `POST /messages/{id}/forward`: the forward, as a send answers. The original must be a live message `me` can see (a
/// server's: as a member; 404 otherwise) that is plain words, files or a GIF (not a quote, clip, poll, plan or system
/// line: 400), and the target a chat they can post in. Rate limited as a send and as a forward.
async fn forward(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<MessageId>,
    Json(req): Json<ForwardMessageRequest>,
) -> AppResult<Json<Message>> {
    if !s.limits.forward_user.hit(&me.to_string()) || !s.limits.send_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    crate::media::writes_ok(&s.cfg)?;
    let src = load(&s.db, Some(me), id).await?.ok_or(AppError::NotFound)?;
    if src.deleted {
        return Err(AppError::NotFound);
    }
    let src_ch = channel_for(&s.db, me, src.channel_id).await?;
    if let Some(server) = src_ch.server_id
        && !is_server_member(&s.db, server, me).await?
    {
        return Err(AppError::NotFound);
    }
    // A forward names the kind of chat its original was in, and a client from before builds channels (#616) can't read
    // that kind: nothing from a builds channel is forwarded until every client can.
    if src_ch.kind == ChannelKind::Builds {
        return Err(AppError::BadRequest(
            "builds and their comments can't be forwarded yet".into(),
        ));
    }
    let stored = match &src.forwarded {
        // a forward's original, as it was forwarded
        Some(f) if f.unavailable => {
            return Err(AppError::BadRequest(
                "the forwarded message is no longer available".into(),
            ));
        }
        Some(f) => Stored {
            source_id: f.message_id,
            source_channel_id: f.channel_id,
            source_author_id: f.author_id,
            source_created_at: f.created_at.clone(),
            content: f.content.clone(),
            gif: f.gif.clone(),
            has_comment: false,
        },
        None => {
            // view-once files (#311) go to their recipients once: a forward would hand them on
            if src.attachments.iter().any(|a| a.view_once) {
                return Err(AppError::BadRequest(
                    "view-once files can't be forwarded".into(),
                ));
            }
            let special = src.kind != MessageKind::Normal
                || src.quote.is_some()
                || src.clip.is_some()
                || src.build.is_some()
                || src.poll.is_some()
                || src.plan.is_some();
            if special {
                return Err(AppError::BadRequest(
                    "only messages, files and GIFs can be forwarded".into(),
                ));
            }
            Stored {
                source_id: src.id,
                source_channel_id: src.channel_id,
                source_author_id: src.author_id,
                source_created_at: src.created_at.clone(),
                // files with no caption: nothing to say but the files
                content: if src.caption_is_fallback {
                    String::new()
                } else {
                    src.content.clone()
                },
                gif: src.gif.clone(),
                has_comment: false,
            }
        }
    };
    let ch = target(&s, me, req.channel_id).await?;
    nonce::check(req.nonce.as_deref())?;
    let comment = caption(&req.comment)?;
    let has_comment = !comment.is_empty();
    let stored = Stored {
        has_comment,
        ..stored
    };
    let (view, files) = view(&s.db, &stored, ch.id).await?;
    if view.unavailable {
        return Err(AppError::NotFound);
    }
    let content = if has_comment {
        comment
    } else {
        STAND_IN.to_string()
    };
    let mentioned = if has_comment {
        mentions::resolve(&s.db, &ch, me, &mentions::parse(&content)).await?
    } else {
        vec![]
    };
    deliver(
        &s,
        me,
        &ch,
        Post {
            content,
            reply_to_id: None,
            thread_root_id: None,
            mentioned,
            pinged: vec![],
            nonce: req.nonce,
            gif: None,
            attachment_ids: vec![],
            attachments: files,
            caption_is_fallback: !has_comment,
            forward: Some(Outgoing { stored, view }),
        },
    )
    .await
    .map(Json)
}
