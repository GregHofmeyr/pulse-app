//! Clips: short videos shared in `clips` channels, tagged with a game from the server's registry.

pub mod games;
pub mod routes;
pub mod sweep;

use std::collections::HashMap;
use std::time::Duration;

use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{GameId, UserId};
use pulse_protocol::rest::{Clip, ClipView, ClipViewPref, Message};
use sqlx::{SqliteConnection, SqlitePool};

use crate::error::{AppError, AppResult};

/// The most one clip's file can be (shown to people as "200 MB").
pub const MAX_BYTES: u64 = 200 * 1024 * 1024;
/// What a ticket accepts. The client keeps clips at or under this.
pub const MAX_DURATION_MS: u32 = 90_000;
/// What confirm accepts: the encoder and AAC padding round the length up a little.
pub const CONFIRM_SLACK_MS: u32 = 500;
/// Clips (posted plus open tickets) one person can start in a rolling hour.
pub const PER_HOUR: i64 = 20;
/// How long a signed upload or play URL works.
pub const URL_TTL: Duration = Duration::from_secs(3600);
/// How long an unconfirmed ticket (and its object) is kept.
pub const TICKET_TTL_HOURS: i64 = 24;
pub const CAPTION_MAX: usize = 300;
pub const MIN_SIDE: u32 = 16;
pub const MAX_SIDE: u32 = 7680;
/// Clips per page of a feed.
pub const PAGE: u32 = 30;

/// The R2 object key of a clip's media row.
pub fn object_key(video_key: &str) -> String {
    crate::media::r2::R2::object_key(crate::media::MediaKind::Clip, video_key)
}

/// The storage, or the 503 that says it isn't set up.
pub fn storage(s: &crate::AppState) -> AppResult<&crate::media::r2::R2> {
    s.r2.as_ref()
        .ok_or_else(|| AppError::Unavailable("Clips storage isn't set up on this server.".into()))
}

type ClipRow = (String, String, i64, i64, i64, i64, Option<String>);

/// Fill `clip` on every live clip message in `msgs` (`me` is whose view it is for the seen point;
/// events pass `None`).
pub async fn attach(db: &SqlitePool, me: Option<UserId>, msgs: &mut [Message]) -> AppResult<()> {
    let ids: Vec<String> = msgs
        .iter()
        .filter(|m| !m.deleted)
        .map(|m| m.id.to_string())
        .collect();
    if ids.is_empty() {
        return Ok(());
    }
    let mut q = sqlx::QueryBuilder::new(
        "SELECT c.message_id, c.thumb_hash, c.duration_ms, c.width, c.height, v.size, c.game_id
         FROM clips c JOIN media v ON v.hash = c.video_key
         WHERE c.message_id IN (",
    );
    let mut list = q.separated(", ");
    for id in &ids {
        list.push_bind(id);
    }
    q.push(")");
    let rows: Vec<ClipRow> = q.build_query_as().fetch_all(db).await?;
    let clip_ids: Vec<String> = rows.iter().map(|r| r.0.clone()).collect();
    let mut threads = crate::messages::threads::summaries(db, me, &clip_ids).await?;
    let mut by_msg: HashMap<String, Clip> = HashMap::new();
    for (message_id, thumb_hash, duration_ms, width, height, size, game_id) in rows {
        let game_id = game_id
            .map(|g| g.parse::<GameId>())
            .transpose()
            .map_err(anyhow::Error::from)?;
        let t = threads.remove(&message_id);
        by_msg.insert(
            message_id,
            Clip {
                thumb_hash,
                duration_ms: duration_ms as u32,
                width: width as u32,
                height: height as u32,
                size: size as u64,
                game_id,
                reply_count: t.as_ref().map_or(0, |t| t.reply_count),
                last_reply_id: t.as_ref().and_then(|t| t.last_reply_id),
                replies_seen: t.and_then(|t| t.replies_seen),
            },
        );
    }
    for m in msgs.iter_mut().filter(|m| !m.deleted) {
        m.clip = by_msg.remove(&m.id.to_string());
    }
    Ok(())
}

/// What deleting a clip leaves for after the commit: the object to remove from storage and the
/// replies' `MessageDeleted` events.
pub struct ClipDeleted {
    pub key: String,
    pub replies: Vec<Event>,
}

/// Part of deleting a clip's message (the soft delete is the caller's): its reactions and `clips`
/// row go and its replies are soft-deleted. The media row stays until the object is deleted.
pub async fn on_delete(
    tx: &mut SqliteConnection,
    message_id: pulse_protocol::ids::MessageId,
) -> AppResult<ClipDeleted> {
    let id = message_id.to_string();
    sqlx::query("DELETE FROM reactions WHERE message_id = ?")
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    let key: String =
        sqlx::query_scalar("DELETE FROM clips WHERE message_id = ? RETURNING video_key")
            .bind(&id)
            .fetch_one(&mut *tx)
            .await?;
    let replies = crate::messages::threads::soft_delete_replies(tx, message_id).await?;
    Ok(ClipDeleted { key, replies })
}

/// The caller's chosen view of each clips channel they changed from the default.
pub async fn views_for_ready(db: &SqlitePool, me: UserId) -> AppResult<Vec<ClipViewPref>> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT channel_id, view FROM clip_view_prefs WHERE user_id = ? ORDER BY channel_id",
    )
    .bind(me.to_string())
    .fetch_all(db)
    .await?;
    rows.into_iter()
        .map(|(channel, view)| {
            Ok(ClipViewPref {
                channel_id: channel.parse().map_err(anyhow::Error::from)?,
                view: match view.as_str() {
                    "list" => ClipView::List,
                    _ => ClipView::Wall,
                },
            })
        })
        .collect()
}

