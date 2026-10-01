//! Mute / deafen state machine (spec §6.5).

use serde::Serialize;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Controls {
    pub muted: bool,
    pub deafened: bool,
    #[serde(skip)]
    muted_before_deafen: bool,
}

impl Controls {
    /// Discord behaviour: un-muting while deafened also undeafens.
    pub fn toggle_mute(&mut self) {
        if self.deafened {
            self.deafened = false;
            self.muted = false;
        } else {
            self.muted = !self.muted;
        }
    }

    /// Deafen also mutes; undeafen restores whatever mute state you had before.
    pub fn toggle_deafen(&mut self) {
        if self.deafened {
            self.deafened = false;
            self.muted = self.muted_before_deafen;
        } else {
            self.muted_before_deafen = self.muted;
            self.deafened = true;
            self.muted = true;
        }
    }

    pub fn mic_open(&self) -> bool {
        !self.muted && !self.deafened
    }

    pub fn playout_on(&self) -> bool {
        !self.deafened
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mute_then_deafen_then_undeafen_stays_muted() {
        let mut c = Controls::default();
        c.toggle_mute();
        c.toggle_deafen();
        assert!(c.muted && c.deafened);
        c.toggle_deafen();
        assert!(c.muted && !c.deafened);
    }

    #[test]
    fn deafen_from_unmuted_then_undeafen_unmutes() {
        let mut c = Controls::default();
        c.toggle_deafen();
        assert!(c.muted && c.deafened);
        c.toggle_deafen();
        assert!(!c.muted && !c.deafened);
    }

    #[test]
    fn mute_toggle_while_deafened_undeafens_and_unmutes() {
        let mut c = Controls::default();
        c.toggle_deafen();
        c.toggle_mute();
        assert!(!c.muted && !c.deafened);
    }

    #[test]
    fn mic_and_playout_gates() {
        let mut c = Controls::default();
        assert!(c.mic_open() && c.playout_on());
        c.toggle_mute();
        assert!(!c.mic_open() && c.playout_on());
        c.toggle_mute();
        c.toggle_deafen();
        assert!(!c.mic_open() && !c.playout_on());
    }
}
