//! Who is streaming in which voice channel, who is watching, and each stream's latest preview.
//! In memory only: after a restart live clients re-announce themselves. One stream per
//! (channel, user); a re-start while live is an update.
//!
//! Ghost watch sessions (spec §5) live here too, and only here: an opaque id per (login
//! session, channel) mapped to the user, never stored and never sent to another client. Clients
//! see a count.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bytes::Bytes;
use pulse_protocol::ids::{ChannelId, UserId};
use pulse_protocol::rest::{StreamInfo, StreamRole, StreamSettings, StreamStatus};
use sha2::{Digest, Sha256};

use super::ghost_ids::{new_ghost_id, new_grant, suffix};

/// A ghost session nobody connected with is dropped after this (a stream ticket's lifetime).
pub const GHOST_UNJOINED_FOR: Duration = Duration::from_secs(10 * 60);

/// One ghost watch session.
struct Ghost {
    user: UserId,
    channel: ChannelId,
    /// The login session (token hash) it belongs to.
    session: String,
    /// `pulse_grant` for `#v` and `#a`: random, checked on the join webhook.
    grants: [String; 2],
    /// The streamers watched.
    watching: BTreeSet<UserId>,
    /// The current `#v` connection (LiveKit sid), once joined.
    video_sid: Option<String>,
    issued: Instant,
}

fn role_index(role: StreamRole) -> usize {
    match role {
        StreamRole::V => 0,
        StreamRole::A => 1,
    }
}

/// What [`StreamState::purge_ghosts`] matches a ghost on.
pub struct GhostView<'a> {
    pub channel: ChannelId,
    pub user: UserId,
    pub session: &'a str,
}

/// Ghosts taken out of the state: their ids per channel (to remove from LiveKit), and each
/// stream whose ghost count changed, with its new count.
#[derive(Debug, Default, PartialEq)]
pub struct Purged {
    pub removed: Vec<(ChannelId, String)>,
    pub counts: Vec<(ChannelId, UserId, u32)>,
}

/// What [`StreamState::ghost_watch`] did.
#[derive(Debug, PartialEq)]
pub enum GhostWatch {
    /// Now watching: the stream's new ghost count, and the ghost's id (for the server log).
    Added {
        count: u32,
        id: String,
    },
    Already,
    NoStream,
    /// That stream, or another one live in the channel, is voice-only.
    NotAllowed,
}

struct Stream {
    info: StreamInfo,
    preview: Option<(Bytes, String)>,
    epoch: u64,
    /// Since when LiveKit has said the streamer's `#v` isn't in the stream room (the liveness
    /// check, `liveness.rs`); cleared whenever it's seen, rejoins or the stream is re-announced.
    video_missing_since: Option<Instant>,
}

#[derive(Default)]
struct Inner {
    streams: HashMap<(ChannelId, UserId), Stream>,
    /// Each person's current `#v` connection (LiveKit participant sid) in a stream room, streaming
    /// or watching. A rejoin replaces it, and the replaced one's late leave is then ignored.
    video_sids: HashMap<(ChannelId, UserId), String>,
    /// Source of epochs: never reused, so a new stream never matches an old timer's epoch.
    last_epoch: u64,
    /// Ghost watch sessions by id.
    ghosts: HashMap<String, Ghost>,
}

impl Inner {
    /// Ghosts may be in `channel`'s stream room: something is live there and all of it allows
    /// them (a ticket can subscribe to every track in the room, so this is per room).
    fn ghosts_allowed(&self, channel: ChannelId) -> bool {
        let mut any = false;
        for ((c, _), s) in &self.streams {
            if *c == channel {
                if !s.info.settings.allow_ghosts {
                    return false;
                }
                any = true;
            }
        }
        any
    }

    /// Set `streamer`'s ghost count from the sessions; the new count, if the stream exists.
    /// Only ghosts whose `#v` is connected count (LiveKit's word wins for who is there): a
    /// ticket and a watch with no connection behind them are nobody watching.
    fn recount(&mut self, channel: ChannelId, streamer: UserId) -> Option<u32> {
        let n = self
            .ghosts
            .values()
            .filter(|g| {
                g.channel == channel && g.video_sid.is_some() && g.watching.contains(&streamer)
            })
            .count();
        let n = u32::try_from(n).unwrap_or(u32::MAX);
        let s = self.streams.get_mut(&(channel, streamer))?;
        s.info.ghosts = n;
        Some(n)
    }

    /// Login `session`'s ghost in `channel`, if any.
    fn ghost_of(&self, channel: ChannelId, session: &str) -> Option<String> {
        self.ghosts
            .iter()
            .find(|(_, g)| g.channel == channel && g.session == session)
            .map(|(id, _)| id.clone())
    }

    /// Login `session`'s ghost in `channel`, made now if it has none. Never-joined ghosts past
    /// their ticket's lifetime are dropped first.
    fn ghost_for(
        &mut self,
        channel: ChannelId,
        user: UserId,
        session: &str,
        now: Instant,
    ) -> String {
        self.ghosts.retain(|_, g| {
            g.video_sid.is_some() || now.saturating_duration_since(g.issued) < GHOST_UNJOINED_FOR
        });
        if let Some(id) = self.ghost_of(channel, session) {
            return id;
        }
        let id = new_ghost_id();
        self.ghosts.insert(
            id.clone(),
            Ghost {
                user,
                channel,
                session: session.to_owned(),
                grants: [new_grant(), new_grant()],
                watching: BTreeSet::new(),
                video_sid: None,
                issued: now,
            },
        );
        id
    }

    /// Take out the ghosts `pred` matches.
    fn purge(&mut self, pred: impl Fn(&GhostView) -> bool) -> Purged {
        let ids: Vec<String> = self
            .ghosts
            .iter()
            .filter(|(_, g)| {
                pred(&GhostView {
                    channel: g.channel,
                    user: g.user,
                    session: &g.session,
                })
            })
            .map(|(id, _)| id.clone())
            .collect();
        let mut out = Purged::default();
        let mut touched = BTreeSet::new();
        for id in ids {
            let g = self.ghosts.remove(&id).unwrap();
            touched.extend(g.watching.iter().map(|w| (g.channel, *w)));
            out.removed.push((g.channel, id));
        }
        out.removed.sort();
        for (c, w) in touched {
            if let Some(n) = self.recount(c, w) {
                out.counts.push((c, w, n));
            }
        }
        out
    }

