//! Ends a Go Live whose streamer is no longer connected to the stream room. The leave webhook
//! normally does that (`webhook.rs`), but a webhook can be missed (the server was down when the
//! connection went, or the delivery failed) and a client that dies can't end its own share, so
//! the stream would stay "live" for everyone.
//!
//! Every [`CHECK_EVERY`], LiveKit is asked who is in each stream room with a `Live` stream
//! (RoomService ListParticipants). A streamer whose `#v` connection isn't there starts a clock;
//! seen again (or rejoined, or re-announced) stops it; still missing `stream_media_timeout`
//! after the first miss ends the stream (and the watches on it, and theirs on others: watching
//! rides the same connection), as the webhook path's timeout would.
//!
//! Only LiveKit's answer counts. After a pulse-server restart the stream state starts empty and
//! clients re-announce: their `#v` is still in LiveKit (a deploy doesn't restart it), so nothing
//! ends. A LiveKit that can't be asked is "don't know": nothing ends on an error.

use std::collections::{BTreeMap, HashSet};
use std::time::{Duration, Instant};

use livekit_api::services::room::RoomClient;
use livekit_api::services::{ServerError, ServiceError};
use livekit_protocol::participant_info::State;
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ChannelId, UserId};
use pulse_protocol::rest::StreamEndReason;

use crate::AppState;
use crate::config::Config;

/// How often stream rooms are checked.
pub const CHECK_EVERY: Duration = Duration::from_secs(15);
/// Upper bound on one room's question to LiveKit.
const BUDGET: Duration = Duration::from_secs(5);

/// Who LiveKit says is connected to a room.
pub trait StreamRooms {
    /// The identities connected to `room` (none if LiveKit has no such room); an error when
    /// LiveKit couldn't say.
    fn connected(&self, room: &str)
    -> impl Future<Output = anyhow::Result<HashSet<String>>> + Send;
}

/// [`StreamRooms`] from LiveKit's RoomService, with the server's API URL and credentials.
pub struct LiveKitRooms(RoomClient);

impl LiveKitRooms {
    pub fn new(cfg: &Config) -> Self {
        Self(
            RoomClient::with_api_key(&cfg.livekit_api_url, &cfg.livekit_key, &cfg.livekit_secret)
                .with_failover(false)
                .with_request_timeout(BUDGET),
        )
    }
}

impl StreamRooms for LiveKitRooms {
    async fn connected(&self, room: &str) -> anyhow::Result<HashSet<String>> {
        let participants = match tokio::time::timeout(BUDGET, self.0.list_participants(room)).await
        {
            Err(_) => anyhow::bail!("LiveKit didn't answer in time"),
            // the room closed: nobody is in it
            Ok(Err(ServiceError::Twirp(ServerError::Twirp(e)))) if e.code == "not_found" => {
                return Ok(HashSet::new());
            }
            Ok(r) => r?,
        };
        Ok(participants
            .into_iter()
            .filter(|p| p.state != State::Disconnected as i32)
            .map(|p| p.identity)
            .collect())
    }
}

/// What one round of checks did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Checked {
    /// Streams ended because their streamer's video connection was gone for the grace.
    pub ended: usize,
    /// Rooms LiveKit couldn't be asked about (their streams left as they were).
    pub unknown: usize,
}

fn room_of(channel: ChannelId) -> String {
    format!("{channel}~s")
}

fn video_identity(user: UserId) -> String {
    format!("{user}#v")
}

/// One round: ask `rooms` about every room with a `Live` stream and act on the answers at `now`.
/// A stream ends once its `#v` has been missing for `grace` (see [`super::state::StreamState::video_seen`]).
pub async fn check(
    s: &AppState,
    rooms: &impl StreamRooms,
    now: Instant,
    grace: Duration,
) -> Checked {
    let mut per_room: BTreeMap<ChannelId, Vec<UserId>> = BTreeMap::new();
    for (channel, user) in s.streams.live_streams() {
        per_room.entry(channel).or_default().push(user);
    }
    let mut out = Checked::default();
    for (channel, streamers) in per_room {
        let connected = match rooms.connected(&room_of(channel)).await {
            Ok(c) => c,
            Err(e) => {
                tracing::debug!(error = %e, channel = %channel, "couldn't ask LiveKit about a stream room; left as it is");
                out.unknown += 1;
                continue;
            }
        };
        for user in streamers {
            let present = connected.contains(&video_identity(user));
            if !present {
                tracing::info!(user = %user, channel = %channel, "LiveKit has no video connection for a live stream");
            }
            if s.streams
                .video_seen(channel, user, present, now, grace)
                .is_some()
            {
                out.ended += 1;
                ended(s, channel, user).await;
            }
        }
    }
    out
}

/// Tell everyone the stream ended, and drop the streamer's own watches (same connection).
async fn ended(s: &AppState, channel: ChannelId, user: UserId) {
    tracing::info!(user = %user, channel = %channel, reason = "video_gone", "stream ended");
    s.hub
        .publish(
            &s.db,
            Event::StreamEnded {
                channel_id: channel,
                user_id: user,
                reason: StreamEndReason::TimedOut,
            },
        )
        .await;
    for (streamer, viewers) in s.streams.remove_viewer_everywhere(channel, user) {
        super::routes::publish_viewers(s, channel, streamer, viewers).await;
    }
}

/// Check every [`CHECK_EVERY`] for as long as the server runs, with `stream_media_timeout` as
/// the grace. A LiveKit that stops answering is logged once (and again when it's back).
pub fn spawn(s: AppState) {
    tokio::spawn(async move {
        let rooms = LiveKitRooms::new(&s.cfg);
        let mut tick = tokio::time::interval(CHECK_EVERY);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut failing = false;
        loop {
            tick.tick().await;
            let c = check(&s, &rooms, Instant::now(), s.cfg.stream_media_timeout).await;
            match (c.unknown > 0, failing) {
                (true, false) => tracing::warn!(
                    rooms = c.unknown,
                    "LiveKit can't be asked about stream rooms; their streams are left as they are"
                ),
                (false, true) => tracing::info!("LiveKit answers about stream rooms again"),
                _ => {}
            }
            failing = c.unknown > 0;
        }
    });
}
