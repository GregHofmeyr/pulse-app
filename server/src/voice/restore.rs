//! Rebuild voice presence after a restart from LiveKit's own view of its rooms. A Pulse deploy
//! restarts the server but not LiveKit, so calls carry on and no `participant_joined` is re-sent.

use std::collections::HashSet;
use std::time::Duration;

use anyhow::Context;
use livekit_api::services::room::RoomClient;
use livekit_protocol::participant_info::State;
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ChannelId, UserId};
use sqlx::SqlitePool;

use super::VoiceState;
use crate::AppState;
use crate::access::load_channel;

/// Startup waits this long for LiveKit before serving without it.
const BOOT_BUDGET: Duration = Duration::from_secs(2);
const RETRY_EVERY: Duration = Duration::from_secs(5);
const RETRIES: u32 = 12;

/// Someone LiveKit reports as connected to one of our rooms.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveParticipant {
    pub channel: ChannelId,
    pub user: UserId,
    pub sid: String,
    /// RFC 3339, from LiveKit's join time.
    pub joined_at: String,
}

/// Everyone connected, per LiveKit. Rooms are channel ids and identities user ids (our tokens,
/// `voice/routes.rs`); anything else isn't ours and is skipped. `budget` bounds the whole call.
pub async fn fetch(
    url: &str,
    key: &str,
    secret: &str,
    budget: Duration,
) -> anyhow::Result<Vec<LiveParticipant>> {
    let client = RoomClient::with_api_key(url, key, secret)
        .with_failover(false)
        .with_request_timeout(budget);
    tokio::time::timeout(budget, async {
        let mut out = Vec::new();
        for room in client.list_rooms(vec![]).await? {
            let Ok(channel) = room.name.parse::<ChannelId>() else {
                continue;
            };
            for p in client.list_participants(&room.name).await? {
                let Ok(user) = p.identity.parse::<UserId>() else {
                    continue;
                };
                if p.state == State::Disconnected as i32 {
                    continue;
                }
                // 0 (or negative) means LiveKit didn't set it: treat as now, not 1970.
                let joined_at = (p.joined_at > 0)
                    .then(|| chrono::DateTime::from_timestamp(p.joined_at, 0))
                    .flatten()
                    .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
                    .unwrap_or_else(crate::db::now);
                out.push(LiveParticipant {
                    channel,
                    user,
                    sid: p.sid,
                    joined_at,
                });
            }
        }
        anyhow::Ok(out)
    })
    .await
    .context("LiveKit didn't answer in time")?
}

/// Make `voice` and `voice_sessions` agree with `live`. Session rows opened before `started_at`
/// (this boot) stay open for people still in a call and close at zero length for the rest (we
/// can't know when they left); server-channel calls with no open row get one from LiveKit's join
/// time. Returns who was newly added to `voice`.
pub async fn apply(
    db: &SqlitePool,
    voice: &VoiceState,
    live: &[LiveParticipant],
    started_at: &str,
) -> anyhow::Result<Vec<LiveParticipant>> {
    // Only rooms that are still channels and identities that are still users (a row for anyone
    // else would fail the whole transaction on the FK); only server voice feeds stats (spec §8).
    let mut known = Vec::new();
    for p in live {
        let Some(ch) = load_channel(db, p.channel).await? else {
            continue;
        };
        let user: Option<i64> = sqlx::query_scalar("SELECT 1 FROM users WHERE id = ?")
            .bind(p.user.to_string())
            .fetch_optional(db)
            .await?;
        if user.is_some() {
            known.push((p, ch.server_id.is_some()));
        }
    }
    let here: HashSet<(String, String)> = known
        .iter()
        .map(|(p, _)| (p.user.to_string(), p.channel.to_string()))
        .collect();

    let mut tx = crate::db::begin_write(db).await?;
    let open: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT id, user_id, channel_id FROM voice_sessions WHERE left_at IS NULL AND joined_at < ?",
    )
    .bind(started_at)
    .fetch_all(&mut *tx)
    .await?;
    for (id, user, channel) in open {
        if !here.contains(&(user, channel)) {
            sqlx::query("UPDATE voice_sessions SET left_at = joined_at WHERE id = ?")
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
    }
    for (p, record) in &known {
        if !record {
            continue;
        }
        let has_open: Option<i64> = sqlx::query_scalar(
            "SELECT 1 FROM voice_sessions WHERE user_id = ? AND channel_id = ? AND left_at IS NULL",
        )
        .bind(p.user.to_string())
        .bind(p.channel.to_string())
        .fetch_optional(&mut *tx)
        .await?;
        if has_open.is_none() {
            sqlx::query(
                "INSERT INTO voice_sessions (id, user_id, channel_id, joined_at) VALUES (?, ?, ?, ?)",
            )
            .bind(pulse_protocol::ids::next_ulid().to_string())
            .bind(p.user.to_string())
            .bind(p.channel.to_string())
            .bind(&p.joined_at)
            .execute(&mut *tx)
            .await?;
        }
    }
    tx.commit().await?;

    Ok(known
        .into_iter()
        .filter(|(p, _)| voice.restore(p.channel, p.user, &p.sid))
        .map(|(p, _)| p.clone())
        .collect())
}

/// Ask LiveKit, apply, and announce whoever was added (clients connected by then catch up).
pub async fn sync(s: &AppState, started_at: &str, budget: Duration) -> anyhow::Result<usize> {
    let live = fetch(
        &s.cfg.livekit_api_url,
        &s.cfg.livekit_key,
        &s.cfg.livekit_secret,
        budget,
    )
    .await?;
    let added = apply(&s.db, &s.voice, &live, started_at).await?;
    for p in &added {
        s.hub
            .publish(
                &s.db,
                Event::VoiceJoined {
                    channel_id: p.channel,
                    user_id: p.user,
                    flags: s.voice.declared(p.user),
                },
            )
            .await;
    }
    Ok(added.len())
}

/// Boot: one quick try before serving (so the first Ready after a deploy is right). If LiveKit
/// can't be reached, close old sessions as before, start empty, and keep trying in the background.
pub async fn on_startup(s: &AppState, started_at: &str) -> anyhow::Result<()> {
    match sync(s, started_at, BOOT_BUDGET).await {
        Ok(n) => {
            tracing::info!(restored = n, "voice presence restored from LiveKit");
            Ok(())
        }
        Err(e) => {
            tracing::warn!(error = %e, "LiveKit unreachable at startup; voice presence starts empty, retrying in the background");
            super::reconcile_on_startup(&s.db).await?;
            tokio::spawn(retry_loop(
                s.clone(),
                started_at.to_owned(),
                RETRY_EVERY,
                RETRIES,
            ));
            Ok(())
        }
    }
}

/// Background retries after a failed boot sync: every `every` (also each attempt's budget), up to
/// `tries` times. Stops at the first success, which announces whoever it found.
pub async fn retry_loop(s: AppState, started_at: String, every: Duration, tries: u32) {
    for attempt in 1..=tries {
        tokio::time::sleep(every).await;
        match sync(&s, &started_at, every).await {
            Ok(n) => {
                tracing::info!(
                    restored = n,
                    attempt,
                    "voice presence restored from LiveKit"
                );
                return;
            }
            Err(e) => tracing::debug!(error = %e, attempt, "LiveKit still unreachable"),
        }
    }
    tracing::warn!("gave up restoring voice presence; it fills in as people rejoin");
}
