//! Ghost viewers (spec §5) at the server's edges: admitting their LiveKit connections, and taking
//! them out (state, counts, LiveKit) when the rules say they must go. Ghost identities are only
//! ever written to the server log, never to the database or to another client.

use livekit_protocol::ParticipantInfo;
use pulse_protocol::ids::{ChannelId, ServerId, UserId};
use pulse_protocol::rest::StreamRole;

use super::state::Purged;
use crate::AppState;
use crate::access::load_channel;
use crate::auth::session::{self, GRANT_ATTRIBUTE};

/// Ghosts taken out of the state: everyone gets the new counts, and their connections (`#v`
/// and `#a`) are removed from LiveKit in the background. `why` is for the log.
pub async fn removed(s: &AppState, purged: Purged, why: &'static str) {
    for (channel, streamer, _) in &purged.counts {
        super::routes::publish_audience(s, *channel, *streamer).await;
    }
    for (channel, id) in purged.removed {
        tracing::info!(channel = %channel, ghost = %id, why, "ghost removed");
        let s = s.clone();
        tokio::spawn(async move {
            let identities = [format!("{id}#v"), format!("{id}#a")];
            crate::voice::revoke::remove_identities(&s, &format!("{channel}~s"), &identities).await;
        });
    }
}

/// A stream in `channel` is voice-only now: no ghost may stay in its stream room (a ticket can
/// subscribe to anything there).
pub async fn voice_only(s: &AppState, channel: ChannelId) {
    if s.streams.ghosts_allowed(channel) {
        return;
    }
    let purged = s.streams.purge_ghosts(|g| g.channel == channel);
    removed(s, purged, "voice_only").await;
}

/// `user` joined `channel`'s call: their ghosts there go (their client reconnects as themselves).
pub async fn joined_voice(s: &AppState, channel: ChannelId, user: UserId) {
    let purged = s
        .streams
        .purge_ghosts(|g| g.channel == channel && g.user == user);
    removed(s, purged, "joined_voice").await;
}

/// The channel is gone: so are its ghosts.
pub async fn channel_gone(s: &AppState, channel: ChannelId) {
    let purged = s.streams.purge_ghosts(|g| g.channel == channel);
    removed(s, purged, "channel_deleted").await;
}

/// Sessions were revoked (logout, signing out other devices, a password change): ghosts whose
/// login session is no longer live go.
pub async fn sessions_revoked(s: &AppState) {
    let mut dead = Vec::new();
    for h in s.streams.ghost_sessions() {
        match session::is_live(&s.db, &h).await {
            Ok(false) => dead.push(h),
            Ok(true) => {}
            Err(e) => tracing::warn!(error = %e, "couldn't check a ghost's session"),
        }
    }
    if dead.is_empty() {
        return;
    }
    let purged = s
        .streams
        .purge_ghosts(|g| dead.iter().any(|d| d == g.session));
    removed(s, purged, "session_revoked").await;
}

/// `user` left or was removed from `server`: their stream-room connections in its channels go,
/// ghost and named. A join they make later is refused anyway (`voice::revoke::admit`). For the
/// leave / remove-member route when there is one (no such route yet).
pub async fn member_removed(s: &AppState, server: ServerId, user: UserId) {
    let mut channels = Vec::new();
    for c in s.streams.all().iter().map(|i| i.channel_id) {
        if !channels.contains(&c) {
            channels.push(c);
        }
    }
    let mut theirs = Vec::new();
    for c in channels {
        if let Ok(Some(ch)) = load_channel(&s.db, c).await
            && ch.server_id == Some(server)
        {
            theirs.push(c);
        }
    }
    let purged = s
        .streams
        .purge_ghosts(|g| g.user == user && theirs.contains(&g.channel));
    removed(s, purged, "left_server").await;
    for c in theirs {
        for (streamer, viewers) in s.streams.remove_viewer_everywhere(c, user) {
            super::routes::publish_viewers(s, c, streamer, viewers).await;
        }
        let named = [format!("{user}#v"), format!("{user}#a")];
        crate::voice::revoke::remove_identities(s, &format!("{c}~s"), &named).await;
    }
}

/// The join check for a ghost connection (`g_<id>#v` / `#a`) in `channel`'s stream room: its
/// grant is that ghost's, ghosts are allowed there now, the login session is still live, and
/// the user may still be in that channel.
pub async fn admit(
    s: &AppState,
    channel: ChannelId,
    id: &str,
    role: StreamRole,
    p: &ParticipantInfo,
) -> anyhow::Result<bool> {
    let grant = p.attributes.get(GRANT_ATTRIBUTE).map(String::as_str);
    let Some((user, login)) = s.streams.ghost_admit(channel, id, role, grant) else {
        return Ok(false);
    };
    if !session::is_live(&s.db, &login).await? {
        return Ok(false);
    }
    let Some(ch) = load_channel(&s.db, channel).await? else {
        return Ok(false);
    };
    if !crate::voice::routes::may_join(&s.db, user, &ch).await? {
        return Ok(false);
    }
    // its `#v` is the watch session: from now on it counts
    if role == StreamRole::V {
        for (streamer, _) in s.streams.ghost_connected(channel, id, &p.sid) {
            super::routes::publish_audience(s, channel, streamer).await;
        }
    }
    Ok(true)
}

/// A ghost connection left `channel`'s stream room: its `#v` ends the ghost (new counts out).
pub async fn left(s: &AppState, channel: ChannelId, id: &str, role: StreamRole, sid: &str) {
    for (streamer, _) in s.streams.ghost_left(channel, id, role, sid) {
        super::routes::publish_audience(s, channel, streamer).await;
    }
}
