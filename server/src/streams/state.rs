//! Who is streaming in which voice channel, who is watching, and each stream's latest preview.
//! In memory only: after a restart live clients re-announce themselves. One stream per
//! (channel, user); a re-start while live is an update.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use pulse_protocol::ids::{ChannelId, UserId};
use pulse_protocol::rest::{StreamInfo, StreamSettings, StreamStatus};
use sha2::{Digest, Sha256};

struct Stream {
    info: StreamInfo,
    preview: Option<(Bytes, String)>,
    epoch: u64,
}

#[derive(Default)]
struct Inner {
    streams: HashMap<(ChannelId, UserId), Stream>,
    /// Each person's current `#v` connection (LiveKit participant sid) in a stream room, streaming
    /// or watching. A rejoin replaces it, and the replaced one's late leave is then ignored.
    video_sids: HashMap<(ChannelId, UserId), String>,
    /// Source of epochs: never reused, so a new stream never matches an old timer's epoch.
    last_epoch: u64,
}

impl Inner {
    fn next_epoch(&mut self) -> u64 {
        self.last_epoch += 1;
        self.last_epoch
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
            return Started::Updated(s.info.clone());
        }
        let info = StreamInfo {
            channel_id: channel,
            user_id: user,
            settings,
            status: StreamStatus::Live,
            viewers: Vec::new(),
            started_at: now,
        };
        let epoch = inner.next_epoch();
        inner.streams.insert(
            (channel, user),
            Stream {
                info: info.clone(),
                preview: None,
                epoch,
            },
        );
        Started::New(info)
    }

    /// End `user`'s stream in `channel`, dropping its preview.
    pub fn end(&self, channel: ChannelId, user: UserId) -> Option<StreamInfo> {
        let mut inner = self.inner.lock().unwrap();
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
        let mut inner = self.inner.lock().unwrap();
        let mut out = Vec::new();
        for ((c, streamer), s) in inner.streams.iter_mut() {
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
}