    /// A stream ended: its ghost watches go with it.
    fn forget_streamer(&mut self, channel: ChannelId, streamer: UserId) {
        for g in self.ghosts.values_mut() {
            if g.channel == channel {
                g.watching.remove(&streamer);
            }
        }
    }

    fn next_epoch(&mut self) -> u64 {
        self.last_epoch += 1;
        self.last_epoch
    }

    fn remove_viewer_everywhere(
        &mut self,
        channel: ChannelId,
        viewer: UserId,
    ) -> Vec<(UserId, Vec<UserId>)> {
        let mut out = Vec::new();
        for ((c, streamer), s) in self.streams.iter_mut() {
            if *c != channel {
                continue;
            }
            if let Some(pos) = s.info.viewers.iter().position(|v| *v == viewer) {
                s.info.viewers.remove(pos);
                out.push((*streamer, s.info.viewers.clone()));
            }
        }
        out
    }
}

/// A stream as the liveness check reads it before asking LiveKit.
#[derive(Clone, Debug, PartialEq)]
pub struct ToCheck {
    pub channel: ChannelId,
    pub user: UserId,
    pub status: StreamStatus,
    pub epoch: u64,
}

/// What [`StreamState::video_seen`] made of LiveKit's answer.
#[derive(Debug, PartialEq)]
pub enum Seen {
    /// No such stream, or not `Live` (a `Reconnecting` one has its own timer).
    Ignored,
    Present,
    /// Missing, and the clock started.
    FirstMiss,
    /// Missing, the clock running.
    StillMissing,
    /// Missing for the grace: the stream ended, and the streamer stopped watching these
    /// streams (their new viewer lists).
    Ended {
        info: StreamInfo,
        viewers: Vec<(UserId, Vec<UserId>)>,
    },
}

impl Seen {
    pub fn ended(&self) -> bool {
        matches!(self, Seen::Ended { .. })
    }
}

/// What `start` did.
#[derive(Debug, PartialEq)]
pub enum Started {
    New(StreamInfo),
    Updated(StreamInfo),
}

#[derive(Clone, Default)]
pub struct StreamState {
    inner: Arc<Mutex<Inner>>,
}

impl StreamState {
    /// Go live, or (already live) replace the settings and mark `Live`, keeping viewers and
    /// `started_at`.
    pub fn start(
        &self,
        channel: ChannelId,
        user: UserId,
        settings: StreamSettings,
        now: String,
    ) -> Started {
        let mut inner = self.inner.lock().unwrap();
        if let Some(s) = inner.streams.get_mut(&(channel, user)) {
            s.info.settings = settings;
            s.info.status = StreamStatus::Live;
            s.video_missing_since = None;
            return Started::Updated(s.info.clone());
        }
        let info = StreamInfo {
            channel_id: channel,
            user_id: user,
            settings,
            status: StreamStatus::Live,
            viewers: Vec::new(),
            ghosts: 0,
            started_at: now,
        };
        let epoch = inner.next_epoch();
        inner.streams.insert(
            (channel, user),
            Stream {
                info: info.clone(),
                preview: None,
                epoch,
                video_missing_since: None,
            },
        );
        Started::New(info)
    }

    /// End `user`'s stream in `channel`, dropping its preview.
    pub fn end(&self, channel: ChannelId, user: UserId) -> Option<StreamInfo> {
        let mut inner = self.inner.lock().unwrap();
        inner.forget_streamer(channel, user);
        inner.streams.remove(&(channel, user)).map(|s| s.info)
    }

    /// End every stream `user` has, in any channel.
    pub fn end_any_for(&self, user: UserId) -> Vec<StreamInfo> {
        let mut inner = self.inner.lock().unwrap();
        let keys: Vec<_> = inner
            .streams
            .keys()
            .filter(|(_, u)| *u == user)
            .copied()
            .collect();
        for &(c, u) in &keys {
            inner.forget_streamer(c, u);
        }
        keys.into_iter()
            .filter_map(|k| inner.streams.remove(&k).map(|s| s.info))
            .collect()
    }

    /// The new info if the status changed; `None` if unchanged or there is no such stream.
    pub fn set_status(
        &self,
        channel: ChannelId,
        user: UserId,
        status: StreamStatus,
    ) -> Option<StreamInfo> {
        let mut inner = self.inner.lock().unwrap();
        let s = inner.streams.get_mut(&(channel, user))?;
        if s.info.status == status {
            return None;
        }
        s.info.status = status;
        Some(s.info.clone())
    }

    /// The new viewer list; `None` if the stream is not live or `viewer` was already watching.
    pub fn add_viewer(
        &self,
        channel: ChannelId,
        streamer: UserId,
        viewer: UserId,
    ) -> Option<Vec<UserId>> {
        let mut inner = self.inner.lock().unwrap();
        let s = inner.streams.get_mut(&(channel, streamer))?;
        if s.info.status != StreamStatus::Live || s.info.viewers.contains(&viewer) {
            return None;
        }
        s.info.viewers.push(viewer);
        Some(s.info.viewers.clone())
    }

    /// The new viewer list; `None` if there is no such stream or `viewer` wasn't watching.
    pub fn remove_viewer(
        &self,
        channel: ChannelId,
        streamer: UserId,
        viewer: UserId,
    ) -> Option<Vec<UserId>> {
        let mut inner = self.inner.lock().unwrap();
        let s = inner.streams.get_mut(&(channel, streamer))?;
        let pos = s.info.viewers.iter().position(|v| *v == viewer)?;
        s.info.viewers.remove(pos);
        Some(s.info.viewers.clone())
    }

