//! When sessions are revoked, remove the LiveKit participants they admitted (voice rooms and
//! stream rooms). Best-effort: the REST/gateway revocation has already happened, so a LiveKit
//! that is down or slow is logged and the logout still succeeds.

use std::time::Duration;

use livekit_api::services::room::RoomClient;
use livekit_api::services::{ServerError, ServiceError};

use crate::AppState;
use crate::auth::session::LiveKitGrant;

/// Upper bound on how long a logout waits for LiveKit (all removals run in parallel).
const BUDGET: Duration = Duration::from_secs(2);

/// Ask LiveKit to remove every `grants` participant. Never fails; problems are logged.
pub async fn remove_participants(s: &AppState, grants: &[LiveKitGrant]) {
    if grants.is_empty() {
        return;
    }
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
