//! Ends a Go Live whose streamer is no longer connected to the stream room. The leave webhook
//! normally does that (`webhook.rs`), but a webhook can be missed (the server was down when the
//! connection went, or the delivery failed) and a client that dies can't end its own share, so
//! the stream would stay "live" for everyone.
//!
//! Every [`CHECK_EVERY`], LiveKit is asked who is in each stream room with a `Live` stream
//! (RoomService ListParticipants, all rooms at once). A streamer whose `#v` connection isn't
//! there starts a clock; seen again (or rejoined, or re-announced) stops it; still missing
//! `stream_media_timeout` after the first miss ends the stream, as the leave webhook and its
//! timeout would: the stream (and so the watches on it) and the streamer's own watches (watching
//! rides the same connection).
//!
//! Only LiveKit's answer counts. After a pulse-server restart the stream state starts empty and
//! clients re-announce: their `#v` is still in LiveKit (a deploy doesn't restart it), so nothing
//! ends. A LiveKit that can't be asked is "don't know": nothing ends on an error. A room LiveKit
//! doesn't have (closed once the last connection left) lists nobody: LiveKit v1.13.10 answers an
//! empty list (checked against the dev LiveKit, `tests/stream_liveness_it.rs`), and a `not_found`
//! error is taken the same way.

use std::collections::{BTreeMap, HashSet};
use std::time::{Duration, Instant};

use livekit_api::services::room::RoomClient;
use livekit_protocol::participant_info::State;
use pulse_protocol::ids::{ChannelId, UserId};
use pulse_protocol::rest::StreamEndReason;

use crate::AppState;
use crate::config::Config;
use crate::voice::revoke::not_found;

/// How often stream rooms are checked.
pub const CHECK_EVERY: Duration = Duration::from_secs(15);
/// Upper bound on each of a round's questions to LiveKit (rooms are asked in parallel).
pub const BUDGET: Duration = Duration::from_secs(5);

/// Who LiveKit says is connected to a room.
pub trait StreamRooms {
    /// The identities connected to `room` (none if LiveKit has no such room); an error when
    /// LiveKit couldn't say.
    fn connected(&self, room: &str)
    -> impl Future<Output = anyhow::Result<HashSet<String>>> + Send;
}

/// [`StreamRooms`] from LiveKit's RoomService, with the server's API URL and credentials.
pub struct LiveKitRooms {
    client: RoomClient,
    budget: Duration,
}

impl LiveKitRooms {
    pub fn new(cfg: &Config) -> Self {
        Self::with_budget(cfg, BUDGET)
    }

    /// Each question given up on (an error: "don't know") after `budget`.
    pub fn with_budget(cfg: &Config, budget: Duration) -> Self {
        Self {
            client: RoomClient::with_api_key(
                &cfg.livekit_api_url,
                &cfg.livekit_key,
                &cfg.livekit_secret,
            )
            .with_failover(false)
            .with_request_timeout(budget),
            budget,
        }
    }
}

impl StreamRooms for LiveKitRooms {
    async fn connected(&self, room: &str) -> anyhow::Result<HashSet<String>> {
        let participants =
            match tokio::time::timeout(self.budget, self.client.list_participants(room)).await {
                Err(_) => anyhow::bail!("LiveKit didn't answer in time"),
                Ok(Err(e)) if not_found(&e) => return Ok(HashSet::new()),
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
    /// Rooms LiveKit answered about.
    pub answered: Vec<ChannelId>,
    /// Rooms LiveKit couldn't be asked about, with why (their streams left as they were).
    pub unknown: Vec<(ChannelId, String)>,
}

fn room_of(channel: ChannelId) -> String {
    format!("{channel}~s")
}

fn video_identity(user: UserId) -> String {
    format!("{user}#v")
}

/// A line in the `share` log, beside the stream-room webhook's.
fn share_log(what: &str, channel: ChannelId, user: UserId) {
    tracing::info!(
        target: "share",
        "share: {}",
        serde_json::json!({ "what": what, "channel": channel.to_string(), "user": user.to_string() })
    );
}

/// One round: ask `rooms` about every room with a `Live` stream (in parallel) and act on the
/// answers at `now`. A stream ends once its `#v` has been missing for `grace` (see
/// [`super::state::StreamState::video_seen`]).
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
    let answers = futures::future::join_all(
        per_room
            .keys()
            .map(|&channel| async move { (channel, rooms.connected(&room_of(channel)).await) }),
    )
    .await;
    let mut out = Checked::default();
    for (channel, answer) in answers {
        let connected = match answer {
            Ok(c) => c,
            Err(e) => {
                out.unknown.push((channel, format!("{e:#}")));
                continue;
            }
        };
        out.answered.push(channel);
        for &user in &per_room[&channel] {
            let present = connected.contains(&video_identity(user));
            if !present {
                share_log("stream_video_missing", channel, user);
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
    // the leave webhook's timeout gives the same reason: the video connection didn't come back
    tracing::info!(user = %user, channel = %channel, reason = "video_gone", "stream ended");
    share_log("stream_ended_video_gone", channel, user);
    super::routes::publish_ended(s, channel, user, StreamEndReason::TimedOut).await;
    for (streamer, viewers) in s.streams.remove_viewer_everywhere(channel, user) {
        super::routes::publish_viewers(s, channel, streamer, viewers).await;
    }
}

/// Check every `every` for as long as the server runs, ending streams whose video connection
/// has been gone `grace`. Each room LiveKit can't be asked about is logged (with why) when it
/// starts failing, and again when it answers.
pub fn spawn(s: AppState, every: Duration, grace: Duration) {
    tokio::spawn(async move {
        let rooms = LiveKitRooms::new(&s.cfg);
        let mut tick = tokio::time::interval(every);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut failing: HashSet<ChannelId> = HashSet::new();
        loop {
            tick.tick().await;
            let c = check(&s, &rooms, Instant::now(), grace).await;
            for (channel, error) in &c.unknown {
                if !failing.contains(channel) {
                    tracing::warn!(channel = %channel, error = %error, "LiveKit can't be asked about a stream room; its streams are left as they are");
                }
            }
            for channel in &c.answered {
                if failing.contains(channel) {
                    tracing::info!(channel = %channel, "LiveKit answers about a stream room again");
                }
            }
            // rooms no longer checked (their streams ended) are forgotten
            failing = c.unknown.iter().map(|(ch, _)| *ch).collect();
        }
    });
}