    /// Drop `viewer` from every stream in `channel`; one `(streamer, new viewers)` per stream
    /// they were watching.
    pub fn remove_viewer_everywhere(
        &self,
        channel: ChannelId,
        viewer: UserId,
    ) -> Vec<(UserId, Vec<UserId>)> {
        self.inner
            .lock()
            .unwrap()
            .remove_viewer_everywhere(channel, viewer)
    }

    pub fn get(&self, channel: ChannelId, user: UserId) -> Option<StreamInfo> {
        let inner = self.inner.lock().unwrap();
        inner.streams.get(&(channel, user)).map(|s| s.info.clone())
    }

    pub fn all(&self) -> Vec<StreamInfo> {
        let inner = self.inner.lock().unwrap();
        inner.streams.values().map(|s| s.info.clone()).collect()
    }

    /// Keep the latest preview; false if the stream is not live (nothing stored).
    pub fn set_preview(&self, channel: ChannelId, user: UserId, bytes: Bytes) -> bool {
        let mut inner = self.inner.lock().unwrap();
        let Some(s) = inner.streams.get_mut(&(channel, user)) else {
            return false;
        };
        let etag = hex::encode(&Sha256::digest(&bytes)[..8]);
        s.preview = Some((bytes, etag));
        true
    }

    /// The latest preview and its etag (first 16 hex chars of the bytes' SHA-256).
    pub fn preview(&self, channel: ChannelId, user: UserId) -> Option<(Bytes, String)> {
        let inner = self.inner.lock().unwrap();
        inner.streams.get(&(channel, user))?.preview.clone()
    }

    /// `user`'s `#v` connection `sid` joined `channel`'s stream room.
    pub fn video_joined(&self, channel: ChannelId, user: UserId, sid: &str) {
        let mut inner = self.inner.lock().unwrap();
        inner.video_sids.insert((channel, user), sid.to_owned());
        if let Some(s) = inner.streams.get_mut(&(channel, user)) {
            s.video_missing_since = None;
        }
    }

    /// Every stream, for the liveness check: what it is and its status and epoch as of now.
    pub fn streams_to_check(&self) -> Vec<ToCheck> {
        let inner = self.inner.lock().unwrap();
        let mut out: Vec<_> = inner
            .streams
            .iter()
            .map(|(&(channel, user), s)| ToCheck {
                channel,
                user,
                status: s.info.status.clone(),
                epoch: s.epoch,
            })
            .collect();
        out.sort_by_key(|c| (c.channel, c.user));
        out
    }

    /// LiveKit's answer, at `now`, to whether a `Live` stream's `#v` is in the stream room.
    /// There: the clock stops. Missing: the first miss starts it, and a miss `grace` or more
    /// after that ends the stream, and with it the streamer's watches in that channel (same
    /// connection), in one step. A rejoin or re-announce meanwhile restarts the clock, so an
    /// answer from before them can't end the stream. Not live (or no stream): ignored.
    pub fn video_seen(
        &self,
        channel: ChannelId,
        user: UserId,
        present: bool,
        now: Instant,
        grace: Duration,
    ) -> Seen {
        let mut inner = self.inner.lock().unwrap();
        let Some(s) = inner.streams.get_mut(&(channel, user)) else {
            return Seen::Ignored;
        };
        if s.info.status != StreamStatus::Live {
            return Seen::Ignored;
        }
        if present {
            s.video_missing_since = None;
            return Seen::Present;
        }
        match s.video_missing_since {
            Some(since) if now.saturating_duration_since(since) >= grace => {
                // LiveKit says that connection is gone: its sid can't make a later one's leave stale
                inner.video_sids.remove(&(channel, user));
                inner.forget_streamer(channel, user);
                let info = inner.streams.remove(&(channel, user)).unwrap().info;
                let viewers = inner.remove_viewer_everywhere(channel, user);
                Seen::Ended { info, viewers }
            }
            Some(_) => Seen::StillMissing,
            None => {
                s.video_missing_since = Some(now);
                Seen::FirstMiss
            }
        }
    }

    /// LiveKit lists the `#v` of a `Reconnecting` stream again (its join webhook was missed, or
    /// is on its way): `Live` again in a new episode, as the join webhook makes it, provided the
    /// stream is still in the episode `epoch` read before asking (a leave or join since then makes
    /// the answer stale). The new info if it changed.
    pub fn video_back(&self, channel: ChannelId, user: UserId, epoch: u64) -> Option<StreamInfo> {
        let mut inner = self.inner.lock().unwrap();
        let next = inner.next_epoch();
        let s = inner.streams.get_mut(&(channel, user))?;
        if s.info.status != StreamStatus::Reconnecting || s.epoch != epoch {
            return None;
        }
        s.info.status = StreamStatus::Live;
        s.epoch = next;
        s.video_missing_since = None;
        Some(s.info.clone())
    }

    /// `user`'s `#v` connection `sid` left; false if a rejoin already replaced it (a late leave
    /// to ignore). Unknown connections (e.g. from before a restart) count as current.
    pub fn video_left(&self, channel: ChannelId, user: UserId, sid: &str) -> bool {
        let mut inner = self.inner.lock().unwrap();
        match inner.video_sids.get(&(channel, user)) {
            Some(cur) if !sid.is_empty() && !cur.is_empty() && cur != sid => false,
            _ => {
                inner.video_sids.remove(&(channel, user));
                true
            }
        }
    }

    /// Start a new reconnect episode for this stream and return its epoch (0 if there is no
    /// such stream; real epochs start at 1). Epochs are unique across all streams.
    pub fn bump_epoch(&self, channel: ChannelId, user: UserId) -> u64 {
        let mut inner = self.inner.lock().unwrap();
        if !inner.streams.contains_key(&(channel, user)) {
            return 0;
        }
        let epoch = inner.next_epoch();
        inner.streams.get_mut(&(channel, user)).unwrap().epoch = epoch;
        epoch
    }

    /// The stream's current epoch; `None` once it has ended.
    pub fn epoch(&self, channel: ChannelId, user: UserId) -> Option<u64> {
        let inner = self.inner.lock().unwrap();
        inner.streams.get(&(channel, user)).map(|s| s.epoch)
    }

