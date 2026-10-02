//! Who is in which voice room right now, with their mute/deafen flags. In memory only, so it is
//! empty after a restart: LiveKit does NOT re-report participants who were already connected
//! (rebuilding from LiveKit's ListParticipants at startup is a follow-up).

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
}

impl VoiceState {
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

    /// Remove `user` if `sid` is their current connection (an empty `sid` matches any).
    pub fn leave(&self, channel: ChannelId, user: UserId, sid: &str) -> Left {
        let mut rooms = self.rooms.lock().unwrap();
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

    /// Update a user's flags; returns the room they are in (None if not in voice).
    pub fn set_flags(&self, user: UserId, flags: VoiceFlags) -> Option<ChannelId> {
        self.declared.lock().unwrap().insert(user, flags);
        let mut rooms = self.rooms.lock().unwrap();
        for (channel, members) in rooms.iter_mut() {
            if let Some(m) = members.get_mut(&user) {
                m.flags = flags;
                return Some(*channel);
            }
        }
        None
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
