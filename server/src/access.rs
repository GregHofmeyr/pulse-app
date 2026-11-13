//! The single access rule for channel-scoped data.

use pulse_protocol::ids::{CategoryId, ChannelId, ServerId, UserId};
use pulse_protocol::rest::{Channel, ChannelKind};
use sqlx::{SqliteConnection, SqlitePool};

use crate::error::{AppError, AppResult};

pub type ChannelRow = (
    String,
    Option<String>,
    String,
    Option<String>,
    i64,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<i64>,
);

pub fn channel_from_row(
    (id, server_id, kind, name, position, category_id, icon_hash, icon_still, bitrate): ChannelRow,
) -> AppResult<Channel> {
    Ok(Channel {
        id: id.parse().map_err(anyhow::Error::from)?,
        server_id: server_id
            .map(|s| s.parse::<ServerId>())
            .transpose()
            .map_err(anyhow::Error::from)?,
        kind: ChannelKind::parse(&kind)
            .ok_or_else(|| anyhow::anyhow!("bad channel kind {kind}"))?,
        name,
        position,
        category_id: category_id
            .map(|c| c.parse::<CategoryId>())
            .transpose()
            .map_err(anyhow::Error::from)?,
        icon_hash,
        icon_still,
        // only ever one of the presets (#75); anything else reads as Default
        bitrate: bitrate.and_then(|b| u32::try_from(b).ok()),
    })
}

/// The columns [`channel_from_row`] reads, from `channels` (unaliased): a group's icon (#56) comes
/// with its still, for an animated one; a voice channel's bitrate (#75) last.
pub const CHANNEL_COLS: &str = "id, server_id, kind, name, position, category_id, icon_hash, \
     (SELECT m.still_hash FROM media m WHERE m.hash = icon_hash) AS icon_still, bitrate";

/// The ids of the servers in their owner's Recently deleted (#184), as a subquery: a channel whose
/// `server_id` is in it is gone with its server.
pub const DELETED_SERVERS: &str = "(SELECT id FROM servers WHERE deleted_at IS NOT NULL)";

/// The channel, unless there is none, it is in Recently deleted, or its server is: a deleted
/// channel is gone for everything but its own Recently deleted routes (`servers::channels`), and a
/// deleted server's channels for everything but its purge (`servers::deleted`).
pub async fn load_channel(db: &SqlitePool, id: ChannelId) -> AppResult<Option<Channel>> {
    let row: Option<ChannelRow> = sqlx::query_as(&format!(
        "SELECT {CHANNEL_COLS} FROM channels WHERE id = ? AND deleted_at IS NULL
         AND (server_id IS NULL OR server_id NOT IN {DELETED_SERVERS})"
    ))
    .bind(id.to_string())
    .fetch_optional(db)
    .await?;
    row.map(channel_from_row).transpose()
}

/// Whether [`load_channel`] would find `id`, asked inside a write transaction.
pub async fn channel_is_live_in(db: &mut SqliteConnection, id: ChannelId) -> sqlx::Result<bool> {
    let hit: Option<i64> = sqlx::query_scalar(&format!(
        "SELECT 1 FROM channels WHERE id = ? AND deleted_at IS NULL
         AND (server_id IS NULL OR server_id NOT IN {DELETED_SERVERS})"
    ))
    .bind(id.to_string())
    .fetch_optional(&mut *db)
    .await?;
    Ok(hit.is_some())
}

/// [`load_channel`]'s rule again inside a write transaction (#109), as `server_live_in` is for
/// servers (#287): 404 unless the channel is still there. Under `begin_write`'s lock a delete
/// can't commit between this and the transaction's own commit, so a write that passed its early
/// check just before its channel (or the channel's server) was deleted, or a group went with its
/// last member, writes nothing and publishes nothing.
pub async fn channel_live_in(db: &mut SqliteConnection, id: ChannelId) -> AppResult<()> {
    if channel_is_live_in(db, id).await? {
        Ok(())
    } else {
        Err(AppError::NotFound)
    }
}

pub async fn is_channel_member(
    db: &SqlitePool,
    user: UserId,
    channel: ChannelId,
) -> AppResult<bool> {
    let hit: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM channel_members WHERE channel_id = ? AND user_id = ?")
            .bind(channel.to_string())
            .bind(user.to_string())
            .fetch_optional(db)
            .await?;
    Ok(hit.is_some())
}

/// Whether `user` may read `ch`: a DM or group only if they are in it; a server channel always.
/// Everything that reads messages (listing, search) goes through this one rule.
pub async fn can_read(db: &SqlitePool, user: UserId, ch: &Channel) -> AppResult<bool> {
    Ok(!ch.kind.is_private() || is_channel_member(db, user, ch.id).await?)
}

/// The channel, if `user` may see it. Missing and forbidden look identical (404) so private
/// channels never leak their existence. Server channels are visible to every user.
pub async fn channel_for(db: &SqlitePool, user: UserId, id: ChannelId) -> AppResult<Channel> {
    let ch = load_channel(db, id).await?.ok_or(AppError::NotFound)?;
    if !can_read(db, user, &ch).await? {
        return Err(AppError::NotFound);
    }
    Ok(ch)
}

/// The ids of every channel holding messages that `user` may read, as "everywhere" search sees
/// it (#331): each chat channel of every live server they are a member of (what
/// [`readable_server_channels`] gives for each), and every DM, group and notes they are in
/// ([`can_read`]). Never a deleted channel, a deleted server's, or a conversation they left.
pub async fn readable_channel_ids_everywhere(
    db: &SqlitePool,
    user: UserId,
) -> AppResult<Vec<String>> {
    // Server channels: only the kinds with a chat (kept in Rust, as `readable_server_channels`
    // keeps them). Conversations: only the ones you're in, the one rule `can_read` applies.
    let rows: Vec<(String, String)> = sqlx::query_as(&format!(
        "SELECT c.id, c.kind FROM channels c
         WHERE c.deleted_at IS NULL
           AND c.server_id IN (SELECT server_id FROM server_members WHERE user_id = ?1)
           AND c.server_id NOT IN {DELETED_SERVERS}
         UNION ALL
         SELECT c.id, c.kind FROM channel_members cm JOIN channels c ON c.id = cm.channel_id
         WHERE cm.user_id = ?1 AND c.deleted_at IS NULL AND c.server_id IS NULL"
    ))
    .bind(user.to_string())
    .fetch_all(db)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for (id, kind) in rows {
        let kind =
            ChannelKind::parse(&kind).ok_or_else(|| anyhow::anyhow!("bad channel kind {kind}"))?;
        if kind.has_chat() || kind.is_private() {
            out.push(id);
        }
    }
    Ok(out)
}

/// The channels of `server` that hold messages and `user` may read ([`can_read`]), in order.
/// 404 when there is no such server (or it is deleted).
pub async fn readable_server_channels(
    db: &SqlitePool,
    user: UserId,
    server: ServerId,
) -> AppResult<Vec<Channel>> {
    crate::servers::routes::server_exists(db, server).await?;
    let rows: Vec<ChannelRow> = sqlx::query_as(&format!(
        "SELECT {CHANNEL_COLS} FROM channels WHERE server_id = ? AND deleted_at IS NULL
         ORDER BY position, id"
    ))
    .bind(server.to_string())
    .fetch_all(db)
    .await?;
    let mut out = Vec::new();
    for ch in rows.into_iter().map(channel_from_row) {
        let ch = ch?;
        if ch.kind.has_chat() && can_read(db, user, &ch).await? {
            out.push(ch);
        }
    }
    Ok(out)
}
