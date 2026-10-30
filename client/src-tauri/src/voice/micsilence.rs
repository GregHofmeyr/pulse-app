//! "No audio from your mic" during a call: the mic delivers pure digital silence (Windows'
//! microphone privacy switch, a hardware mute switch, a dead or wrong device) while you're unmuted.
//! Judged on the raw input, before any processing. A real mic is never this quiet, but a gated
//! virtual mic (Sonar, Voicemeeter…) is between words, so once the mic has been heard in this call
//! it isn't warned about again until the call or the device changes.

use std::time::{Duration, Instant};

/// Peak at or below this (two 16-bit steps) is digital silence, not a quiet room.
pub const DIGITAL_SILENCE: f32 = 2.0 / 32768.0;
/// Unmuted and silent this long (and never heard) before the warning shows.
pub const WARN_AFTER: Duration = Duration::from_secs(10);

#[derive(Debug, Default)]
pub struct MicSilence {
    /// Real sound came through since the last restart.
    heard: bool,
    silent_since: Option<Instant>,
    warning: bool,
    changed: bool,
}

impl MicSilence {
    /// One raw 10 ms mic block: its peak, and whether the mic is open (unmuted).
    pub fn block(&mut self, peak: f32, mic_open: bool, now: Instant) {
        if !mic_open {
            // muted: nothing to warn about; the wait starts again on unmute
            self.silent_since = None;
            self.set(false);
            return;
        }
        if peak > DIGITAL_SILENCE {
            self.heard = true;
            self.silent_since = None;
            self.set(false);
            return;
        }
        if self.heard {
            return;
        }
        let since = *self.silent_since.get_or_insert(now);
        if now.duration_since(since) >= WARN_AFTER {
            self.set(true);
        }
    }

    /// A new call or a newly opened mic: judge it from scratch.
    pub fn restart(&mut self) {
        self.heard = false;
        self.silent_since = None;
        self.set(false);
    }

    /// The warning, if it changed since the last take.
    pub fn take_change(&mut self) -> Option<bool> {
        std::mem::take(&mut self.changed).then_some(self.warning)
    }

    fn set(&mut self, warning: bool) {
        if warning != self.warning {
            self.warning = warning;
            self.changed = true;
        }
    }
}

/// The loudest sample in a block.
pub fn peak(block: &[f32]) -> f32 {
    block.iter().fold(0.0, |m, s| m.max(s.abs()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const QUIET_ROOM: f32 = 0.0005; // ≈ −66 dBFS: a real mic with nobody talking
    const ZERO: f32 = 0.0;

    /// `secs` of 10 ms blocks at `peak` from `t`; returns the time after.
    fn feed(m: &mut MicSilence, t: Instant, secs: u64, peak: f32, open: bool) -> Instant {
        let mut now = t;
        for _ in 0..secs * 100 {
            m.block(peak, open, now);
            now += Duration::from_millis(10);
        }
        now
    }

    #[test]
    fn an_unmuted_mic_sending_only_zeros_is_warned_about() {
        let mut m = MicSilence::default();
        let t = feed(&mut m, Instant::now(), 9, ZERO, true);
        assert_eq!(m.take_change(), None, "not before WARN_AFTER");
        feed(&mut m, t, 2, ZERO, true);
        assert_eq!(m.take_change(), Some(true));
        assert_eq!(m.take_change(), None, "reported once");
    }

    #[test]
    fn a_quiet_room_is_not_silence() {
        let mut m = MicSilence::default();
        feed(&mut m, Instant::now(), 30, QUIET_ROOM, true);
        assert_eq!(m.take_change(), None);
    }

    #[test]
    fn a_muted_mic_is_never_warned_about() {
        let mut m = MicSilence::default();
        feed(&mut m, Instant::now(), 30, ZERO, false);
        assert_eq!(m.take_change(), None);
    }

    #[test]
    fn muting_clears_the_warning_and_unmuting_waits_again() {
        let mut m = MicSilence::default();
        let t = feed(&mut m, Instant::now(), 11, ZERO, true);
        assert_eq!(m.take_change(), Some(true));
        let t = feed(&mut m, t, 1, ZERO, false);
        assert_eq!(m.take_change(), Some(false));
        let t = feed(&mut m, t, 9, ZERO, true);
        assert_eq!(m.take_change(), None, "a fresh wait after unmuting");
        feed(&mut m, t, 2, ZERO, true);
        assert_eq!(m.take_change(), Some(true));
    }

    #[test]
    fn sound_clears_the_warning() {
        let mut m = MicSilence::default();
        let t = feed(&mut m, Instant::now(), 11, ZERO, true);
        assert_eq!(m.take_change(), Some(true));
        m.block(0.3, true, t);
        assert_eq!(m.take_change(), Some(false));
    }

    #[test]
    fn a_gated_mic_heard_once_isnt_warned_about_between_words() {
        let mut m = MicSilence::default();
        let t = feed(&mut m, Instant::now(), 1, 0.3, true);
        feed(&mut m, t, 60, ZERO, true);
        assert_eq!(m.take_change(), None);
    }

    #[test]
    fn a_new_call_or_mic_is_judged_from_scratch() {
        let mut m = MicSilence::default();
        let t = feed(&mut m, Instant::now(), 1, 0.3, true);
        m.restart();
        feed(&mut m, t, 11, ZERO, true);
        assert_eq!(m.take_change(), Some(true));
    }

    #[test]
    fn restarting_clears_a_shown_warning() {
        let mut m = MicSilence::default();
        feed(&mut m, Instant::now(), 11, ZERO, true);
        assert_eq!(m.take_change(), Some(true));
        m.restart();
        assert_eq!(m.take_change(), Some(false));
    }

    #[test]
    fn peak_is_the_loudest_sample_either_way() {
        assert_eq!(peak(&[0.1, -0.4, 0.2]), 0.4);
        assert_eq!(peak(&[]), 0.0);
    }
}
