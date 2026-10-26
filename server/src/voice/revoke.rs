//! When sessions are revoked, remove the LiveKit participants they admitted (voice rooms and
//! stream rooms). Best-effort: the REST/gateway revocation has already happened, so a LiveKit
//! that is down or slow is logged and the logout still succeeds.
//!
//! Removing a participant does not invalidate its JWT (LiveKit OSS, checked on v1.13.10: the
//! `revoke_token_ts` field is ignored, and LiveKit keeps refreshing a connected participant's
//! token), so a removed device could rejoin with the token it holds. [`admit`] closes that: the
//! join webhook removes again any participant whose grant (the `pulse_grant` attribute every
//! token we issue carries; LiveKit keeps it on refreshed tokens) isn't a live session's grant
//! for that room + identity. Participants can't change it: our tokens don't grant
//! `canUpdateOwnMetadata`.

use std::time::Duration;

use livekit_api::services::room::RoomClient;
use livekit_api::services::{ServerError, ServiceError};
use livekit_protocol::ParticipantInfo;

use crate::AppState;
use crate::auth::session::{self, GRANT_ATTRIBUTE, LiveKitParticipant};

/// Upper bound on how long a logout waits for LiveKit (all removals run in parallel).
const BUDGET: Duration = Duration::from_secs(2);

/// A LiveKit join webhook for participant `p` in `room`: true if it joined with a live
/// session's grant for exactly this room + identity (record presence as usual; the grant is
/// remembered as the one this participant holds). Otherwise the participant is removed in the
/// background and false is returned (record nothing). Covers a revoked device rejoining with its
/// old JWT, a token whose session went away between minting and joining, an expired session,
/// and a grant used for another room or identity.
pub async fn admit(s: &AppState, room: &str, p: &ParticipantInfo) -> anyhow::Result<bool> {
    let grant = p.attributes.get(GRANT_ATTRIBUTE).map(String::as_str);
    if session::admit_join(&s.db, grant, room, &p.identity).await? {
        return Ok(true);
    }
    tracing::warn!(
        room,
        identity = %p.identity,
        "LiveKit join without a live session's grant; removing"
    );
    let s = s.clone();
    let who = LiveKitParticipant {
        room: room.to_owned(),
        identity: p.identity.clone(),
    };
    tokio::spawn(async move { remove_participants(&s, &[who]).await });
    Ok(false)
}

/// Ask LiveKit to remove every one of `participants`. Never fails; problems are logged.
pub async fn remove_participants(s: &AppState, participants: &[LiveKitParticipant]) {
    if participants.is_empty() {
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
    let calls = futures::future::join_all(participants.iter().map(|g| {
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
            count = participants.len(),
            "LiveKit didn't answer in time; revoked sessions' participants may still be connected"
        );
    }
}
