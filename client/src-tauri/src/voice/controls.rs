//! Mute / deafen state machine (spec §6.5).

use serde::Serialize;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Controls {
    pub muted: bool,
    pub deafened: bool,
    /// Someone in the server muted you (#140): your mic stays closed whatever your own mute
    /// says, until someone else undoes it. Set from the gateway's state, never by toggles.
    pub server_muted: bool,
    /// Someone in the server deafened you: you hear nothing and your mic stays closed.
    pub server_deafened: bool,
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
        !self.muted && !self.deafened && !self.server_muted && !self.server_deafened
    }

    pub fn playout_on(&self) -> bool {
        !self.deafened && !self.server_deafened
    }

    /// The server's mute / deafen of you, as the gateway last said. Your own toggles are kept.
    pub fn set_server(&mut self, muted: bool, deafened: bool) {
        self.server_muted = muted;
        self.server_deafened = deafened;
    }

    /// Talking now counts as talking into your own muted mic (The Doos): muted by you, and not
    /// by the server (a server mute isn't yours to forget).
    pub fn counts_muted_talk(&self) -> bool {
        (self.muted || self.deafened) && !self.server_muted && !self.server_deafened
    }

    /// Talking now plays the "you're muted" cue (#252): your own mute, not deafened (you couldn't
    /// hear it) and not by the server (not yours to undo).
    pub fn cues_muted_talk(&self) -> bool {
        self.muted && !self.deafened && !self.server_muted && !self.server_deafened
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

    /// The mic chain has to clean audio: it goes out, or the meter shows the cleaned level.
    /// Muted with no meter on screen, the denoiser can idle.
    pub fn denoise_needed(&self) -> bool {
        self.mic_open() || self.meters_shown()
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
    fn denoiser_idles_only_when_muted_and_unwatched() {
        let mut c = Controls::default();
        assert!(c.denoise_needed(), "talking");
        c.toggle_mute();
        assert!(!c.denoise_needed(), "muted, nobody looking");
        c.watch_meters(true);
        assert!(c.denoise_needed(), "muted but the meter is open");
        c.watch_meters(false);
        c.toggle_deafen();
        assert!(!c.denoise_needed(), "deafened (implies muted)");
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
    fn server_mute_closes_the_mic_but_keeps_self_state() {
        let mut c = Controls::default();
        c.set_server(true, false);
        assert!(!c.mic_open() && c.playout_on());
        assert!(!c.muted, "your own mute is yours");
        c.toggle_mute(); // still toggles your own mute
        assert!(c.muted && !c.mic_open());
        c.toggle_mute();
        c.set_server(false, false);
        assert!(c.mic_open(), "undone: the mic is back as you left it");
    }

    #[test]
    fn server_deafen_stops_playout_and_mic() {
        let mut c = Controls::default();
        c.set_server(false, true);
        assert!(!c.mic_open() && !c.playout_on());
        assert!(!c.denoise_needed());
        assert!(!c.deafened && !c.muted);
        c.set_server(false, false);
        assert!(c.mic_open() && c.playout_on());
    }

    #[test]
    fn muted_talk_counts_self_mute_only() {
        let mut c = Controls::default();
        assert!(!c.counts_muted_talk(), "open mic");
        c.set_server(true, false);
        assert!(!c.counts_muted_talk(), "server muted, not muted by you");
        c.toggle_mute();
        assert!(!c.counts_muted_talk(), "server muted time never counts");
        c.set_server(false, false);
        assert!(c.counts_muted_talk(), "your own mute");
        c.toggle_mute();
        c.toggle_deafen();
        assert!(c.counts_muted_talk(), "your own deafen implies mute");
        c.set_server(false, true);
        assert!(!c.counts_muted_talk());
    }

    /// #252: the "you're muted" cue sounds for your own mute only; deafened you couldn't hear
    /// it, and a server mute isn't yours to undo.
    #[test]
    fn muted_cue_is_your_own_mute_and_not_deafened() {
        let mut c = Controls::default();
        assert!(!c.cues_muted_talk(), "open mic");
        c.toggle_mute();
        assert!(c.cues_muted_talk(), "your own mute");
        c.toggle_deafen();
        assert!(!c.cues_muted_talk(), "deafened: you wouldn't hear it");
        c.toggle_deafen();
        assert!(c.cues_muted_talk(), "undeafened back to muted");
        c.set_server(true, false);
        assert!(!c.cues_muted_talk(), "server muted");
        c.set_server(false, true);
        assert!(!c.cues_muted_talk(), "server deafened");
        c.set_server(false, false);
        c.toggle_mute();
        assert!(!c.cues_muted_talk(), "unmuted");
    }

    #[test]
    fn server_state_goes_to_the_ui() {
        let mut c = Controls::default();
        c.set_server(true, true);
        let v = serde_json::to_value(c).unwrap();
        assert_eq!(
            v,
            serde_json::json!({"muted": false, "deafened": false, "server_muted": true, "server_deafened": true})
        );
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
