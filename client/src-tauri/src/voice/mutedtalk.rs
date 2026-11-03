//! "You're muted": talking into a muted mic for a while (The Doos badge). Judged on the
//! input's RMS after WebRTC APM (echo cancelled, so the call coming out of the speakers isn't
//! you) and before the denoiser. Loud blocks build up talk time and quiet ones drain it
//! slowly, so the pauses between words don't reset it; unmuting does. It fires once, then rests.

use std::time::{Duration, Instant};

/// RMS at or above this (≈ −34 dBFS) is talking, not a quiet room.
pub const TALK_RMS: f32 = 0.02;
/// Built-up talk time that fires it.
pub const TALK_FOR: Duration = Duration::from_millis(1500);
/// After firing, this long before it can fire again.
pub const COOLDOWN: Duration = Duration::from_secs(60);
/// What one 10 ms block adds while loud, and takes away while quiet.
const LOUD_STEP: Duration = Duration::from_millis(10);
const QUIET_STEP: Duration = Duration::from_millis(5);

#[derive(Debug, Default)]
pub struct MutedTalk {
    talked: Duration,
    last_fired: Option<Instant>,
    fired: bool,
}

impl MutedTalk {
    /// One echo-cancelled 10 ms block: its RMS, and whether you're muted (deafened implies it).
    pub fn block(&mut self, rms: f32, muted: bool, now: Instant) {
        if !muted {
            self.talked = Duration::ZERO;
            return;
        }
        if rms >= TALK_RMS {
            self.talked += LOUD_STEP;
        } else {
            self.talked = self.talked.saturating_sub(QUIET_STEP);
        }
        if self.talked < TALK_FOR {
            return;
        }
        if self
            .last_fired
            .is_some_and(|at| now.duration_since(at) < COOLDOWN)
        {
            return;
        }
        self.talked = Duration::ZERO;
        self.last_fired = Some(now);
        self.fired = true;
    }

    /// Whether it fired since the last take.
    pub fn take_fired(&mut self) -> bool {
        std::mem::take(&mut self.fired)
    }
}

/// The root-mean-square level of a block.
pub fn rms(block: &[f32]) -> f32 {
    if block.is_empty() {
        return 0.0;
    }
    (block.iter().map(|s| s * s).sum::<f32>() / block.len() as f32).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TALK: f32 = 0.1;
    const QUIET: f32 = 0.001;

    /// `ms` of 10 ms blocks at `rms` from `t`; returns the time after.
    fn feed(m: &mut MutedTalk, t: Instant, ms: u64, rms: f32, muted: bool) -> Instant {
        let mut now = t;
        for _ in 0..ms / 10 {
            m.block(rms, muted, now);
            now += Duration::from_millis(10);
        }
        now
    }

    #[test]
    fn talking_while_muted_fires_once() {
        let mut m = MutedTalk::default();
        let t = feed(&mut m, Instant::now(), 1400, TALK, true);
        assert!(!m.take_fired(), "not before TALK_FOR");
        let t = feed(&mut m, t, 200, TALK, true);
        assert!(m.take_fired());
        assert!(!m.take_fired(), "taken once");
        feed(&mut m, t, 10_000, TALK, true);
        assert!(!m.take_fired(), "resting");
    }

    #[test]
    fn unmuted_never_fires() {
        let mut m = MutedTalk::default();
        feed(&mut m, Instant::now(), 10_000, TALK, false);
        assert!(!m.take_fired());
    }

    #[test]
    fn short_blips_dont_fire() {
        let mut m = MutedTalk::default();
        let mut t = Instant::now();
        // 100 ms of sound then 300 ms of quiet, for a minute: the quiet drains it
        for _ in 0..150 {
            t = feed(&mut m, t, 100, TALK, true);
            t = feed(&mut m, t, 300, QUIET, true);
        }
        assert!(!m.take_fired());
        // and unmuting in the middle of talking starts it again
        let t = feed(&mut m, t, 1000, TALK, true);
        let t = feed(&mut m, t, 10, TALK, false);
        feed(&mut m, t, 1000, TALK, true);
        assert!(!m.take_fired());
    }

    #[test]
    fn pauses_between_words_still_build_up() {
        let mut m = MutedTalk::default();
        let mut t = Instant::now();
        // 300 ms of words, 100 ms of gap: +30 −5 per round
        for _ in 0..6 {
            t = feed(&mut m, t, 300, TALK, true);
            t = feed(&mut m, t, 100, QUIET, true);
        }
        assert!(m.take_fired());
    }

    #[test]
    fn cooldown_then_again() {
        let mut m = MutedTalk::default();
        let t = feed(&mut m, Instant::now(), 1500, TALK, true);
        assert!(m.take_fired());
        let t = feed(&mut m, t, 30_000, QUIET, true);
        let t = feed(&mut m, t, 1500, TALK, true);
        assert!(!m.take_fired(), "within the cooldown");
        let t = feed(&mut m, t, 30_000, QUIET, true);
        feed(&mut m, t, 1500, TALK, true);
        assert!(m.take_fired(), "after the cooldown");
    }

    #[test]
    fn rms_of_a_block() {
        assert_eq!(rms(&[]), 0.0);
        assert!((rms(&[0.5, -0.5]) - 0.5).abs() < 1e-6);
    }
}
