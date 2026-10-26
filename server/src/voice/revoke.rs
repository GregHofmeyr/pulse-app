//! When sessions are revoked, remove the LiveKit participants they admitted (voice rooms and
//! stream rooms). Best-effort: the REST/gateway revocation has already happened, so a LiveKit
//! that is down or slow is logged and the logout still succeeds.
//!
//! Every token we issue carries a random grant id as the participant attribute `pulse_grant`
//! (LiveKit keeps it on refreshed tokens; participants can't change it, as our tokens don't
//! grant `canUpdateOwnMetadata`). Revoking asks LiveKit who is in the rooms the revoked sessions
//! hold grants for and removes the participants carrying one of those grants, so LiveKit is the
//! source of truth even if a join webhook was lost.
//!
//! Removing a participant does not invalidate its JWT (LiveKit OSS, checked on v1.13.10: the
//! `revoke_token_ts` field is ignored, and LiveKit keeps refreshing a connected participant's
//! token), so a removed device could rejoin with the token it holds. [`admit`] closes that: the
//! join webhook removes again any participant whose grant isn't a live session's grant for that
//! room + identity.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use livekit_api::services::room::RoomClient;
use livekit_api::services::{ServerError, ServiceError};
use livekit_protocol::ParticipantInfo;

use crate::AppState;
use crate::auth::session::{self, GRANT_ATTRIBUTE, RevokedGrant};

/// Upper bound on how long a logout waits for LiveKit (all rooms are handled in parallel).
const BUDGET: Duration = Duration::from_secs(2);

fn client(s: &AppState) -> RoomClient {
    RoomClient::with_api_key(
        &s.cfg.livekit_api_url,
        &s.cfg.livekit_key,
        &s.cfg.livekit_secret,
    )
    .with_failover(false)
    .with_request_timeout(BUDGET)
}

fn grant_of(p: &ParticipantInfo) -> Option<&str> {
    p.attributes.get(GRANT_ATTRIBUTE).map(String::as_str)
}

fn not_found(e: &ServiceError) -> bool {
    matches!(e, ServiceError::Twirp(ServerError::Twirp(e)) if e.code == "not_found")
}

/// A LiveKit join webhook for participant `p` in `room`: true if it joined with a live
/// session's grant for exactly this room + identity (record presence as usual). Otherwise the
/// participant is removed in the background and false is returned (record nothing). Covers a
/// revoked device rejoining with its old JWT, a token whose session went away between minting
/// and joining, an expired session, and a grant used for another room or identity.
pub async fn admit(s: &AppState, room: &str, p: &ParticipantInfo) -> anyhow::Result<bool> {
    if session::admit_join(&s.db, grant_of(p), room, &p.identity, &p.sid).await? {
        return Ok(true);
    }
    tracing::warn!(
        room,
        identity = %p.identity,
        "LiveKit join without a live session's grant; removing"
    );
    let (s, room, p) = (s.clone(), room.to_owned(), p.clone());
    tokio::spawn(async move {
        if tokio::time::timeout(BUDGET, remove_if_still_there(&s, &room, &p))
            .await
            .is_err()
        {
            tracing::warn!(room, identity = %p.identity, "LiveKit didn't answer in time; refused participant may still be connected");
        }
    });
    Ok(false)
}

/// Remove the refused participant `p` only if LiveKit still has that very connection (same sid
/// and grant) under its identity: a valid device that rejoined with the identity since must not
/// be hit. This narrows the race to the time between the check and the removal (one API round
/// trip); LiveKit has no compare-and-remove.
async fn remove_if_still_there(s: &AppState, room: &str, p: &ParticipantInfo) {
    let client = client(s);
    match client.get_participant(room, &p.identity).await {
        Ok(now) if now.sid == p.sid && grant_of(&now) == grant_of(p) => {
            remove(&client, room, &p.identity).await;
        }
        Ok(_) => {
            tracing::info!(room, identity = %p.identity, "refused connection already replaced; left alone");
        }
        Err(e) if not_found(&e) => {}
        Err(e) => {
            tracing::warn!(error = %e, room, identity = %p.identity, "couldn't check refused LiveKit participant")
        }
    }
}

async fn remove(client: &RoomClient, room: &str, identity: &str) {
    match client.remove_participant(room, identity).await {
        Ok(()) => tracing::info!(room, identity, "removed LiveKit participant"),
        Err(e) if not_found(&e) => {
            tracing::debug!(room, identity, "LiveKit participant already gone")
        }
        Err(e) => {
            tracing::warn!(error = %e, room, identity, "couldn't remove LiveKit participant")
        }
    }
}

/// Remove every participant LiveKit reports carrying one of the revoked `grants` (per room,
/// in parallel, within [`BUDGET`]). Never fails; problems are logged.
pub async fn remove_revoked(s: &AppState, grants: &[RevokedGrant]) {
    let mut rooms: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for g in grants {
        rooms.entry(&g.room).or_default().insert(&g.id);
    }
    if rooms.is_empty() {
        return;
    }
    // Default RemoveParticipant options (`revoke_token_ts` 0): v1.13.10 ignores the field. A
    // LiveKit that starts honouring its documented default (revoke this identity's tokens until
    // now + 1 min) would lock the caller's other devices out of a shared identity for about a
    // minute: re-check on upgrades.
    let client = client(s);
    let per_room = rooms.iter().map(|(room, ids)| {
        let client = &client;
        async move {
            let participants = match client.list_participants(room).await {
                Ok(p) => p,
                Err(e) if not_found(&e) => return, // room already closed
                Err(e) => {
                    tracing::warn!(error = %e, room, "couldn't list LiveKit room to remove revoked sessions' participants");
                    return;
                }
            };
            // Removal is by identity: a valid device that took the identity over between the
            // list and the removal would be hit (one API round trip of exposure).
            for p in participants
                .iter()
                .filter(|p| grant_of(p).is_some_and(|g| ids.contains(g)))
            {
                remove(client, room, &p.identity).await;
            }
        }
    });
    if tokio::time::timeout(BUDGET, futures::future::join_all(per_room))
        .await
        .is_err()
    {
        tracing::warn!(
            rooms = rooms.len(),
            "LiveKit didn't answer in time; revoked sessions' participants may still be connected"
        );
    }
}

/// A LiveKit leave webhook: mark the grant finished (prunable) if this was its connection.
pub async fn left(s: &AppState, p: &ParticipantInfo) -> anyhow::Result<()> {
    if let Some(grant) = grant_of(p) {
        session::note_left(&s.db, grant, &p.sid).await?;
    }
    Ok(())
}
