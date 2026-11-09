//! "You're muted": talking into a muted mic for a while (The Doos badge). Judged on the
//! input's RMS after WebRTC APM (echo cancelled, so the call coming out of the speakers isn't
//! you) and before the denoiser. Loud blocks build up talk time and quiet ones drain it
//! slowly, so the pauses between words don't reset it; unmuting does. It fires once, then rests.
//!
//! [`MutedCue`] is the same judgement for the "you're muted" sound (#252), on its own clock: the
//! badge is rate-limited by design, while the reminder repeats every [`CUE_COOLDOWN`] while you
//! keep talking, and a fresh mute starts it over.

use std::time::{Duration, Instant};

/// RMS at or above this (≈ −34 dBFS) is talking, not a quiet room.
pub const TALK_RMS: f32 = 0.02;
/// Built-up talk time that fires it.
pub const TALK_FOR: Duration = Duration::from_millis(1500);
/// After firing, this long before it can fire again.
pub const COOLDOWN: Duration = Duration::from_secs(60);
/// While you keep talking muted, the cue sounds again after this long: a reminder, not a nag.
pub const CUE_COOLDOWN: Duration = Duration::from_secs(12);
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
        self.talked = build_up(self.talked, rms);
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

/// Talk time after one block at `rms`: loud adds, quiet drains slowly.
fn build_up(talked: Duration, rms: f32) -> Duration {
    if rms >= TALK_RMS {
        talked + LOUD_STEP
    } else {
        talked.saturating_sub(QUIET_STEP)
    }
}

/// The "you're muted" cue (#252): fires after [`TALK_FOR`] of talk like [`MutedTalk`], then again
/// every [`CUE_COOLDOWN`] while you keep at it. Unmuting (or deafening) re-arms it at once.
#[derive(Debug, Default)]
pub struct MutedCue {
    talked: Duration,
    last_fired: Option<Instant>,
    fired: bool,
}

impl MutedCue {
    /// One echo-cancelled 10 ms block: its RMS, and whether the cue applies (`cues_muted_talk`).
    pub fn block(&mut self, rms: f32, on: bool, now: Instant) {
        if !on {
            self.talked = Duration::ZERO;
            self.last_fired = None;
            return;
        }
        // capped: a long talk through the cooldown doesn't bank up an instant cue after it
        self.talked = build_up(self.talked, rms).min(TALK_FOR);
        if self.talked < TALK_FOR
            || self
                .last_fired
                .is_some_and(|at| now.duration_since(at) < CUE_COOLDOWN)
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

    /// `ms` of 10 ms blocks into the cue at `rms`; returns the time after and how often it fired.
    fn cue(c: &mut MutedCue, t: Instant, ms: u64, rms: f32, on: bool) -> (Instant, u32) {
        let (mut now, mut n) = (t, 0);
        for _ in 0..ms / 10 {
            c.block(rms, on, now);
            n += c.take_fired() as u32;
            now += Duration::from_millis(10);
        }
        (now, n)
    }

    #[test]
    fn the_cue_builds_up_like_the_badge() {
        let mut c = MutedCue::default();
        let (t, n) = cue(&mut c, Instant::now(), 1400, TALK, true);
        assert_eq!(n, 0, "not before TALK_FOR");
        let (_, n) = cue(&mut c, t, 200, TALK, true);
        assert_eq!(n, 1);
        let mut c = MutedCue::default();
        let (_, n) = cue(&mut c, Instant::now(), 10_000, TALK, false);
        assert_eq!(n, 0, "not muted");
        // 100 ms of a cough then 300 ms of quiet, for a minute: drained, never a cue
        let (mut t, mut n) = (Instant::now(), 0);
        for _ in 0..150 {
            let (a, k) = cue(&mut c, t, 100, TALK, true);
            let (b, j) = cue(&mut c, a, 300, QUIET, true);
            (t, n) = (b, n + k + j);
        }
        assert_eq!(n, 0, "short blips");
    }

    #[test]
    fn the_cue_repeats_while_you_keep_talking_muted() {
        let mut c = MutedCue::default();
        // a minute of talking muted: the first after TALK_FOR, then one per CUE_COOLDOWN
        let (_, n) = cue(&mut c, Instant::now(), 60_000, TALK, true);
        let every = CUE_COOLDOWN.as_secs_f32();
        let want = 1 + ((60.0 - TALK_FOR.as_secs_f32()) / every) as u32;
        assert_eq!(n, want);
        assert!(
            CUE_COOLDOWN < COOLDOWN,
            "more often than the badge's detection"
        );
        assert!(CUE_COOLDOWN >= Duration::from_secs(10), "not a nag");
    }

    #[test]
    fn the_cue_waits_out_its_cooldown_then_needs_fresh_talk() {
        let mut c = MutedCue::default();
        let (t, n) = cue(&mut c, Instant::now(), 1500, TALK, true);
        assert_eq!(n, 1);
        // talking on through most of the cooldown, then 3 s of quiet until past it: the long
        // talk doesn't bank up, so one word just after isn't an instant cue
        let talk_on = CUE_COOLDOWN.as_millis() as u64 - 3_000;
        let (t, n) = cue(&mut c, t, talk_on, TALK, true);
        assert_eq!(n, 0, "within the cooldown");
        let (t, n) = cue(&mut c, t, 3_100, QUIET, true);
        assert_eq!(n, 0, "past the cooldown now, but quiet");
        let (t, n) = cue(&mut c, t, 200, TALK, true);
        assert_eq!(n, 0, "a fresh build-up is needed");
        let (_, n) = cue(&mut c, t, 1400, TALK, true);
        assert_eq!(n, 1);
    }

    #[test]
    fn unmuting_rearms_the_cue() {
        let mut c = MutedCue::default();
        let (t, n) = cue(&mut c, Instant::now(), 1500, TALK, true);
        assert_eq!(n, 1);
        // unmute, re-mute two seconds later and talk again: well inside the cooldown, it sounds
        let (t, _) = cue(&mut c, t, 500, TALK, false);
        let (_, n) = cue(&mut c, t, 1500, TALK, true);
        assert_eq!(n, 1, "a new mute is a new reminder");
    }

    #[test]
    fn the_badge_and_the_cue_count_apart() {
        // the badge rests 60 s after firing; the cue comes back sooner, on its own
        let (mut m, mut c) = (MutedTalk::default(), MutedCue::default());
        let mut now = Instant::now();
        let (mut badges, mut cues) = (0, 0);
        for _ in 0..3000 {
            m.block(TALK, true, now);
            c.block(TALK, true, now);
            badges += m.take_fired() as u32;
            cues += c.take_fired() as u32;
            now += Duration::from_millis(10);
        }
        assert_eq!(badges, 1, "30 s: the badge once");
        assert!(cues >= 2, "30 s: the cue {cues} times");
    }

    #[test]
    fn rms_of_a_block() {
        assert_eq!(rms(&[]), 0.0);
        assert!((rms(&[0.5, -0.5]) - 0.5).abs() < 1e-6);
    }
}