/// R2's price per GB-month past the free tier.
const USD_PER_GB_MONTH: f64 = 0.015;
const FREE_GB: f64 = 10.0;

/// What a server's live clips cost to keep: bytes (videos plus thumbnails), count and dollars a month.
pub fn est_monthly_usd(bytes: u64) -> f64 {
    bytes as f64 / 1e9 * USD_PER_GB_MONTH
}

/// A server's live clips: `(bytes, count)`, the video and thumbnail rows together.
pub async fn server_usage(
    db: &SqlitePool,
    server: pulse_protocol::ids::ServerId,
) -> AppResult<(u64, u32)> {
    let (bytes, count): (i64, i64) = sqlx::query_as(
        "SELECT COALESCE(SUM(v.size + COALESCE(t.size, 0)), 0), COUNT(*)
         FROM clips c
         JOIN messages m ON m.id = c.message_id AND m.deleted_at IS NULL
         JOIN channels ch ON ch.id = m.channel_id
         JOIN media v ON v.hash = c.video_key
         LEFT JOIN media t ON t.hash = c.thumb_hash
         WHERE ch.server_id = ?",
    )
    .bind(server.to_string())
    .fetch_one(db)
    .await?;
    Ok((bytes as u64, count as u32))
}

/// One server's line of the `clip-storage` report.
pub struct ServerUsage {
    pub name: String,
    pub count: u32,
    pub bytes: u64,
}

/// Every server's usage, by name (the `clip-storage` command).
pub async fn usage_by_server(db: &SqlitePool) -> AppResult<Vec<ServerUsage>> {
    let servers: Vec<(String, String)> =
        sqlx::query_as("SELECT id, name FROM servers ORDER BY name, id")
            .fetch_all(db)
            .await?;
    let mut out = Vec::new();
    for (id, name) in servers {
        let (bytes, count) = server_usage(db, id.parse().map_err(anyhow::Error::from)?).await?;
        out.push(ServerUsage { name, count, bytes });
    }
    Ok(out)
}

/// The `clip-storage` output: a line per server, then the total with the estimated cost.
pub fn format_storage_report(servers: &[ServerUsage]) -> String {
    let gb = |bytes: u64| bytes as f64 / 1e9;
    let mut lines: Vec<String> = servers
        .iter()
        .map(|s| format!("{}  {} clips  {:.2} GB", s.name, s.count, gb(s.bytes)))
        .collect();
    let count: u32 = servers.iter().map(|s| s.count).sum();
    let total = gb(servers.iter().map(|s| s.bytes).sum());
    let cost = (total - FREE_GB).max(0.0) * USD_PER_GB_MONTH;
    lines.push(format!(
        "total {count} clips  {total:.2} GB  ~${cost:.2}/month (after R2's 10 GB free tier; the backups bucket shares it)"
    ));
    lines.join("\n")
}
