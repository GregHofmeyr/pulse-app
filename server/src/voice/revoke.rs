//! When sessions are revoked, remove the LiveKit participants they admitted (voice rooms and
//! stream rooms). Best-effort: the REST/gateway revocation has already happened, so a LiveKit
//! that is down or slow is logged and the logout still succeeds.
//!
//! Removing a participant does not invalidate its JWT (LiveKit OSS, checked on v1.13.10: the
//! `revoke_token_ts` field is ignored, and LiveKit keeps refreshing a connected participant's
//! token), so a removed device could rejoin with the token it holds. [`admit`] closes that: the
//! join webhook removes again any participant whose room + identity has no grant held by a live
//! session.

use std::time::Duration;

use livekit_api::services::room::RoomClient;
use livekit_api::services::{ServerError, ServiceError};

use crate::AppState;
use crate::auth::session::{self, LiveKitGrant};

/// Upper bound on how long a logout waits for LiveKit (all removals run in parallel).
const BUDGET: Duration = Duration::from_secs(2);

/// A LiveKit join webhook for `identity` in `room`: true if a live session holds the grant for
/// it (record presence as usual). Otherwise the participant is removed in the background and
/// false is returned (record nothing). Covers a revoked device rejoining with its old JWT, a
/// token whose session went away between minting and joining, and an expired session.
pub async fn admit(s: &AppState, room: &str, identity: &str) -> anyhow::Result<bool> {
    if session::grant_is_live(&s.db, room, identity).await? {
        return Ok(true);
    }
    tracing::warn!(
        room,
        identity,
        "LiveKit join with no live session behind it; removing"
    );
    let s = s.clone();
    let grant = LiveKitGrant {
        room: room.to_owned(),
        identity: identity.to_owned(),
    };
    tokio::spawn(async move { remove_participants(&s, &[grant]).await });
    Ok(false)
}

/// Ask LiveKit to remove every `grants` participant. Never fails; problems are logged.
pub async fn remove_participants(s: &AppState, grants: &[LiveKitGrant]) {
    if grants.is_empty() {
        return;
    }
    // Default options (`revoke_token_ts` 0): v1.13.10 ignores the field. A LiveKit that starts
    // honouring its documented default (revoke this identity's tokens until now + 1 min) would lock
    // the caller's other devices out of a shared identity for about a minute: re-check on upgrades.
    let client = RoomClient::with_api_key(
        &s.cfg.livekit_api_url,
        &s.cfg.livekit_key,
        &s.cfg.livekit_secret,
    )
    .with_failover(false)
    .with_request_timeout(BUDGET);
    let calls = futures::future::join_all(grants.iter().map(|g| {
        let client = &client;
        async move {
            match client.remove_participant(&g.room, &g.identity).await {
                Ok(()) => tracing::info!(room = %g.room, identity = %g.identity, "removed revoked session's LiveKit participant"),
                // Already left (or never joined with that token): nothing to do.
                Err(ServiceError::Twirp(ServerError::Twirp(e))) if e.code == "not_found" => {
                    tracing::debug!(room = %g.room, identity = %g.identity, "revoked session's participant not in room");
                }
                Err(e) => tracing::warn!(error = %e, room = %g.room, identity = %g.identity, "couldn't remove revoked session's LiveKit participant"),
            }
        }
    }));
    if tokio::time::timeout(BUDGET, calls).await.is_err() {
        tracing::warn!(
            count = grants.len(),
            "LiveKit didn't answer in time; revoked sessions' participants may still be connected"
        );
    }
}
