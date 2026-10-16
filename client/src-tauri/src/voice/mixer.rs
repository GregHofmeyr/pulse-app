//! Per-peer receive queues, per-peer gain on a dB curve, soft limiter (FINDINGS: dB slider + soft limit).

use std::collections::{HashMap, VecDeque};

/// Slider percent (0..=200) → linear gain. 100% = 0 dB; above maps linearly in dB up to +12 dB at 200%;
/// below falls to −40 dB at 1% and true silence at 0%. Loudness is logarithmic, so this feels even.
pub fn percent_to_gain(pct: u16) -> f32 {
    let pct = pct.min(200) as f32;
    if pct == 0.0 {
        return 0.0;
    }
    let db = if pct >= 100.0 {
        (pct - 100.0) / 100.0 * 12.0
    } else {
        (pct - 100.0) / 100.0 * 40.0
    };
    10f32.powf(db / 20.0)
}

const KNEE: f32 = 0.8;

/// Identity up to the knee, then a smooth tanh curve towards ±1.0. Boosted voices get louder without
/// the harsh hard-clip distortion the spike had.
pub fn soft_limit(x: f32) -> f32 {
    let a = x.abs();
    if a <= KNEE {
        return x;
    }
    let y = KNEE + (1.0 - KNEE) * ((a - KNEE) / (1.0 - KNEE)).tanh();
    y.min(1.0).copysign(x)
}

/// Each peer's queue is steered so its lowest point (just after the speakers pull) sits here:
/// margin for the sender's 10 ms timer jitter. It then averages about this plus one 10 ms frame
/// (~20 ms) instead of settling wherever start-up left it (anything up to 55 ms before).
pub const TARGET_MS: usize = 10;
/// How often each peer's fill is judged and its correction budget renewed.
const WINDOW_MS: usize = 250;
/// Most samples dropped or repeated per window: 0.5 % of what plays, inaudible on speech.
const MAX_ADJUST_PER_MILLE: usize = 5;
/// Further over the target than this (an output reopen, a stall): cut straight back, crossfaded.
const CUT_ABOVE_MS: usize = 80;
/// A sample this quiet (1 % of full scale, near a zero crossing) is where one is dropped or
/// repeated: the step can't be heard.
const QUIET: u16 = 328;

/// Drop the oldest `n` samples, crossfading what would have played into what plays instead
/// (a hard cut clicks).
fn skip(q: &mut VecDeque<i16>, n: usize) {
    use super::backlog::FADE;
    let n = n.min(q.len());
    // full-length fade even for a small skip (both ends are copied first); stack only: this runs
    // in the audio callback
    let fade = FADE.min(q.len() - n);
    if fade > 0 {
        let (mut old, mut new) = ([0i16; FADE], [0i16; FADE]);
        for k in 0..fade {
            old[k] = q[k];
            new[k] = q[n + k];
        }
        super::backlog::crossfade(&old[..fade], &mut new[..fade]);
        for (k, v) in new[..fade].iter().enumerate() {
            q[n + k] = *v;
        }
    }
    q.drain(..n);
}

pub struct Mixer {
    cap: usize,
    /// Lowest fill wanted (samples), and what a dry peer refills to before playing on.
    target: usize,
    refill: usize,
    window: usize,
    /// Samples a peer may drop or repeat per window.
    budget: usize,
    cut_above: usize,
    /// Samples pulled since the last window was judged.
    pulled: usize,
    peers: HashMap<String, Peer>,
    /// Total samples ever pushed (diagnostics: intake must track real time, see FINDINGS).
    pushed: u64,
    /// Samples a peer didn't have when the speakers asked (diagnostics: gaps / clicks).
    underrun: u64,
    /// Samples dropped or repeated so far (diagnostics: how hard drift is being corrected).
    adjusted: u64,
}

struct Peer {
    queue: VecDeque<i16>,
    /// Shortest the queue has been (right after a pull) this window.
    low: usize,
    /// Drift correction due this window: > 0 samples to drop, < 0 to repeat.
    adjust: i32,
    /// The front sample was just played once and is to be played again (a repeat).
    held: bool,
    /// Ran dry: silent until the queue holds `Mixer::refill` again.
    rebuffering: bool,
    gain: f32,
    /// Level of what we received (drives the speaking ring: lit = audible).
    meter: super::meter::Meter,
}