    /// Whether ghosts may be in `channel`'s stream room now (see `Inner::ghosts_allowed`).
    pub fn ghosts_allowed(&self, channel: ChannelId) -> bool {
        self.inner.lock().unwrap().ghosts_allowed(channel)
    }

    /// A ghost ticket's identity (`g_<id>#v` / `#a`) and grant for `user`'s login `session` in
    /// `channel`: that session's ghost, made now if it has none. `None` while ghosts aren't
    /// allowed there.
    pub fn ghost_ticket(
        &self,
        channel: ChannelId,
        user: UserId,
        session: &str,
        role: StreamRole,
        now: Instant,
    ) -> Option<(String, String)> {
        let mut inner = self.inner.lock().unwrap();
        if !inner.ghosts_allowed(channel) {
            return None;
        }
        let id = inner.ghost_for(channel, user, session, now);
        let grant = inner.ghosts[&id].grants[role_index(role)].clone();
        Some((format!("{id}#{}", suffix(role)), grant))
    }

    /// `user` (login `session`) watches `streamer` as a ghost.
    pub fn ghost_watch(
        &self,
        channel: ChannelId,
        user: UserId,
        session: &str,
        streamer: UserId,
        now: Instant,
    ) -> GhostWatch {
        let mut inner = self.inner.lock().unwrap();
        let live = inner
            .streams
            .get(&(channel, streamer))
            .is_some_and(|s| s.info.status == StreamStatus::Live);
        if !live {
            return GhostWatch::NoStream;
        }
        if !inner.ghosts_allowed(channel) {
            return GhostWatch::NotAllowed;
        }
        let id = inner.ghost_for(channel, user, session, now);
        if !inner.ghosts.get_mut(&id).unwrap().watching.insert(streamer) {
            return GhostWatch::Already;
        }
        let count = inner.recount(channel, streamer).unwrap_or(0);
        GhostWatch::Added { count, id }
    }

    /// Login `session`'s ghost stops watching `streamer`: the new count, if it was watching.
    pub fn ghost_unwatch(
        &self,
        channel: ChannelId,
        session: &str,
        streamer: UserId,
    ) -> Option<u32> {
        let mut inner = self.inner.lock().unwrap();
        let id = inner.ghost_of(channel, session)?;
        if !inner
            .ghosts
            .get_mut(&id)
            .unwrap()
            .watching
            .remove(&streamer)
        {
            return None;
        }
        inner.recount(channel, streamer)
    }

    /// The join webhook for ghost `id`'s `role` connection in `channel`'s stream room: it may
    /// come in if `grant` is that ghost's grant for the role and ghosts are allowed there now.
    /// Returns the user and login session behind it, for the caller's own checks; nothing is
    /// recorded until [`Self::ghost_connected`].
    pub fn ghost_admit(
        &self,
        channel: ChannelId,
        id: &str,
        role: StreamRole,
        grant: Option<&str>,
    ) -> Option<(UserId, String)> {
        let inner = self.inner.lock().unwrap();
        if !inner.ghosts_allowed(channel) {
            return None;
        }
        let g = inner.ghosts.get(id)?;
        if g.channel != channel || grant != Some(g.grants[role_index(role)].as_str()) {
            return None;
        }
        Some((g.user, g.session.clone()))
    }

    /// Ghost `id`'s admitted `#v` connection `sid` is up: from now on it counts. Each stream it
    /// watches whose count changed, with the new count.
    pub fn ghost_connected(&self, channel: ChannelId, id: &str, sid: &str) -> Vec<(UserId, u32)> {
        let mut inner = self.inner.lock().unwrap();
        let Some(g) = inner.ghosts.get_mut(id).filter(|g| g.channel == channel) else {
            return Vec::new();
        };
        let first = g.video_sid.is_none();
        g.video_sid = Some(sid.to_owned());
        if !first {
            return Vec::new(); // a reconnect: already counted
        }
        let watching: Vec<UserId> = g.watching.iter().copied().collect();
        watching
            .into_iter()
            .filter_map(|w| inner.recount(channel, w).map(|n| (w, n)))
            .collect()
    }

    /// Ghost `id`'s `role` connection `sid` left. Its current `#v` leaving ends the ghost
    /// session: each stream it watched, with the new count.
    pub fn ghost_left(
        &self,
        channel: ChannelId,
        id: &str,
        role: StreamRole,
        sid: &str,
    ) -> Vec<(UserId, u32)> {
        let mut inner = self.inner.lock().unwrap();
        let current = role == StreamRole::V
            && inner.ghosts.get(id).is_some_and(|g| {
                g.channel == channel && g.video_sid.as_deref().is_none_or(|cur| cur == sid)
            });
        if !current {
            return Vec::new();
        }
        let g = inner.ghosts.remove(id).unwrap();
        g.watching
            .iter()
            .filter_map(|w| inner.recount(channel, *w).map(|n| (*w, n)))
            .collect()
    }

    /// Take out every ghost `pred` matches (allow-ghosts off, a ghost joining voice, a revoked
    /// login, someone leaving the server).
    pub fn purge_ghosts(&self, pred: impl Fn(&GhostView) -> bool) -> Purged {
        self.inner.lock().unwrap().purge(pred)
    }

