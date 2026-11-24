//! Clients older than protocol 5 and builds channels (#616). A version-4 client's `ChannelKind` has no catch-all, so a
//! builds channel anywhere in a frame it reads (a Ready, a server it joins, a channel event) fails the whole frame. Such
//! a client is never told a builds channel exists, nor anything that happens in one: its Ready and snapshots leave them
//! out, the hub skips it for events in one, and REST channel lists answer a request without
//! [`pulse_protocol::PROTOCOL_HEADER`] (every older client) as they would have before builds.

use std::collections::HashSet;

use axum::http::HeaderMap;
use pulse_protocol::gateway::{Event, Ready, ServerFrame, ServerSnapshot};
use pulse_protocol::ids::ChannelId;
use pulse_protocol::rest::{Channel, ChannelKind};

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
        Event::BuildsArchived { .. } => Some(None),
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