impl Mixer {
    pub fn new(sample_rate: u32, cap_ms: u32) -> Self {
        Self {
            cap: (sample_rate as usize * cap_ms as usize / 1000).max(1),
            target: sample_rate as usize * TARGET_MS / 1000,
            refill: sample_rate as usize * TARGET_MS / 1000 + sample_rate as usize / 100,
            window: sample_rate as usize * WINDOW_MS / 1000,
            budget: sample_rate as usize * WINDOW_MS / 1000 * MAX_ADJUST_PER_MILLE / 1000,
            cut_above: sample_rate as usize * CUT_ABOVE_MS / 1000,
            pulled: 0,
            peers: HashMap::new(),
            pushed: 0,
            underrun: 0,
            adjusted: 0,
        }
    }

    fn peer(&mut self, id: &str) -> &mut Peer {
        self.peers.entry(id.to_string()).or_insert_with(|| Peer {
            queue: VecDeque::new(),
            low: usize::MAX,
            adjust: 0,
            held: false,
            rebuffering: false,
            gain: 1.0,
            meter: Default::default(),
        })
    }

    /// Queue received samples (mono i16). Beyond the cap, the oldest are dropped (bounded latency).
    pub fn push(&mut self, peer: &str, samples: &[i16]) {
        let cap = self.cap;
        self.pushed += samples.len() as u64;
        let p = self.peer(peer);
        for s in samples {
            p.meter.add(*s as f32 / i16::MAX as f32 * p.gain);
        }
        let q = &mut p.queue;
        q.extend(samples.iter().copied());
        if q.len() > cap {
            let excess = q.len() - cap;
            skip(q, excess);
        }
    }

    pub fn set_gain(&mut self, peer: &str, gain: f32) {
        self.peer(peer).gain = gain;
    }

    pub fn remove(&mut self, peer: &str) {
        self.peers.remove(peer);
    }

    /// (peer, rms since the last call) — what each person sounded like, after their volume.
    pub fn take_peer_levels(&mut self) -> Vec<(String, f32)> {
        self.peers
            .iter_mut()
            .map(|(id, p)| (id.clone(), p.meter.take().0))
            .collect()
    }

    /// Underrun samples since the last call.
    pub fn take_underrun_samples(&mut self) -> u64 {
        std::mem::take(&mut self.underrun)
    }

    pub fn pushed_total(&self) -> u64 {
        self.pushed
    }

    /// Samples queued for the fullest peer.
    pub fn max_buffered(&self) -> usize {
        self.peers
            .values()
            .map(|p| p.queue.len())
            .max()
            .unwrap_or(0)
    }

    pub fn buffered(&self, peer: &str) -> usize {
        self.peers.get(peer).map_or(0, |p| p.queue.len())
    }

    /// Fill interleaved `out` (`channels` per frame), one sample per peer per frame.
    pub fn mix_into(&mut self, out: &mut [f32], channels: usize, master: f32) {
        for p in self.peers.values_mut() {
            if p.rebuffering && p.queue.len() >= self.refill {
                p.rebuffering = false;
            }
        }
        for frame in out.chunks_mut(channels.max(1)) {
            let mut acc = 0.0f32;
            for p in self.peers.values_mut() {
                if p.rebuffering {
                    self.underrun += 1;
                    continue;
                }
                match p.next() {
                    Some((s, adjusted)) => {
                        acc += s as f32 / i16::MAX as f32 * p.gain;
                        self.adjusted += adjusted as u64;
                    }
                    None => {
                        // ran dry: refill before playing on (one gap, not a dribble of clicks)
                        self.underrun += 1;
                        p.rebuffering = true;
                    }
                }
            }
            frame.fill(soft_limit(acc * master));
        }
        for p in self.peers.values_mut() {
            p.low = p.low.min(p.queue.len());
        }
        self.pulled += out.len() / channels.max(1);
        if self.pulled >= self.window {
            self.pulled = 0;
            for p in self.peers.values_mut() {
                p.judge(self.target, self.budget, self.cut_above);
            }
        }
    }

    /// Samples dropped or repeated (drift correction) since the last call.
    pub fn take_adjusted_samples(&mut self) -> u64 {
        std::mem::take(&mut self.adjusted)
    }
}

