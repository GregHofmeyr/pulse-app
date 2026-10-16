//! Mute / deafen state machine (spec §6.5).

use serde::Serialize;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Controls {
    pub muted: bool,
    pub deafened: bool,
    #[serde(skip)]
    muted_before_deafen: bool,
    /// How many UI views show the voice meter (the voice settings page). Lives here because, like
    /// mute, it outlasts a session and the audio thread reads it. Not part of the UI's state.
    #[serde(skip)]
    meter_views: u16,
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

    /// A view showing the voice meter appeared (`true`) or went away (`false`).
    pub fn watch_meters(&mut self, on: bool) {
        self.meter_views = if on {
            self.meter_views.saturating_add(1)
        } else {
            self.meter_views.saturating_sub(1)
        };
    }

    /// The page (re)loaded: views from the old page can't send their close any more.
    pub fn reset_meters(&mut self) {
        self.meter_views = 0;
    }

    /// Levels go to the UI only while this holds.
    pub fn meters_shown(&self) -> bool {
        self.meter_views > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meters_are_shown_while_any_view_watches() {
        let mut c = Controls::default();
        assert!(!c.meters_shown());
        c.watch_meters(true);
        c.watch_meters(true); // a second view (or a reopen racing the close)
        c.watch_meters(false);
        assert!(c.meters_shown(), "one view still open");
        c.watch_meters(false);
        c.watch_meters(false); // an unmatched close can't go negative
        assert!(!c.meters_shown());
        c.watch_meters(true);
        assert!(
            c.meters_shown(),
            "reopening after a stray close still shows levels"
        );
    }

    #[test]
    fn a_reloaded_page_forgets_the_old_pages_meter_views() {
        let mut c = Controls::default();
        c.watch_meters(true); // settings open when the renderer died: its close never comes
        c.reset_meters(); // the new page's restore_session
        assert!(!c.meters_shown());
        c.watch_meters(true);
        c.watch_meters(false);
        assert!(
            !c.meters_shown(),
            "the new page's open/close pair balances again"
        );
    }

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
