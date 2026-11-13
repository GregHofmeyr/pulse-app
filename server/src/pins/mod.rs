//! Pinned messages (#178): anyone who can read and write a chat (a text channel, a DM, a group, a voice channel's
//! chat, and since #257 a quotes or clips channel, whose posts are its quotes or clips) may pin a message in it for
//! an hour, a day, a week or for good, and unpin it.
//!
//! Expiry: a pin past its `expires_at` is never shown (every read filters on the time), and a sweep every
//! [`SWEEP_EVERY`] deletes the overdue rows and tells the chat with `MessageUnpinned`. So a pin is gone from every
//! fresh read the moment it runs out, and from open clients within the sweep's interval, with no line or notification.

pub mod routes;

use std::collections::HashMap;
use std::time::Duration;

use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ChannelId, MessageId};
use pulse_protocol::rest::{ChannelKind, Message, Pin};
use sqlx::{SqliteConnection, SqlitePool};

use crate::AppState;
use crate::db::now;
use crate::error::AppResult;

pub use routes::router;

/// The most live pins a chat holds.
pub const MAX_PINS: i64 = 50;

/// How often overdue pins are swept.
pub const SWEEP_EVERY: Duration = Duration::from_secs(30);

/// What a pin's system line calls the pinned post in a chat of this kind ("alex pinned a quote"), or None where a
/// pin posts no line: every chat takes pins (quotes and clips channels too, #257), but a clips channel shows only
/// its clips, so a line there would never be seen (in its feed, by this client or an older one).
pub fn line_noun(kind: ChannelKind) -> Option<&'static str> {
    match kind {
        ChannelKind::Text | ChannelKind::Voice | ChannelKind::Dm | ChannelKind::Group => {
            Some("message")
        }
        ChannelKind::Quotes => Some("quote"),
        ChannelKind::Clips => None,
    }
}

/// `message_id, pinned_by, pinned_at, expires_at`.
pub(crate) type PinRow = (String, String, String, Option<String>);

pub(crate) fn pin_from_row((_, by, at, until): PinRow) -> AppResult<Pin> {
    Ok(Pin {
        pinned_by: by.parse().map_err(anyhow::Error::from)?,
        pinned_at: at,
        expires_at: until,
    })
}

/// Set each live message's live pin (and clear any other).
pub(crate) async fn attach(db: &SqlitePool, page: &mut [Message]) -> AppResult<()> {
    let ids: Vec<String> = page
        .iter()
        .filter(|m| !m.deleted)
        .map(|m| m.id.to_string())
        .collect();
    if ids.is_empty() {
        return Ok(());
    }
    let mut q = sqlx::QueryBuilder::new(
        "SELECT message_id, pinned_by, pinned_at, expires_at FROM pins \
         WHERE (expires_at IS NULL OR expires_at > ",
    );
    q.push_bind(now());
    q.push(") AND message_id IN (");
    let mut list = q.separated(", ");
    for id in &ids {
        list.push_bind(id);
    }
    q.push(")");
    let rows: Vec<PinRow> = q.build_query_as().fetch_all(db).await?;
    let mut by_id: HashMap<String, Pin> = HashMap::new();
    for r in rows {
        let id = r.0.clone();
        by_id.insert(id, pin_from_row(r)?);
    }
    for m in page.iter_mut() {
        m.pin = by_id.remove(&m.id.to_string());
    }
    Ok(())
}

/// A message is being deleted (in `tx`): its pin goes with it. True if it had a live one to announce.
pub(crate) async fn on_delete(tx: &mut SqliteConnection, id: MessageId) -> AppResult<bool> {
    let gone: Option<Option<String>> =
        sqlx::query_scalar("DELETE FROM pins WHERE message_id = ? RETURNING expires_at")
            .bind(id.to_string())
            .fetch_optional(&mut *tx)
            .await?;
    let at = now();
    Ok(matches!(gone, Some(until) if until.as_ref().is_none_or(|u| *u > at)))
}

/// Delete the pins whose time is up at `now` and tell their chats. Returns how many went.
pub async fn expire_overdue(s: &AppState, now: &str) -> AppResult<usize> {
    // In commit order with the pins and unpins the routes publish.
    let _order = s.hub.in_order().await;
    let gone: Vec<(String, String)> = sqlx::query_as(
        "DELETE FROM pins WHERE expires_at IS NOT NULL AND expires_at <= ? \
         RETURNING channel_id, message_id",
    )
    .bind(now)
    .fetch_all(&s.db)
    .await?;
    for (channel, message) in &gone {
        let channel_id: ChannelId = channel.parse().map_err(anyhow::Error::from)?;
        let message_id: MessageId = message.parse().map_err(anyhow::Error::from)?;
        s.hub
            .publish(
                &s.db,
                Event::MessageUnpinned {
                    channel_id,
                    message_id,
                },
            )
            .await;
    }
    Ok(gone.len())
}

/// Sweep overdue pins every [`SWEEP_EVERY`]. Call after binding the listener.
pub fn spawn_sweeper(s: AppState) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(SWEEP_EVERY);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            match expire_overdue(&s, &now()).await {
                Ok(0) => {}
                Ok(n) => tracing::debug!(n, "pins ran out"),
                Err(e) => tracing::warn!(error = %e, "pin sweep failed"),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pin_line_names_the_post_and_clips_get_none() {
        for k in [
            ChannelKind::Text,
            ChannelKind::Voice,
            ChannelKind::Dm,
            ChannelKind::Group,
        ] {
            assert_eq!(line_noun(k), Some("message"), "{k:?}");
        }
        assert_eq!(line_noun(ChannelKind::Quotes), Some("quote"));
        assert_eq!(line_noun(ChannelKind::Clips), None);
    }
}