    /// Every ghost's login session, once each (to check which are still live).
    pub fn ghost_sessions(&self) -> Vec<String> {
        let inner = self.inner.lock().unwrap();
        let all: BTreeSet<&String> = inner.ghosts.values().map(|g| &g.session).collect();
        all.into_iter().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(n: i64) -> ChannelId {
        format!("{n:026}").parse().unwrap()
    }
    fn u(n: i64) -> UserId {
        format!("{n:026}").parse().unwrap()
    }
    fn settings(fps: u8) -> StreamSettings {
        StreamSettings {
            height: 720,
            fps,
            audio: true,
            allow_ghosts: true,
        }
    }

    #[test]
    fn start_twice_is_an_update_that_keeps_viewers() {
        let s = StreamState::default();
        let Started::New(first) = s.start(ch(1), u(1), settings(30), "t0".into()) else {
            panic!("first start must be New");
        };
        assert_eq!(first.status, StreamStatus::Live);
        s.add_viewer(ch(1), u(1), u(2)).unwrap();
        s.set_status(ch(1), u(1), StreamStatus::Reconnecting)
            .unwrap();
        let Started::Updated(again) = s.start(ch(1), u(1), settings(60), "t1".into()) else {
            panic!("second start must be Updated");
        };
        assert_eq!(again.settings.fps, 60);
        assert_eq!(again.started_at, "t0");
        assert_eq!(again.viewers, vec![u(2)]);
        assert_eq!(again.status, StreamStatus::Live);
        assert_eq!(s.all().len(), 1);
    }

    #[test]
    fn end_drops_preview() {
        let s = StreamState::default();
        s.start(ch(1), u(1), settings(30), "t".into());
        assert!(s.set_preview(ch(1), u(1), Bytes::from_static(b"webp")));
        assert!(s.preview(ch(1), u(1)).is_some());
        assert!(s.end(ch(1), u(1)).is_some());
        assert!(s.preview(ch(1), u(1)).is_none());
        assert!(!s.set_preview(ch(1), u(1), Bytes::from_static(b"webp")));
        assert!(s.end(ch(1), u(1)).is_none());
        assert_eq!(s.epoch(ch(1), u(1)), None);
    }

    #[test]
    fn preview_etag_is_16_hex_and_tracks_content() {
        let s = StreamState::default();
        s.start(ch(1), u(1), settings(30), "t".into());
        s.set_preview(ch(1), u(1), Bytes::from_static(b"a"));
        let (_, e1) = s.preview(ch(1), u(1)).unwrap();
        s.set_preview(ch(1), u(1), Bytes::from_static(b"b"));
        let (b, e2) = s.preview(ch(1), u(1)).unwrap();
        assert_eq!(e1.len(), 16);
        assert!(e1.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(e1, e2);
        assert_eq!(&b[..], b"b");
    }

    #[test]
    fn viewer_add_is_idempotent() {
        let s = StreamState::default();
        assert_eq!(s.add_viewer(ch(1), u(1), u(2)), None, "no stream");
        s.start(ch(1), u(1), settings(30), "t".into());
        assert_eq!(s.add_viewer(ch(1), u(1), u(2)), Some(vec![u(2)]));
        assert_eq!(s.add_viewer(ch(1), u(1), u(2)), None);
        assert_eq!(s.remove_viewer(ch(1), u(1), u(2)), Some(vec![]));
        assert_eq!(s.remove_viewer(ch(1), u(1), u(2)), None);
    }

    #[test]
    fn remove_viewer_everywhere_reports_each_stream() {
        let s = StreamState::default();
        s.start(ch(1), u(1), settings(30), "t".into());
        s.start(ch(1), u(3), settings(30), "t".into());
        s.start(ch(2), u(4), settings(30), "t".into());
        s.add_viewer(ch(1), u(1), u(2));
        s.add_viewer(ch(1), u(1), u(5));
        s.add_viewer(ch(1), u(3), u(2));
        s.add_viewer(ch(2), u(4), u(2));
        let mut got = s.remove_viewer_everywhere(ch(1), u(2));
        got.sort_by_key(|(streamer, _)| *streamer);
        assert_eq!(got, vec![(u(1), vec![u(5)]), (u(3), vec![])]);
        assert_eq!(s.get(ch(2), u(4)).unwrap().viewers, vec![u(2)]);
        assert!(s.remove_viewer_everywhere(ch(1), u(2)).is_empty());
    }

    #[test]
    fn set_status_unchanged_is_none() {
        let s = StreamState::default();
        assert!(
            s.set_status(ch(1), u(1), StreamStatus::Reconnecting)
                .is_none()
        );
        s.start(ch(1), u(1), settings(30), "t".into());
        assert!(s.set_status(ch(1), u(1), StreamStatus::Live).is_none());
        let info = s
            .set_status(ch(1), u(1), StreamStatus::Reconnecting)
            .unwrap();
        assert_eq!(info.status, StreamStatus::Reconnecting);
        assert!(
            s.set_status(ch(1), u(1), StreamStatus::Reconnecting)
                .is_none()
        );
    }

    #[test]
    fn end_any_for_covers_all_channels() {
        let s = StreamState::default();
        s.start(ch(1), u(1), settings(30), "t".into());
        s.start(ch(2), u(1), settings(30), "t".into());
        s.start(ch(1), u(2), settings(30), "t".into());
        s.set_preview(ch(2), u(1), Bytes::from_static(b"x"));
        let mut ended = s.end_any_for(u(1));
        ended.sort_by_key(|i| i.channel_id);
        assert_eq!(ended.len(), 2);
        assert_eq!(ended[0].channel_id, ch(1));
        assert_eq!(ended[1].channel_id, ch(2));
        assert!(s.preview(ch(2), u(1)).is_none());
        assert_eq!(s.all().len(), 1);
        assert!(s.end_any_for(u(1)).is_empty());
    }

    #[test]
    fn a_replaced_video_connection_leaving_is_stale() {
        let s = StreamState::default();
        assert!(s.video_left(ch(1), u(1), "PA_x"), "unknown: current");
        s.video_joined(ch(1), u(1), "PA_1");
        s.video_joined(ch(1), u(1), "PA_2");
        assert!(!s.video_left(ch(1), u(1), "PA_1"));
        assert!(s.video_left(ch(1), u(1), "PA_2"));
        // gone: a repeat is unknown again, not stale
        assert!(s.video_left(ch(1), u(1), "PA_2"));
        s.video_joined(ch(1), u(1), "PA_3");
        assert!(s.video_left(ch(2), u(1), "PA_1"), "another room is its own");
    }

    #[test]
    fn epochs_are_unique_per_episode() {
        let s = StreamState::default();
        s.start(ch(1), u(1), settings(30), "t".into());
        let e0 = s.epoch(ch(1), u(1)).unwrap();
        let e1 = s.bump_epoch(ch(1), u(1));
        assert_ne!(e0, e1);
        assert_eq!(s.epoch(ch(1), u(1)), Some(e1));
        assert_eq!(s.bump_epoch(ch(1), u(9)), 0);
        s.end(ch(1), u(1));
        s.start(ch(1), u(1), settings(30), "t".into());
        let e2 = s.epoch(ch(1), u(1)).unwrap();
        assert!(e2 != e0 && e2 != e1, "a restarted stream is a new episode");
    }

    const GRACE: Duration = Duration::from_secs(60);

    fn checked(s: &StreamState) -> Vec<(ChannelId, UserId, StreamStatus)> {
        s.streams_to_check()
            .into_iter()
            .map(|c| (c.channel, c.user, c.status))
            .collect()
    }

    #[test]
    fn video_missing_for_the_grace_ends_the_stream_and_its_streamers_watches() {
        let s = StreamState::default();
        let t0 = Instant::now();
        s.start(ch(1), u(1), settings(30), "t".into());
        s.start(ch(1), u(2), settings(30), "t".into());
        s.add_viewer(ch(1), u(2), u(1)); // u1 watches u2
        s.add_viewer(ch(1), u(1), u(3)); // u3 watches u1
        assert_eq!(s.video_seen(ch(1), u(1), false, t0, GRACE), Seen::FirstMiss);
        assert_eq!(
            s.video_seen(
                ch(1),
                u(1),
                false,
                t0 + GRACE - Duration::from_millis(1),
                GRACE
            ),
            Seen::StillMissing
        );
        let Seen::Ended { info, viewers } = s.video_seen(ch(1), u(1), false, t0 + GRACE, GRACE)
        else {
            panic!("must end");
        };
        assert_eq!((info.user_id, info.viewers), (u(1), vec![u(3)]));
        assert_eq!(viewers, vec![(u(2), vec![])]);
        assert!(s.get(ch(1), u(1)).is_none());
        assert!(s.get(ch(1), u(2)).unwrap().viewers.is_empty());
    }

    #[test]
    fn video_seen_again_restarts_the_clock() {
        let s = StreamState::default();
        let t0 = Instant::now();
        s.start(ch(1), u(1), settings(30), "t".into());
        s.video_seen(ch(1), u(1), false, t0, GRACE);
        assert_eq!(
            s.video_seen(ch(1), u(1), true, t0 + GRACE / 2, GRACE),
            Seen::Present
        );
        assert_eq!(
            s.video_seen(ch(1), u(1), false, t0 + GRACE, GRACE),
            Seen::FirstMiss
        );
        assert!(
            s.video_seen(ch(1), u(1), false, t0 + GRACE * 2, GRACE)
                .ended()
        );
    }

    #[test]
    fn a_rejoin_or_a_reannounce_while_asking_restarts_the_clock() {
        // LiveKit's "not there" can be older than a join or go-live that landed meanwhile
        let s = StreamState::default();
        let t0 = Instant::now();
        s.start(ch(1), u(1), settings(30), "t".into());
        s.video_seen(ch(1), u(1), false, t0, GRACE);
        s.video_joined(ch(1), u(1), "PA_1");
        assert!(!s.video_seen(ch(1), u(1), false, t0 + GRACE, GRACE).ended());
        s.start(ch(1), u(1), settings(60), "t".into()); // re-announce: an update
        assert!(
            !s.video_seen(ch(1), u(1), false, t0 + GRACE * 2, GRACE)
                .ended()
        );
        assert!(
            s.video_seen(ch(1), u(1), false, t0 + GRACE * 3, GRACE)
                .ended()
        );
    }

    #[test]
    fn only_live_streams_are_timed_but_all_are_listed_for_the_check() {
        let s = StreamState::default();
        let t0 = Instant::now();
        assert_eq!(
            s.video_seen(ch(1), u(1), false, t0, GRACE),
            Seen::Ignored,
            "no stream"
        );
        s.start(ch(1), u(1), settings(30), "t".into());
        s.start(ch(2), u(2), settings(30), "t".into());
        s.video_seen(ch(1), u(1), false, t0, GRACE);
        // the leave webhook got there: its own timer ends it, not this
        s.set_status(ch(1), u(1), StreamStatus::Reconnecting);
        assert_eq!(
            checked(&s),
            vec![
                (ch(1), u(1), StreamStatus::Reconnecting),
                (ch(2), u(2), StreamStatus::Live)
            ]
        );
        assert_eq!(
            s.video_seen(ch(1), u(1), false, t0 + GRACE * 2, GRACE),
            Seen::Ignored
        );
        assert!(s.get(ch(1), u(1)).is_some());
        // back (join webhook): a fresh clock
        s.video_joined(ch(1), u(1), "PA_2");
        s.set_status(ch(1), u(1), StreamStatus::Live);
        assert_eq!(
            s.video_seen(ch(1), u(1), false, t0 + GRACE * 2, GRACE),
            Seen::FirstMiss
        );
    }

    #[test]
    fn a_stream_ended_and_started_again_meanwhile_gets_a_fresh_clock() {
        let s = StreamState::default();
        let t0 = Instant::now();
        s.start(ch(1), u(1), settings(30), "t".into());
        s.video_seen(ch(1), u(1), false, t0, GRACE);
        s.end(ch(1), u(1));
        s.start(ch(1), u(1), settings(30), "t".into()); // a new stream, not an update
        assert_eq!(
            s.video_seen(ch(1), u(1), false, t0 + GRACE, GRACE),
            Seen::FirstMiss
        );
        assert!(s.get(ch(1), u(1)).is_some());
    }

    #[test]
    fn ending_for_a_gone_video_connection_forgets_its_sid() {
        let s = StreamState::default();
        let t0 = Instant::now();
        s.start(ch(1), u(1), settings(30), "t".into());
        s.video_joined(ch(1), u(1), "PA_old"); // its leave webhook never came
        s.video_seen(ch(1), u(1), false, t0, GRACE);
        assert!(s.video_seen(ch(1), u(1), false, t0 + GRACE, GRACE).ended());
        // a later connection (whose join was missed too) leaving is current, not stale
        assert!(s.video_left(ch(1), u(1), "PA_new"));
    }

    #[test]
    fn a_reconnecting_stream_livekit_lists_again_is_live_again_unless_the_answer_is_stale() {
        let s = StreamState::default();
        s.start(ch(1), u(1), settings(30), "t".into());
        assert!(
            s.video_back(ch(1), u(1), s.epoch(ch(1), u(1)).unwrap())
                .is_none(),
            "live: nothing to do"
        );
        s.set_status(ch(1), u(1), StreamStatus::Reconnecting);
        let read = s.bump_epoch(ch(1), u(1)); // the leave webhook's episode
        // read before asking; then the connection came back and left again (a new episode)
        s.set_status(ch(1), u(1), StreamStatus::Live);
        s.bump_epoch(ch(1), u(1));
        s.set_status(ch(1), u(1), StreamStatus::Reconnecting);
        let now = s.bump_epoch(ch(1), u(1));
        assert!(s.video_back(ch(1), u(1), read).is_none(), "stale");
        let info = s.video_back(ch(1), u(1), now).unwrap();
        assert_eq!(info.status, StreamStatus::Live);
        assert_ne!(
            s.epoch(ch(1), u(1)),
            Some(now),
            "a new episode: the leave's timer stands down"
        );
        assert!(s.video_back(ch(1), u(1), now).is_none());
    }

    // --- ghosts ---

    const TTL: Duration = Duration::from_secs(600);

    fn ghost_settings(allow: bool) -> StreamSettings {
        StreamSettings {
            allow_ghosts: allow,
            ..settings(30)
        }
    }

    /// `user`'s ghost (login `sess`) in `c` with its `#v` up (connection `sid`); its id.
    fn connected_ghost(
        s: &StreamState,
        c: ChannelId,
        user: UserId,
        sess: &str,
        sid: &str,
    ) -> String {
        let (v, gv) = s
            .ghost_ticket(c, user, sess, StreamRole::V, Instant::now())
            .unwrap();
        let id = v.trim_end_matches("#v").to_owned();
        assert!(s.ghost_admit(c, &id, StreamRole::V, Some(&gv)).is_some());
        s.ghost_connected(c, &id, sid);
        id
    }

    #[test]
    fn ghosts_need_every_stream_in_the_channel_to_allow_them() {
        let s = StreamState::default();
        let t = Instant::now();
        assert!(!s.ghosts_allowed(ch(1)), "nothing live");
        assert!(
            s.ghost_ticket(ch(1), u(9), "sess", StreamRole::V, t)
                .is_none()
        );
        s.start(ch(1), u(1), ghost_settings(true), "t".into());
        assert!(s.ghosts_allowed(ch(1)));
        s.start(ch(1), u(2), ghost_settings(false), "t".into());
        assert!(
            !s.ghosts_allowed(ch(1)),
            "one voice-only stream closes the room"
        );
        assert!(
            s.ghost_ticket(ch(1), u(9), "sess", StreamRole::V, t)
                .is_none()
        );
        s.start(ch(2), u(3), ghost_settings(true), "t".into());
        assert!(s.ghosts_allowed(ch(2)), "per channel");
    }

    #[test]
    fn one_ghost_identity_per_login_session_and_channel_until_it_ends() {
        let s = StreamState::default();
        let t = Instant::now();
        s.start(ch(1), u(1), ghost_settings(true), "t".into());
        let (v, gv) = s
            .ghost_ticket(ch(1), u(9), "sess", StreamRole::V, t)
            .unwrap();
        let (a, ga) = s
            .ghost_ticket(ch(1), u(9), "sess", StreamRole::A, t)
            .unwrap();
        let id = v.strip_suffix("#v").unwrap().to_owned();
        assert_eq!(a, format!("{id}#a"), "both connections share the session");
        assert_ne!(gv, ga, "a grant per connection");
        assert!(!id.contains(&u(9).to_string().to_lowercase()));
        // a refresh: same identity and grant
        assert_eq!(
            s.ghost_ticket(ch(1), u(9), "sess", StreamRole::V, t),
            Some((v.clone(), gv.clone()))
        );
        // another device of the same person is another ghost
        let (other, _) = s
            .ghost_ticket(ch(1), u(9), "sess2", StreamRole::V, t)
            .unwrap();
        assert_ne!(other, v);
        // the #v connection ends the session: the next watch is a new ghost
        s.ghost_connected(ch(1), &id, "PA_1");
        s.ghost_left(ch(1), &id, StreamRole::V, "PA_1");
        let (again, _) = s
            .ghost_ticket(ch(1), u(9), "sess", StreamRole::V, t)
            .unwrap();
        assert_ne!(again, v, "fresh per watch session");
    }

    #[test]
    fn a_ghost_session_never_joined_expires_after_the_ticket_ttl() {
        let s = StreamState::default();
        let t = Instant::now();
        s.start(ch(1), u(1), ghost_settings(true), "t".into());
        let (v, _) = s
            .ghost_ticket(ch(1), u(9), "sess", StreamRole::V, t)
            .unwrap();
        let (still, _) = s
            .ghost_ticket(ch(1), u(9), "sess", StreamRole::V, t + TTL / 2)
            .unwrap();
        assert_eq!(still, v);
        let (later, _) = s
            .ghost_ticket(ch(1), u(9), "sess", StreamRole::V, t + TTL * 2)
            .unwrap();
        assert_ne!(later, v);
    }

    #[test]
    fn admission_needs_the_session_grant_for_that_role_and_an_open_room() {
        let s = StreamState::default();
        let t = Instant::now();
        s.start(ch(1), u(1), ghost_settings(true), "t".into());
        let (v, gv) = s
            .ghost_ticket(ch(1), u(9), "sess", StreamRole::V, t)
            .unwrap();
        let (_, ga) = s
            .ghost_ticket(ch(1), u(9), "sess", StreamRole::A, t)
            .unwrap();
        let id = v.strip_suffix("#v").unwrap().to_owned();
        assert!(s.ghost_admit(ch(1), &id, StreamRole::V, None).is_none());
        assert!(
            s.ghost_admit(ch(1), &id, StreamRole::V, Some(&ga))
                .is_none(),
            "the #a grant"
        );
        assert!(
            s.ghost_admit(ch(2), &id, StreamRole::V, Some(&gv))
                .is_none(),
            "another room"
        );
        assert!(
            s.ghost_admit(ch(1), &new_ghost_id(), StreamRole::V, Some(&gv))
                .is_none()
        );
        assert_eq!(
            s.ghost_admit(ch(1), &id, StreamRole::V, Some(&gv)),
            Some((u(9), "sess".to_owned()))
        );
        s.start(ch(1), u(2), ghost_settings(false), "t".into());
        assert!(
            s.ghost_admit(ch(1), &id, StreamRole::A, Some(&ga))
                .is_none()
        );
    }

    #[test]
    fn ghost_watches_are_counted_once_connected_and_never_named() {
        let s = StreamState::default();
        let t = Instant::now();
        s.start(ch(1), u(1), ghost_settings(true), "t".into());
        let id = connected_ghost(&s, ch(1), u(9), "sess", "PA_9");
        assert_eq!(
            s.ghost_watch(ch(1), u(9), "sess", u(1), t),
            GhostWatch::Added {
                count: 1,
                id: id.clone()
            }
        );
        assert_eq!(
            s.ghost_watch(ch(1), u(9), "sess", u(1), t),
            GhostWatch::Already
        );
        // a watch with no connection behind it is nobody watching (yet)
        assert!(matches!(
            s.ghost_watch(ch(1), u(8), "other", u(1), t),
            GhostWatch::Added { count: 1, .. }
        ));
        let other = s
            .ghost_ticket(ch(1), u(8), "other", StreamRole::V, t)
            .unwrap()
            .0;
        assert_eq!(
            s.ghost_connected(ch(1), other.trim_end_matches("#v"), "PA_8"),
            vec![(u(1), 2)]
        );
        assert!(
            s.ghost_connected(ch(1), other.trim_end_matches("#v"), "PA_8b")
                .is_empty(),
            "a reconnect is the same ghost"
        );
        let info = s.get(ch(1), u(1)).unwrap();
        assert_eq!((info.viewers.len(), info.ghosts), (0, 2));
        assert_eq!(
            s.ghost_watch(ch(1), u(9), "sess", u(5), t),
            GhostWatch::NoStream
        );
        assert_eq!(s.ghost_unwatch(ch(1), "sess", u(1)), Some(1));
        assert_eq!(s.ghost_unwatch(ch(1), "sess", u(1)), None);
        s.start(ch(1), u(2), ghost_settings(false), "t".into());
        assert_eq!(
            s.ghost_watch(ch(1), u(9), "sess", u(1), t),
            GhostWatch::NotAllowed
        );
    }

    #[test]
    fn a_ghost_leaving_lowers_the_count_and_a_stale_leave_does_not() {
        let s = StreamState::default();
        let t = Instant::now();
        s.start(ch(1), u(1), ghost_settings(true), "t".into());
        let id = connected_ghost(&s, ch(1), u(9), "sess", "PA_1");
        s.ghost_watch(ch(1), u(9), "sess", u(1), t);
        s.ghost_connected(ch(1), &id, "PA_2"); // a reconnect
        assert!(
            s.ghost_left(ch(1), &id, StreamRole::V, "PA_1").is_empty(),
            "stale"
        );
        assert!(
            s.ghost_left(ch(1), &id, StreamRole::A, "PA_x").is_empty(),
            "#a alone"
        );
        assert_eq!(
            s.ghost_left(ch(1), &id, StreamRole::V, "PA_2"),
            vec![(u(1), 0)]
        );
        assert_eq!(s.get(ch(1), u(1)).unwrap().ghosts, 0);
    }

    #[test]
    fn purging_ghosts_reports_what_to_remove_and_the_new_counts() {
        let s = StreamState::default();
        let t = Instant::now();
        s.start(ch(1), u(1), ghost_settings(true), "t".into());
        s.start(ch(2), u(2), ghost_settings(true), "t".into());
        connected_ghost(&s, ch(1), u(9), "sess", "PA_a");
        connected_ghost(&s, ch(1), u(8), "other", "PA_b");
        connected_ghost(&s, ch(2), u(9), "sess", "PA_c");
        s.ghost_watch(ch(1), u(9), "sess", u(1), t);
        s.ghost_watch(ch(1), u(8), "other", u(1), t);
        s.ghost_watch(ch(2), u(9), "sess", u(2), t);
        let p = s.purge_ghosts(|g| g.channel == ch(1) && g.user == u(9));
        assert_eq!(p.removed.len(), 1);
        assert_eq!(p.removed[0].0, ch(1));
        assert_eq!(p.counts, vec![(ch(1), u(1), 1)]);
        assert_eq!(
            s.get(ch(2), u(2)).unwrap().ghosts,
            1,
            "other channels untouched"
        );
        let p = s.purge_ghosts(|g| g.session == "sess");
        assert_eq!(p.counts, vec![(ch(2), u(2), 0)]);
        assert!(s.purge_ghosts(|g| g.session == "sess").removed.is_empty());
    }

    #[test]
    fn a_stream_ending_forgets_its_ghost_watches() {
        let s = StreamState::default();
        let t = Instant::now();
        s.start(ch(1), u(1), ghost_settings(true), "t".into());
        connected_ghost(&s, ch(1), u(9), "sess", "PA_1");
        s.ghost_watch(ch(1), u(9), "sess", u(1), t);
        s.end(ch(1), u(1));
        s.start(ch(1), u(1), ghost_settings(true), "t".into());
        assert_eq!(s.get(ch(1), u(1)).unwrap().ghosts, 0);
        assert!(matches!(
            s.ghost_watch(ch(1), u(9), "sess", u(1), t),
            GhostWatch::Added { count: 1, .. }
        ));
    }
}
