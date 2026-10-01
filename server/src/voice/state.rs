//! Who is in which voice room right now, with their mute/deafen flags. In memory only, so it is
//! empty after a restart: LiveKit does NOT re-report participants who were already connected
//! (rebuilding from LiveKit's ListParticipants at startup is a follow-up).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use pulse_protocol::gateway::VoiceFlags;
use pulse_protocol::ids::{ChannelId, UserId};

type Rooms = HashMap<ChannelId, HashMap<UserId, VoiceFlags>>;

#[derive(Clone, Default)]
pub struct VoiceState {
    rooms: Arc<Mutex<Rooms>>,
}

impl VoiceState {
    pub fn join(&self, channel: ChannelId, user: UserId) -> bool {
        self.rooms
            .lock()
            .unwrap()
            .entry(channel)
            .or_default()
            .insert(user, VoiceFlags::default())
            .is_none()
    }

    pub fn leave(&self, channel: ChannelId, user: UserId) -> bool {
        let mut rooms = self.rooms.lock().unwrap();
        let removed = rooms
            .get_mut(&channel)
            .is_some_and(|r| r.remove(&user).is_some());
        if rooms.get(&channel).is_some_and(|r| r.is_empty()) {
            rooms.remove(&channel);
        }
        removed
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
        let mut rooms = self.rooms.lock().unwrap();
        for (channel, members) in rooms.iter_mut() {
            if let Some(f) = members.get_mut(&user) {
                *f = flags;
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
            .map(|(c, m)| (*c, m.iter().map(|(u, f)| (*u, *f)).collect()))
            .collect()
    }
}
