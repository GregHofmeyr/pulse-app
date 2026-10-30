//! Who is in which voice room right now, with their mute/deafen flags. In memory; after a
//! restart it is rebuilt from LiveKit's room list (`voice::restore`), because LiveKit does NOT
//! re-report participants who were already connected.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use pulse_protocol::gateway::VoiceFlags;
use pulse_protocol::ids::{ChannelId, UserId};

/// One user's live LiveKit connection in a room. `sid` changes on every (re)join.
struct Member {
    flags: VoiceFlags,
    sid: String,
}

type Rooms = HashMap<ChannelId, HashMap<UserId, Member>>;

/// What a `participant_left` meant.
#[derive(Debug, PartialEq, Eq)]
pub enum Left {
    Removed,
    NotPresent,
    /// That connection was already replaced by a rejoin (LiveKit kicks the old one): ignore it.
    Stale,
}

#[derive(Clone, Default)]
pub struct VoiceState {
    rooms: Arc<Mutex<Rooms>>,
    /// Last flags each user declared, in a room or not: the client announces them right after
    /// joining, which can beat LiveKit's join webhook.
    declared: Arc<Mutex<HashMap<UserId, VoiceFlags>>>,
    /// While a LiveKit snapshot is being restored: the sids that left since it was taken, which
    /// the snapshot may still list. `None` the rest of the time, so nothing piles up. Locked
    /// after `rooms`.
    left_since_snapshot: Arc<Mutex<Option<HashSet<String>>>>,
}

/// Records leaves from [`VoiceState::snapshot`] until dropped.
#[must_use]
pub struct Snapshot(VoiceState);

impl Drop for Snapshot {
    fn drop(&mut self) {
        *self.0.left_since_snapshot.lock().unwrap() = None;
    }
}

impl VoiceState {
    /// Call just before asking LiveKit who is connected, and hold the guard until the answer is
    /// applied: a leave that lands in between keeps [`Self::restore`] from adding that
    /// connection back. One snapshot at a time (the boot try, then the retries, in turn).
    pub fn snapshot(&self) -> Snapshot {
        *self.left_since_snapshot.lock().unwrap() = Some(HashSet::new());
        Snapshot(self.clone())
    }

    /// Record `user`'s connection `sid`; true if they weren't already in the room.
    pub fn join(&self, channel: ChannelId, user: UserId, sid: &str) -> bool {
        let flags = self.declared(user);
        self.rooms
            .lock()
            .unwrap()
            .entry(channel)
            .or_default()
            .insert(
                user,
                Member {
                    flags,
                    sid: sid.to_owned(),
                },
            )
            .is_none()
    }

    /// Add `user` from LiveKit's room list unless a webhook already placed them (its `sid` is
    /// newer than our snapshot) or that connection has left since the snapshot. True if added.
    pub fn restore(&self, channel: ChannelId, user: UserId, sid: &str) -> bool {
        let flags = self.declared(user);
        let mut rooms = self.rooms.lock().unwrap();
        if rooms.values().any(|r| r.contains_key(&user)) {
            return false;
        }
        if let Some(left) = self.left_since_snapshot.lock().unwrap().as_ref()
            && left.contains(sid)
        {
            return false;
        }
        rooms.entry(channel).or_default().insert(
            user,
            Member {
                flags,
                sid: sid.to_owned(),
            },
        );
        true
    }

    /// Remove `user` if `sid` is their current connection (an empty `sid` matches any).
    pub fn leave(&self, channel: ChannelId, user: UserId, sid: &str) -> Left {
        let mut rooms = self.rooms.lock().unwrap();
        if let Some(left) = self.left_since_snapshot.lock().unwrap().as_mut()
            && !sid.is_empty()
        {
            left.insert(sid.to_owned());
        }
        let Some(room) = rooms.get_mut(&channel) else {
            return Left::NotPresent;
        };
        let left = match room.get(&user) {
            None => Left::NotPresent,
            Some(m) if !sid.is_empty() && !m.sid.is_empty() && m.sid != sid => Left::Stale,
            Some(_) => {
                room.remove(&user);
                Left::Removed
            }
        };
        if room.is_empty() {
            rooms.remove(&channel);
        }
        left
    }

    /// Flags the user last declared (default if never).
    pub fn declared(&self, user: UserId) -> VoiceFlags {
        self.declared
            .lock()
            .unwrap()
            .get(&user)
            .copied()
            .unwrap_or_default()
    }

    pub fn members(&self, channel: ChannelId) -> HashSet<UserId> {
        self.rooms
            .lock()
            .unwrap()
            .get(&channel)
            .map(|r| r.keys().copied().collect())
            .unwrap_or_default()
    }

    /// Update a user's flags; returns the room they are in, so the change can be published.
    /// None if not in voice, or if the flags didn't change (nothing to publish).
    pub fn set_flags(&self, user: UserId, flags: VoiceFlags) -> Option<ChannelId> {
        self.declared.lock().unwrap().insert(user, flags);
        let mut rooms = self.rooms.lock().unwrap();
        for (channel, members) in rooms.iter_mut() {
            if let Some(m) = members.get_mut(&user) {
                if m.flags == flags {
                    return None;
                }
                m.flags = flags;
                return Some(*channel);
            }
        }
        None
    }

    /// The room `user` is in and their current flags (None if not in voice).
    pub fn room_flags(&self, user: UserId) -> Option<(ChannelId, VoiceFlags)> {
        self.rooms
            .lock()
            .unwrap()
            .iter()
            .find_map(|(c, members)| members.get(&user).map(|m| (*c, m.flags)))
    }

    pub fn rooms(&self) -> Vec<(ChannelId, Vec<(UserId, VoiceFlags)>)> {
        self.rooms
            .lock()
            .unwrap()
            .iter()
            .map(|(c, m)| (*c, m.iter().map(|(u, m)| (*u, m.flags)).collect()))
            .collect()
    }
}
