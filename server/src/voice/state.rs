//! Who is in which voice room right now. In memory only: LiveKit re-reports after a restart.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use pulse_protocol::ids::{ChannelId, UserId};

#[derive(Clone, Default)]
pub struct VoiceState {
    rooms: Arc<Mutex<HashMap<ChannelId, HashSet<UserId>>>>,
}

impl VoiceState {
    pub fn join(&self, channel: ChannelId, user: UserId) -> bool {
        self.rooms
            .lock()
            .unwrap()
            .entry(channel)
            .or_default()
            .insert(user)
    }

    pub fn leave(&self, channel: ChannelId, user: UserId) -> bool {
        let mut rooms = self.rooms.lock().unwrap();
        let removed = rooms.get_mut(&channel).is_some_and(|r| r.remove(&user));
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
            .cloned()
            .unwrap_or_default()
    }
}