/// Drift steering: each window the queue's lowest fill is judged against the target, and the
/// difference is corrected one quiet sample at a time while it plays.
impl Peer {
    /// End of a window: set this window's correction from how low the queue got (samples).
    fn judge(&mut self, target: usize, budget: usize, cut_above: usize) {
        if self.low != usize::MAX && !self.rebuffering {
            let over = self.low as i64 - target as i64;
            if over > cut_above as i64 {
                skip(&mut self.queue, over as usize);
                self.adjust = 0;
            } else {
                self.adjust = over.clamp(-(budget as i64), budget as i64) as i32;
            }
        }
        self.low = usize::MAX;
    }

    /// The next sample to play, steering towards the target: while a correction is due, drop or
    /// repeat one quiet sample. `(sample, adjusted)`; `None` = ran dry.
    fn next(&mut self) -> Option<(i16, bool)> {
        let s = *self.queue.front()?;
        let quiet = s.unsigned_abs() < QUIET;
        if self.adjust < 0 && quiet && !self.held {
            self.adjust += 1;
            self.held = true; // play it again next time (once)
            return Some((s, true));
        }
        self.held = false;
        self.queue.pop_front();
        if self.adjust > 0 && quiet && !self.queue.is_empty() {
            self.adjust -= 1;
            self.queue.pop_front(); // and skip the one after it
            return Some((s, true));
        }
        Some((s, false))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Push and pull at the same rate, as a call does once running.
    fn steady(m: &mut Mixer, ms: usize) {
        let mut out = vec![0.0; 480];
        for _ in 0..ms / 10 {
            m.push("a", &[100; 480]);
            m.mix_into(&mut out, 1, 1.0);
        }
    }

    #[test]
    fn a_standing_backlog_drains_to_the_target() {
        // e.g. the output was reopened while audio kept arriving: the queue sits at the cap
        let mut m = Mixer::new(48_000, 200);
        m.push("a", &[100; 9600]);
        steady(&mut m, 2500);
        assert!(
            m.buffered("a") <= TARGET_MS * 48 + 480,
            "{} queued",
            m.buffered("a")
        );
    }

    #[test]
    fn trimming_a_backlog_does_not_click() {
        // a 437 Hz tone (not a whole number of cycles in the cut, so the cut lands out of phase):
        // neighbouring samples differ by at most ~600; a hard cut jumps up to ~20000
        let tone = |i: usize| {
            (10_000.0 * (i as f64 * 437.0 * std::f64::consts::TAU / 48_000.0).sin()) as i16
        };
        let mut m = Mixer::new(48_000, 200);
        let mut next = 0;
        let mut feed = |m: &mut Mixer, n: usize| {
            let chunk: Vec<i16> = (next..next + n).map(tone).collect();
            next += n;
            m.push("a", &chunk);
        };
        feed(&mut m, 9600); // a 200 ms standing backlog
        let mut played = Vec::new();
        let mut out = vec![0.0; 480];
        for _ in 0..250 {
            feed(&mut m, 480);
            m.mix_into(&mut out, 1, 1.0);
            played.extend(out.iter().map(|v| (v * i16::MAX as f32) as i32));
        }
        assert!(m.buffered("a") < 9600, "the backlog was trimmed");
        let step = played
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .max()
            .unwrap();
        assert!(step < 1500, "a click: step of {step}");
    }

    /// The speakers asked for audio a peer didn't have (yet): heard as a gap or a click.
    #[test]
    fn underruns_are_counted_per_peer_sample_missed() {
        let mut m = Mixer::new(48_000, 200);
        m.push("a", &[1; 240]);
        let mut out = vec![0.0; 480];
        m.mix_into(&mut out, 1, 1.0);
        assert_eq!(m.take_underrun_samples(), 240);
        assert_eq!(m.take_underrun_samples(), 0, "restarts after a take");
    }

    #[test]
    fn a_queue_at_the_target_is_left_alone() {
        let mut m = Mixer::new(48_000, 200);
        m.push("a", &[100; 480]); // 10 ms: at the target, nothing to drop or repeat
        steady(&mut m, 3000);
        assert_eq!(m.buffered("a"), 480);
    }

    #[test]
    fn gain_curve_anchors_and_monotonic() {
        assert_eq!(percent_to_gain(0), 0.0);
        assert!((percent_to_gain(100) - 1.0).abs() < 1e-6);
        assert!((percent_to_gain(200) - 3.981).abs() < 0.01);
        let mut prev = -1.0;
        for p in 0..=200 {
            let g = percent_to_gain(p);
            assert!(g > prev, "not increasing at {p}");
            prev = g;
        }
    }

    #[test]
    fn soft_limit_is_identity_below_knee_and_bounded() {
        for x in [-0.8f32, -0.5, 0.0, 0.3, 0.8] {
            assert_eq!(soft_limit(x), x);
        }
        let mut prev = -2.0;
        for i in -1000..=1000 {
            let x = i as f32 / 100.0;
            let y = soft_limit(x);
            assert!(y.abs() <= 1.0, "{x} -> {y}");
            assert!(y >= prev);
            prev = y;
        }
    }

    #[test]
    fn mixes_two_peers_with_per_peer_gain() {
        let mut m = Mixer::new(48_000, 200);
        m.push("a", &[8192; 4]); // 0.25
        m.push("b", &[8192; 4]);
        m.set_gain("b", 0.0);
        let mut out = [0.0f32; 8];
        m.mix_into(&mut out, 2, 1.0);
        for v in out {
            assert!((v - 0.25).abs() < 1e-3, "{v}");
        }
    }

    #[test]
    fn empty_peer_contributes_silence() {
        let mut m = Mixer::new(48_000, 200);
        m.push("a", &[16384; 2]);
        let mut out = [1.0f32; 4];
        m.mix_into(&mut out, 1, 1.0);
        assert!((out[0] - 0.5).abs() < 1e-3);
        assert_eq!(&out[2..], &[0.0, 0.0]);
    }

    #[test]
    fn cap_drops_oldest() {
        let mut m = Mixer::new(1000, 10); // cap = 10 samples
        let samples: Vec<i16> = (0..25).collect();
        m.push("a", &samples);
        assert_eq!(m.buffered("a"), 10);
        let mut out = [0.0f32; 10];
        m.mix_into(&mut out, 1, 1.0);
        // the oldest 15 were dropped: playback fades from where they were (0) into the newest
        // (10 samples here, the whole queue: 0.1 x 9 + 0.9 x 24 = 22.5 at the end)
        let at = |i: usize| out[i] * i16::MAX as f32;
        assert!(at(0).abs() < 0.5, "{}", at(0));
        assert!((at(9) - 22.5).abs() < 1.0, "{}", at(9));
    }

    #[test]
    fn remove_peer_drops_queue() {
        let mut m = Mixer::new(48_000, 200);
        m.push("a", &[1; 10]);
        m.remove("a");
        assert_eq!(m.buffered("a"), 0);
    }

    #[test]
    fn per_peer_levels_reflect_pushed_audio() {
        let mut m = Mixer::new(48_000, 200);
        m.push("loud", &[16384; 480]);
        m.push("quiet", &[0; 480]);
        let mut levels = m.take_peer_levels();
        levels.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(levels[0].0, "loud");
        assert!((levels[0].1 - 0.5).abs() < 0.01);
        assert_eq!(levels[1].1, 0.0);
        assert!(
            m.take_peer_levels().iter().all(|(_, v)| *v == 0.0),
            "reset after take"
        );
    }

    use std::f64::consts::TAU;

    fn tone(i: usize) -> i16 {
        (10_000.0 * (i as f64 * 437.0 * TAU / 48_000.0).sin()) as i16
    }

    /// One sender, one speaker: per 10 ms pulled, the sender delivers 480 × (1 + ppm/10⁶) samples
    /// (fraction carried). Returns what played, as i16-scaled values.
    fn drifting(m: &mut Mixer, secs: usize, ppm: f64, next: &mut usize) -> Vec<i32> {
        let (mut owed, mut out, mut played) = (0.0f64, vec![0.0f32; 480], Vec::new());
        for _ in 0..secs * 100 {
            owed += 480.0 * (1.0 + ppm / 1e6);
            let n = owed as usize;
            owed -= n as f64;
            let chunk: Vec<i16> = (*next..*next + n).map(tone).collect();
            *next += n;
            m.push("a", &chunk);
            m.mix_into(&mut out, 1, 1.0);
            played.extend(out.iter().map(|v| (v * i16::MAX as f32) as i32));
        }
        played
    }

    fn max_step(p: &[i32]) -> i32 {
        p.windows(2).map(|w| (w[1] - w[0]).abs()).max().unwrap_or(0)
    }

    /// The sender's clock runs fast (10× a bad real-world drift, so a minute shows it): the
    /// queue must stay near the target, eased down without clicks, never trimmed in big cuts.
    #[test]
    fn a_fast_sender_is_held_near_the_target() {
        let mut m = Mixer::new(48_000, 200);
        let mut next = 0;
        let played = drifting(&mut m, 60, 1000.0, &mut next);
        assert!(
            m.buffered("a") <= (TARGET_MS + 12) * 48,
            "queue grew to {} samples",
            m.buffered("a")
        );
        assert_eq!(m.take_underrun_samples(), 0);
        assert!(
            max_step(&played) < 1500,
            "a click: step of {}",
            max_step(&played)
        );
        assert!(
            m.take_adjusted_samples() > 0,
            "drift was corrected sample by sample"
        );
    }

    /// The sender's clock runs slow: once settled, the speakers must never run dry (today they
    /// do, a few samples at a time, forever).
    #[test]
    fn a_slow_sender_never_underruns_once_settled() {
        let mut m = Mixer::new(48_000, 200);
        let mut next = 0;
        drifting(&mut m, 10, -1000.0, &mut next);
        m.take_underrun_samples();
        let played = drifting(&mut m, 50, -1000.0, &mut next);
        assert_eq!(m.take_underrun_samples(), 0, "ran dry while settled");
        assert!(
            max_step(&played) < 1500,
            "a click: step of {}",
            max_step(&played)
        );
    }

    /// After running dry (the sender stalled), playback resumes with a cushion, not at zero
    /// margin where the next late frame is another gap.
    #[test]
    fn after_an_underrun_playback_resumes_with_a_cushion() {
        let mut m = Mixer::new(48_000, 200);
        let mut next = 0;
        drifting(&mut m, 1, 0.0, &mut next);
        let mut out = vec![0.0f32; 480];
        for _ in 0..5 {
            m.mix_into(&mut out, 1, 1.0); // 50 ms with nothing arriving
        }
        assert!(m.take_underrun_samples() > 0);
        drifting(&mut m, 1, 0.0, &mut next);
        assert!(
            m.buffered("a") >= TARGET_MS * 48,
            "resumed with {} samples of margin",
            m.buffered("a")
        );
    }

    /// A queue a little over the target (start-up) is eased down, not cut.
    #[test]
    fn a_small_excess_is_eased_out_without_a_cut() {
        let mut m = Mixer::new(48_000, 200);
        let pre: Vec<i16> = (0..48 * 45).map(tone).collect(); // 45 ms
        m.push("a", &pre);
        let mut next = pre.len();
        let played = drifting(&mut m, 3, 0.0, &mut next);
        let left = m.buffered("a");
        assert!(left < 48 * 45 && left > TARGET_MS * 48, "{left} queued");
        assert!(
            max_step(&played) < 1500,
            "a click: step of {}",
            max_step(&played)
        );
    }

    /// Big device callbacks (Linux ALSA hands out 2048 frames at a time, ~43 ms): the first pull
    /// runs dry once, the refill then holds, and it never runs dry again (no adaptive target
    /// needed). The sender's frames and the speakers' pulls interleave in real time here.
    #[test]
    fn big_device_callbacks_underrun_once_then_hold() {
        for ppm in [-100.0, 0.0, 100.0] {
            let mut m = Mixer::new(48_000, 200);
            let mut out = vec![0.0f32; 2048];
            let (mut next_push, mut next_pull) = (0.0f64, 0);
            let mut i = 0;
            for t in 0..48_000 * 30 {
                while next_push <= t as f64 {
                    let chunk: Vec<i16> = (i..i + 480).map(tone).collect();
                    i += 480;
                    m.push("a", &chunk);
                    next_push += 480.0 * (1.0 + ppm / 1e6);
                }
                if t == next_pull {
                    m.mix_into(&mut out, 1, 1.0);
                    next_pull += out.len();
                }
                if t == 48_000 {
                    assert!(m.take_underrun_samples() > 0, "the first pull ran dry");
                }
            }
            assert_eq!(m.take_underrun_samples(), 0, "ran dry again at {ppm} ppm");
            assert!(m.buffered("a") <= 4096, "{} queued", m.buffered("a"));
        }
    }
}
