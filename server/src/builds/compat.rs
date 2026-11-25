//! Clients older than protocol 5 and builds channels (#616). A version-4 client's `ChannelKind` has no catch-all, so a
//! builds channel anywhere in a frame it reads (a Ready, a server it joins, a channel event) fails the whole frame. Such
//! a client is kept away from builds channels:
//! - its Ready and the server snapshots it gets leave them out (with their read points and places in its own orders);
//! - the hub skips it for every event about one, its own included (`audience::route`: messages, comments, reactions,
//!   read points, thread seen points, inbox items, bookmarks and reminders of builds), and sends it a copy of its own
//!   channel order without them;
//! - REST answers to a request without [`pulse_protocol::PROTOCOL_HEADER`] 5 (every older client) leave them out: a
//!   server's channels, Recently deleted, the inbox and its Threads, search, bookmarks and reminders; a layout save
//!   needn't name them.
//!
//! Left as they are, since nothing in them breaks an older client: a server layout's placements (a channel it doesn't
//! hold is skipped) and the inbox's unread count (it may count a comment it won't list).

use std::collections::HashSet;

use axum::http::HeaderMap;
use pulse_protocol::gateway::{Event, Ready, ServerFrame, ServerSnapshot};
use pulse_protocol::ids::ChannelId;
use pulse_protocol::rest::{Channel, ChannelKind};
use sqlx::SqlitePool;

/// The first protocol version that reads builds channels.
pub const BUILDS_SINCE: u32 = 5;

/// Whether a client speaking `version` may be told about builds channels.
pub fn speaks_builds(version: u32) -> bool {
    version >= BUILDS_SINCE
}

/// The protocol a REST request's client speaks: its [`pulse_protocol::PROTOCOL_HEADER`], or 4 without one (every client
/// from before the header).
pub fn rest_version(headers: &HeaderMap) -> u32 {
    headers
        .get(pulse_protocol::PROTOCOL_HEADER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(BUILDS_SINCE - 1)
}

fn hidden(kind: ChannelKind) -> bool {
    matches!(kind, ChannelKind::Builds | ChannelKind::Unknown)
}

/// Drops the builds channels from `channels`; the ids it dropped.
pub fn strip_channels(channels: &mut Vec<Channel>) -> HashSet<ChannelId> {
    let gone: HashSet<ChannelId> = channels
        .iter()
        .filter(|c| hidden(c.kind))
        .map(|c| c.id)
        .collect();
    channels.retain(|c| !hidden(c.kind));
    gone
}

/// A Ready for an older client: no builds channels, nor their read points or places in a personal order.
pub fn strip_ready(r: &mut Ready) {
    let gone = strip_channels(&mut r.channels);
    if gone.is_empty() {
        return;
    }
    r.read_states.retain(|s| !gone.contains(&s.channel_id));
    for order in &mut r.channel_orders {
        order.uncategorised.retain(|c| !gone.contains(c));
        for cat in &mut order.categories {
            cat.channels.retain(|c| !gone.contains(c));
        }
    }
}

/// A server snapshot (`ServerJoined`, `ServerRestored`) for an older client: no builds channels, nor their places.
pub fn strip_snapshot(s: &mut ServerSnapshot) {
    let gone = strip_channels(&mut s.channels);
    s.layout.channels.retain(|p| !gone.contains(&p.channel_id));
}

/// What a client too old for builds gets of `frame`: the frame itself (`None`), its own copy without builds channels
/// (`Some(Some(_))`), or nothing (`Some(None)`). Events *in* a builds channel are found by the hub from the database
/// (`audience::route`); this covers what the frame itself carries.
pub fn for_old_client(frame: &ServerFrame) -> Option<Option<ServerFrame>> {
    let ServerFrame::Event(event) = frame else {
        return None;
    };
    match event {
        Event::ChannelCreated { channel }
        | Event::ChannelUpdated { channel }
        | Event::ChannelRestored { channel }
            if hidden(channel.kind) =>
        {
            Some(None)
        }
        Event::BuildsArchived { .. } | Event::BuildFavouriteChanged { .. } => Some(None),
        Event::ServerJoined { snapshot, .. } | Event::ServerRestored { snapshot }
            if snapshot.channels.iter().any(|c| hidden(c.kind)) =>
        {
            let mut copy = event.clone();
            if let Event::ServerJoined { snapshot, .. } | Event::ServerRestored { snapshot } =
                &mut copy
            {
                strip_snapshot(snapshot);
            }
            Some(Some(ServerFrame::Event(copy)))
        }
        _ => None,
    }
}

/// The channel an event for one person is about, when it is about one in a way a builds channel would matter.
pub fn own_event_channel(event: &Event) -> Option<ChannelId> {
    match event {
        Event::ReadStateUpdated { channel_id, .. } | Event::ThreadSeen { channel_id, .. } => {
            Some(*channel_id)
        }
        Event::InboxItemAdded { item, .. } => Some(item.message.channel_id),
        Event::BookmarkSaved { bookmark, .. } => bookmark.message.as_ref().map(|m| m.channel_id),
        Event::ReminderSaved { reminder, .. } => Some(reminder.channel_id),
        other => other.channel_id(),
    }
}

/// Whether `id` is a builds channel (deleted or not).
pub async fn is_builds(db: &SqlitePool, id: ChannelId) -> anyhow::Result<bool> {
    let kind: Option<String> = sqlx::query_scalar("SELECT kind FROM channels WHERE id = ?")
        .bind(id.to_string())
        .fetch_optional(db)
        .await?;
    Ok(kind.as_deref() == Some("builds"))
}

/// Which of `ids` are builds channels.
pub async fn builds_among(
    db: &SqlitePool,
    ids: impl IntoIterator<Item = ChannelId>,
) -> anyhow::Result<HashSet<ChannelId>> {
    let ids: HashSet<ChannelId> = ids.into_iter().collect();
    if ids.is_empty() {
        return Ok(HashSet::new());
    }
    let mut q =
        sqlx::QueryBuilder::new("SELECT id FROM channels WHERE kind = 'builds' AND id IN (");
    let mut list = q.separated(", ");
    for id in &ids {
        list.push_bind(id.to_string());
    }
    q.push(")");
    let rows: Vec<String> = q.build_query_scalar().fetch_all(db).await?;
    Ok(rows.into_iter().filter_map(|r| r.parse().ok()).collect())
}

/// What an older client gets instead of `event`, when that is a copy without builds channels: its own channel order
/// with them taken out (`None`: the event as it is, or nothing, as the hub decides).
pub async fn old_copy(db: &SqlitePool, event: &Event) -> anyhow::Result<Option<Event>> {
    let Event::ChannelOrderChanged {
        user_id,
        server_id,
        order: Some(order),
    } = event
    else {
        return Ok(None);
    };
    let named = order
        .uncategorised
        .iter()
        .chain(order.categories.iter().flat_map(|c| &c.channels))
        .copied();
    let gone = builds_among(db, named).await?;
    if gone.is_empty() {
        return Ok(None);
    }
    let mut order = order.clone();
    order.uncategorised.retain(|c| !gone.contains(c));
    for cat in &mut order.categories {
        cat.channels.retain(|c| !gone.contains(c));
    }
    Ok(Some(Event::ChannelOrderChanged {
        user_id: *user_id,
        server_id: *server_id,
        order: Some(order),
    }))
}
