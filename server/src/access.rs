//! The single access rule for channel-scoped data.

use pulse_protocol::ids::{ChannelId, ServerId, UserId};
use pulse_protocol::rest::{Channel, ChannelKind};
use sqlx::SqlitePool;

use crate::error::{AppError, AppResult};

type ChannelRow = (String, Option<String>, String, Option<String>, i64);

pub fn channel_from_row((id, server_id, kind, name, position): ChannelRow) -> AppResult<Channel> {
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
    })
}

pub const CHANNEL_COLS: &str = "id, server_id, kind, name, position";

pub async fn load_channel(db: &SqlitePool, id: ChannelId) -> AppResult<Option<Channel>> {
    let row: Option<ChannelRow> =
        sqlx::query_as(&format!("SELECT {CHANNEL_COLS} FROM channels WHERE id = ?"))
            .bind(id.to_string())
            .fetch_optional(db)
            .await?;
    row.map(channel_from_row).transpose()
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

/// The channel, if `user` may see it. Missing and forbidden look identical (404) so private
/// channels never leak their existence. Server channels are visible to every user.
pub async fn channel_for(db: &SqlitePool, user: UserId, id: ChannelId) -> AppResult<Channel> {
    let ch = load_channel(db, id).await?.ok_or(AppError::NotFound)?;
    if ch.kind.is_private() && !is_channel_member(db, user, id).await? {
        return Err(AppError::NotFound);
    }
    Ok(ch)
}
