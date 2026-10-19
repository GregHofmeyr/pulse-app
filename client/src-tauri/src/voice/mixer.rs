//! Per-peer receive queues, per-peer gain on a dB curve, soft limiter (FINDINGS: dB slider + soft limit).

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering::Relaxed};
use std::sync::{Arc, Mutex};

use super::meter::Meter;

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
/// A lowest fill up to this far over the target is left alone (never dropped from): each window's
/// low swings by up to one 10 ms push frame (sender timer jitter, big callbacks beating against
/// the frames), and chasing that would drop and repeat back and forth with no drift at all.
/// Below the target is still repeated straight away (the underrun side).
const DEADBAND_MS: usize = 10;
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

/// What a received track is: someone's voice, or the audio of a stream they're sharing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SourceKind {
    Voice,
    Stream,
}

/// Network side → speaker side changes, applied by the speakers at their next pull.
enum Cmd {
    Add {
        id: String,
        ring: rtrb::Consumer<i16>,
        filled: Arc<AtomicUsize>,
        gain: f32,
    },
    Gain {
        id: String,
        gain: f32,
    },
    Remove {
        id: String,
    },
    Clear,
}

/// A mixer in two halves that share no lock the speakers wait on: `MixerInput` for the
/// receive tasks and UI, `Mixer` for the speaker callback only.
pub fn mixer(sample_rate: u32, cap_ms: u32) -> (MixerInput, Mixer) {
    let rate = sample_rate as usize;
    let cap = (rate * cap_ms as usize / 1000).max(1);
    let cmds = Arc::new(Mutex::new(Vec::new()));
    let (underrun, adjusted) = (Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0)));
    let input = MixerInput {
        cmds: cmds.clone(),
        sources: Mutex::new(HashMap::new()),
        gains: Mutex::new(HashMap::new()),
        generation: AtomicU64::new(0),
        cap,
        pushed: AtomicU64::new(0),
        underrun: underrun.clone(),
        adjusted: adjusted.clone(),
    };
    let speakers = Mixer {
        cmds,
        cap,
        peers: HashMap::with_capacity(32), // no growth in the callback for a normal call
        target: rate * TARGET_MS / 1000,
        min_pull: rate / 100,
        pulls: [0; 2],
        window: rate * WINDOW_MS / 1000,
        budget: rate * WINDOW_MS / 1000 * MAX_ADJUST_PER_MILLE / 1000,
        deadband: rate * DEADBAND_MS / 1000,
        cut_above: rate * CUT_ABOVE_MS / 1000,
        pulled: 0,
        underrun,
        adjusted,
    };
    (input, speakers)
}

/// The receive side: one lock-free ring per source into the speakers, plus what the UI and the
/// stats line read. Its locks are only ever taken by network-side tasks.
pub struct MixerInput {
    cmds: Arc<Mutex<Vec<Cmd>>>,
    /// Keyed by track SID: a person can send voice and stream audio at once.
    pub(crate) sources: Mutex<HashMap<String, SourceIn>>,
    /// Volume per (person, kind), kept with no track and across `clear`, so a slider set early applies.
    gains: Mutex<HashMap<(String, SourceKind), f32>>,
    /// Bumped by `clear` (under the sources lock): pushes for an older room are dropped.
    generation: AtomicU64,
    cap: usize,
    /// Total samples ever pushed (diagnostics: intake must track real time, see FINDINGS).
    pushed: AtomicU64,
    /// Samples a peer didn't have when the speakers asked (diagnostics: gaps / clicks).
    underrun: Arc<AtomicU64>,
    /// Samples dropped or repeated so far (diagnostics: how hard drift is being corrected).
    adjusted: Arc<AtomicU64>,
}

pub(crate) struct SourceIn {
    /// Whose track this is (a person's identity).
    owner: String,
    kind: SourceKind,
    ring: rtrb::Producer<i16>,
    gain: f32,
    /// Level of what we received (drives the speaking ring: lit = audible).
    meter: Meter,
    /// Queued for this source as of the speakers' last pull.
    filled: Arc<AtomicUsize>,
}

impl MixerInput {
    /// Which room's sources the mixer holds now: bumped by every `clear`.
    pub fn generation(&self) -> u64 {
        self.generation.load(Relaxed)
    }

    /// Start playing a track (before its receive task starts: earlier pushes are dropped).
    /// Re-adding a SID starts it empty. One for an older room is ignored, as in `push_in`.
    pub fn add_source(&self, generation: u64, source: &str, owner: &str, kind: SourceKind) {
        let mut sources = self.sources.lock().unwrap();
        if generation != self.generation.load(Relaxed) {
            return;
        }
        let gain = self
            .gains
            .lock()
            .unwrap()
            .get(&(owner.to_string(), kind))
            .copied()
            .unwrap_or(1.0);
        // twice the cap: a burst while the speakers reopen is trimmed on their side
        let (tx, rx) = rtrb::RingBuffer::new(2 * self.cap);
        let filled = Arc::new(AtomicUsize::new(0));
        self.cmds.lock().unwrap().push(Cmd::Add {
            id: source.to_string(),
            ring: rx,
            filled: filled.clone(),
            gain,
        });
        sources.insert(
            source.to_string(),
            SourceIn {
                owner: owner.to_string(),
                kind,
                ring: tx,
                gain,
                meter: Meter::default(),
                filled,
            },
        );
    }

    /// Queue received samples (mono i16) for the current room.
    pub fn push(&self, source: &str, samples: &[i16]) {
        self.push_in(self.generation(), source, samples);
    }

    /// Queue received samples (mono i16) from a receive task of room `generation`. One of an
    /// older room (still mid-push when a switch cleared the mixer) is dropped, and so is one
    /// for a source that isn't registered: it would bring back a source nobody feeds any more.
    pub fn push_in(&self, generation: u64, source: &str, samples: &[i16]) {
        let mut sources = self.sources.lock().unwrap();
        // under the sources lock, as `clear` bumps it: no push lands after a clear it predates
        if generation != self.generation.load(Relaxed) {
            return;
        }
        let Some(p) = sources.get_mut(source) else {
            return;
        };
        self.pushed.fetch_add(samples.len() as u64, Relaxed);
        for s in samples {
            p.meter.add(*s as f32 / i16::MAX as f32 * p.gain);
        }
        // a full ring means the speakers aren't pulling (a reopen): keep the newest
        let newest = &samples[samples.len().saturating_sub(p.ring.slots())..];
        if let Ok(chunk) = p.ring.write_chunk_uninit(newest.len()) {
            chunk.fill_from_iter(newest.iter().copied());
        }
    }

    pub fn set_gain(&self, owner: &str, kind: SourceKind, gain: f32) {
        let mut sources = self.sources.lock().unwrap();
        self.gains
            .lock()
            .unwrap()
            .insert((owner.to_string(), kind), gain);
        // still under the sources lock: commands reach the speakers in the order sources changed
        let mut cmds = self.cmds.lock().unwrap();
        for (id, s) in sources.iter_mut() {
            if s.owner == owner && s.kind == kind {
                s.gain = gain;
                cmds.push(Cmd::Gain {
                    id: id.clone(),
                    gain,
                });
            }
        }
    }

    /// Stop playing one track. One for an older room is ignored: the same SID may be back.
    pub fn remove_source(&self, generation: u64, source: &str) {
        let mut sources = self.sources.lock().unwrap();
        if generation != self.generation.load(Relaxed) {
            return;
        }
        if sources.remove(source).is_some() {
            self.cmds.lock().unwrap().push(Cmd::Remove {
                id: source.to_string(),
            });
        }
    }

    /// Stop playing everything a person sends (they left). Ignored for an older room.
    pub fn remove_owner(&self, generation: u64, owner: &str) {
        let mut sources = self.sources.lock().unwrap();
        if generation != self.generation.load(Relaxed) {
            return;
        }
        let mut cmds = self.cmds.lock().unwrap();
        sources.retain(|id, s| {
            let keep = s.owner != owner;
            if !keep {
                cmds.push(Cmd::Remove { id: id.clone() });
            }
            keep
        });
    }

    /// Forget every source (a channel switch keeps the mixer, not the last room's people).
    /// Volumes stay: they belong to the person, not the track.
    pub fn clear(&self) {
        let mut sources = self.sources.lock().unwrap();
        self.generation.fetch_add(1, Relaxed);
        sources.clear();
        self.cmds.lock().unwrap().push(Cmd::Clear);
    }

    /// (person, rms since the last call): how each person's *voice* sounded, after their volume.
    /// Stream meters are drained too but never reported: a loud game isn't talking.
    pub fn take_peer_levels(&self) -> Vec<(String, f32)> {
        let mut out: Vec<(String, f32)> = Vec::new();
        for s in self.sources.lock().unwrap().values_mut() {
            let rms = s.meter.take().0;
            if s.kind != SourceKind::Voice {
                continue;
            }
            match out.iter_mut().find(|(o, _)| *o == s.owner) {
                Some((_, v)) => *v = v.max(rms),
                None => out.push((s.owner.clone(), rms)),
            }
        }
        out
    }

    /// Underrun samples since the last call.
    pub fn take_underrun_samples(&self) -> u64 {
        self.underrun.swap(0, Relaxed)
    }

    /// Samples dropped or repeated (drift correction) since the last call.
    pub fn take_adjusted_samples(&self) -> u64 {
        self.adjusted.swap(0, Relaxed)
    }

    pub fn pushed_total(&self) -> u64 {
        self.pushed.load(Relaxed)
    }

    /// Samples queued for the fullest source, as of the speakers' last pull.
    pub fn max_buffered(&self) -> usize {
        let sources = self.sources.lock().unwrap();
        sources
            .values()
            .map(|p| p.filled.load(Relaxed))
            .max()
            .unwrap_or(0)
    }
}

/// The speaker side: owned by the output callback.
pub struct Mixer {
    cmds: Arc<Mutex<Vec<Cmd>>>,
    cap: usize,
    peers: HashMap<String, Peer>,
    /// Lowest fill wanted (samples). A dry peer refills to this plus one pull before playing on
    /// (`refill`): a big device pull must not run it dry again half-way (two gaps for one).
    target: usize,
    /// The least pull `refill` assumes: one 10 ms frame.
    min_pull: usize,
    /// Largest pull (frames) this window and the last: "recent", so a smaller device later
    /// stops paying for a bigger one's cushion.
    pulls: [usize; 2],
    window: usize,
    /// Samples a peer may drop or repeat per window.
    budget: usize,
    deadband: usize,
    cut_above: usize,
    /// Samples pulled since the last window was judged.
    pulled: usize,
    underrun: Arc<AtomicU64>,
    adjusted: Arc<AtomicU64>,
}

struct Peer {
    ring: rtrb::Consumer<i16>,
    /// What the speakers work on (trims, drift steps): sized once, never grown in the callback.
    queue: VecDeque<i16>,
    /// Published fill (queue + ring) for the network side's `max_buffered`.
    filled: Arc<AtomicUsize>,
    /// Shortest the queue has been (right after a pull) this window.
    low: usize,
    gain: f32,
    /// Drift correction due this window: > 0 samples to drop, < 0 to repeat.
    adjust: i32,
    /// The front sample was just played once and is to be played again (a repeat).
    held: bool,
    /// Ran dry: silent until the queue holds `Mixer::refill` again.
    rebuffering: bool,
}

impl Mixer {
    /// Apply the network side's changes (only if they're not being written right now: never
    /// wait) and move what arrived from the rings into the working queues.
    fn intake(&mut self) {
        if let Ok(mut cmds) = self.cmds.try_lock() {
            for c in cmds.drain(..) {
                match c {
                    Cmd::Add {
                        id,
                        ring,
                        filled,
                        gain,
                    } => {
                        let p = Peer {
                            ring,
                            queue: VecDeque::with_capacity(3 * self.cap),
                            filled,
                            low: usize::MAX,
                            gain,
                            adjust: 0,
                            held: false,
                            rebuffering: false,
                        };
                        self.peers.insert(id, p);
                    }
                    Cmd::Gain { id, gain } => {
                        if let Some(p) = self.peers.get_mut(&id) {
                            p.gain = gain;
                        }
                    }
                    Cmd::Remove { id } => {
                        self.peers.remove(&id);
                    }
                    Cmd::Clear => self.peers.clear(),
                }
            }
        }
        for p in self.peers.values_mut() {
            let n = p.ring.slots();
            if let Ok(chunk) = p.ring.read_chunk(n) {
                let (a, b) = chunk.as_slices();
                p.queue.extend(a.iter().chain(b).copied());
                chunk.commit_all();
            }
            if p.queue.len() > self.cap {
                let excess = p.queue.len() - self.cap;
                skip(&mut p.queue, excess); // oldest out, crossfaded
            }
        }
    }

    /// Samples queued for `peer` (tests and diagnostics; takes in what arrived first).
    pub fn buffered(&mut self, peer: &str) -> usize {
        self.intake();
        self.peers.get(peer).map_or(0, |p| p.queue.len())
    }

    /// What a dry peer refills to before playing on: the target plus the largest recent pull.
    fn refill(&self) -> usize {
        self.target + self.pulls[0].max(self.pulls[1]).max(self.min_pull)
    }

    /// Fill interleaved `out` (`channels` per frame), one sample per peer per frame.
    pub fn mix_into(&mut self, out: &mut [f32], channels: usize, master: f32) {
        self.intake();
        let frames = out.len() / channels.max(1);
        self.pulls[0] = self.pulls[0].max(frames);
        let refill = self.refill();
        let (mut underrun, mut adjusted) = (0u64, 0u64);
        for p in self.peers.values_mut() {
            if p.rebuffering && p.queue.len() >= refill {
                p.rebuffering = false;
            }
        }
        for frame in out.chunks_mut(channels.max(1)) {
            let mut acc = 0.0f32;
            for p in self.peers.values_mut() {
                if p.rebuffering {
                    underrun += 1;
                    continue;
                }
                match p.next() {
                    Some((s, adj)) => {
                        acc += s as f32 / i16::MAX as f32 * p.gain;
                        adjusted += adj as u64;
                    }
                    None => {
                        // ran dry: refill before playing on (one gap, not a dribble of clicks)
                        underrun += 1;
                        p.rebuffering = true;
                    }
                }
            }
            frame.fill(soft_limit(acc * master));
        }
        for p in self.peers.values_mut() {
            p.low = p.low.min(p.queue.len());
        }
        self.pulled += frames;
        if self.pulled >= self.window {
            self.pulled = 0;
            self.pulls = [0, self.pulls[0]];
            for p in self.peers.values_mut() {
                p.judge(self.target, self.budget, self.deadband, self.cut_above);
            }
        }
        for p in self.peers.values() {
            p.filled.store(p.queue.len() + p.ring.slots(), Relaxed);
        }
        self.underrun.fetch_add(underrun, Relaxed);
        self.adjusted.fetch_add(adjusted, Relaxed);
    }
}

/// Drift steering: each window the queue's lowest fill is judged against the target, and the
/// difference is corrected one quiet sample at a time while it plays.
impl Peer {
    /// End of a window: set this window's correction from how low the queue got (samples).
    fn judge(&mut self, target: usize, budget: usize, deadband: usize, cut_above: usize) {
        if self.low != usize::MAX && !self.rebuffering {
            let over = self.low as i64 - target as i64;
            if over > cut_above as i64 {
                skip(&mut self.queue, over as usize);
                self.adjust = 0;
            } else if (0..=deadband as i64).contains(&over) {
                self.adjust = 0; // within a frame of the target: noise, not drift
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

    use super::SourceKind::{Stream, Voice};

    /// A mixer with each of `ids` added as a voice source owned by itself (most tests need one).
    fn mixer_with(rate: u32, cap_ms: u32, ids: &[&str]) -> (MixerInput, Mixer) {
        let (input, m) = mixer(rate, cap_ms);
        for id in ids {
            input.add_source(input.generation(), id, id, Voice);
        }
        (input, m)
    }

    /// One person sending two tracks: their mic and the audio of a stream they share.
    fn alex_mic_and_game() -> (MixerInput, Mixer) {
        let (input, m) = mixer(48_000, 200);
        let g = input.generation();
        input.add_source(g, "TR_mic", "alex", Voice);
        input.add_source(g, "TR_game", "alex", Stream);
        (input, m)
    }

    /// Push and pull at the same rate, as a call does once running.
    fn steady(input: &MixerInput, m: &mut Mixer, ms: usize) {
        let mut out = vec![0.0; 480];
        for _ in 0..ms / 10 {
            input.push("a", &[100; 480]);
            m.mix_into(&mut out, 1, 1.0);
        }
    }

    #[test]
    fn a_standing_backlog_drains_to_the_target() {
        // e.g. the output was reopened while audio kept arriving: the queue sits at the cap
        let (input, mut m) = mixer_with(48_000, 200, &["a"]);
        input.push("a", &[100; 9600]);
        steady(&input, &mut m, 2500);
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
        let (input, mut m) = mixer_with(48_000, 200, &["a"]);
        let mut next = 0;
        let mut feed = |input: &MixerInput, n: usize| {
            let chunk: Vec<i16> = (next..next + n).map(tone).collect();
            next += n;
            input.push("a", &chunk);
        };
        feed(&input, 9600); // a 200 ms standing backlog
        let mut played = Vec::new();
        let mut out = vec![0.0; 480];
        for _ in 0..250 {
            feed(&input, 480);
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
        let (input, mut m) = mixer_with(48_000, 200, &["a"]);
        input.push("a", &[1; 240]);
        let mut out = vec![0.0; 480];
        m.mix_into(&mut out, 1, 1.0);
        assert_eq!(input.take_underrun_samples(), 240);
        assert_eq!(input.take_underrun_samples(), 0, "restarts after a take");
    }

    #[test]
    fn a_queue_at_the_target_is_left_alone() {
        let (input, mut m) = mixer_with(48_000, 200, &["a"]);
        input.push("a", &[100; 480]); // 10 ms: at the target, nothing to drop or repeat
        steady(&input, &mut m, 3000);
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
        let (input, mut m) = mixer_with(48_000, 200, &["a", "b"]);
        input.push("a", &[8192; 4]); // 0.25
        input.push("b", &[8192; 4]);
        input.set_gain("b", Voice, 0.0);
        let mut out = [0.0f32; 8];
        m.mix_into(&mut out, 2, 1.0);
        for v in out {
            assert!((v - 0.25).abs() < 1e-3, "{v}");
        }
    }

    #[test]
    fn empty_peer_contributes_silence() {
        let (input, mut m) = mixer_with(48_000, 200, &["a"]);
        input.push("a", &[16384; 2]);
        let mut out = [1.0f32; 4];
        m.mix_into(&mut out, 1, 1.0);
        assert!((out[0] - 0.5).abs() < 1e-3);
        assert_eq!(&out[2..], &[0.0, 0.0]);
    }

    #[test]
    fn overflow_keeps_the_newest_audio_bounded() {
        let (input, mut m) = mixer_with(1000, 10, &["a"]); // cap = 10 samples
        let samples: Vec<i16> = (0..25).collect();
        input.push("a", &samples);
        assert_eq!(m.buffered("a"), 10);
        let mut out = [0.0f32; 10];
        m.mix_into(&mut out, 1, 1.0);
        // the newest 10 are kept, faded in from what was cut: last = 0.1 x 14 + 0.9 x 24 = 23
        // (the crossfade truncates to i16, so 22.99.. plays as 22)
        assert!(
            (out[9] * i16::MAX as f32 - 23.0).abs() < 1.5,
            "{}",
            out[9] * i16::MAX as f32
        );
    }

    #[test]
    fn remove_peer_drops_queue() {
        let (input, mut m) = mixer_with(48_000, 200, &["a"]);
        input.push("a", &[1; 10]);
        input.remove_source(input.generation(), "a");
        assert_eq!(m.buffered("a"), 0);
    }

    /// Receive tasks and the speakers share no lock: pushes from another thread arrive.
    #[test]
    fn pushes_from_another_thread_reach_the_speakers() {
        let (input, mut m) = mixer_with(48_000, 200, &["a"]);
        let input = std::sync::Arc::new(input);
        let i2 = input.clone();
        std::thread::spawn(move || i2.push("a", &[8192; 480]))
            .join()
            .unwrap();
        let mut out = vec![0.0f32; 480];
        m.mix_into(&mut out, 1, 1.0);
        assert!((out[0] - 0.25).abs() < 1e-3);
        assert_eq!(input.max_buffered(), 0, "fill is published after each pull");
        assert_eq!(input.pushed_total(), 480);
    }

    #[test]
    fn clear_forgets_the_last_rooms_peers() {
        let (input, mut m) = mixer_with(48_000, 200, &["a"]);
        input.push("a", &[100; 960]);
        input.clear();
        assert_eq!(m.buffered("a"), 0);
    }

    /// A receive task of the last room still mid-push when a switch clears the mixer must not
    /// bring its peer back: one nobody feeds would count underrun for the rest of the call.
    #[test]
    fn a_push_from_before_a_clear_is_ignored() {
        let (input, mut m) = mixer_with(48_000, 200, &["a"]);
        let old = input.generation();
        input.push_in(old, "a", &[100; 480]);
        input.clear();
        input.push_in(old, "a", &[100; 480]); // in flight across the clear
        assert!(
            input.take_peer_levels().is_empty(),
            "the old room's peer is back"
        );
        let mut out = vec![0.0f32; 480];
        m.mix_into(&mut out, 1, 1.0);
        assert_eq!(input.take_underrun_samples(), 0);
        input.add_source(input.generation(), "b", "b", Voice); // the new room's
        input.push_in(input.generation(), "b", &[100; 480]);
        assert_eq!(m.buffered("b"), 480);
    }

    #[test]
    fn per_peer_levels_reflect_pushed_audio() {
        let (input, _speakers) = mixer_with(48_000, 200, &["loud", "quiet"]);
        input.push("loud", &[16384; 480]);
        input.push("quiet", &[0; 480]);
        let mut levels = input.take_peer_levels();
        levels.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(levels[0].0, "loud");
        assert!((levels[0].1 - 0.5).abs() < 0.01);
        assert_eq!(levels[1].1, 0.0);
        assert!(
            input.take_peer_levels().iter().all(|(_, v)| *v == 0.0),
            "reset after take"
        );
    }

    #[test]
    fn two_tracks_from_one_person_both_play() {
        let (input, mut m) = alex_mic_and_game();
        input.push("TR_mic", &[8192; 4]); // 0.25
        input.push("TR_game", &[8192; 4]); // 0.25
        let mut out = [0.0f32; 4];
        m.mix_into(&mut out, 1, 1.0);
        for v in out {
            assert!((v - 0.5).abs() < 1e-3, "both sources mixed: {v}");
        }
    }

    #[test]
    fn removing_one_track_keeps_the_other() {
        let (input, mut m) = alex_mic_and_game();
        input.remove_source(input.generation(), "TR_game");
        input.push("TR_mic", &[8192; 4]);
        input.push("TR_game", &[8192; 4]); // gone: ignored
        let mut out = [0.0f32; 4];
        m.mix_into(&mut out, 1, 1.0);
        assert!((out[0] - 0.25).abs() < 1e-3, "{}", out[0]);
        assert_eq!(
            input.pushed_total(),
            4,
            "the removed track's push isn't counted"
        );
        assert_eq!(m.buffered("TR_game"), 0);
    }

    #[test]
    fn remove_owner_drops_all_their_sources() {
        let (input, mut m) = alex_mic_and_game();
        input.add_source(input.generation(), "TR_sam", "sam", Voice);
        for s in ["TR_mic", "TR_game", "TR_sam"] {
            input.push(s, &[1; 10]);
        }
        input.remove_owner(input.generation(), "alex");
        assert_eq!(m.buffered("TR_mic"), 0);
        assert_eq!(m.buffered("TR_game"), 0);
        assert_eq!(m.buffered("TR_sam"), 10);
        assert_eq!(input.take_peer_levels().len(), 1, "only sam is left");
    }

    #[test]
    fn voice_volume_does_not_touch_stream_audio() {
        let (input, mut m) = alex_mic_and_game();
        input.set_gain("alex", Voice, 0.0);
        input.push("TR_mic", &[8192; 4]);
        input.push("TR_game", &[8192; 4]);
        let mut out = [0.0f32; 4];
        m.mix_into(&mut out, 1, 1.0);
        assert!(
            (out[0] - 0.25).abs() < 1e-3,
            "only the stream plays: {}",
            out[0]
        );
    }

    #[test]
    fn gain_set_before_the_source_arrives_applies() {
        // the volume slider is restored before the person's track is subscribed
        let (input, mut m) = mixer(48_000, 200);
        input.set_gain("alex", Voice, 0.5);
        input.add_source(input.generation(), "TR_mic", "alex", Voice);
        input.push("TR_mic", &[16384; 2]); // 0.5
        let mut out = [0.0f32; 2];
        m.mix_into(&mut out, 1, 1.0);
        assert!((out[0] - 0.25).abs() < 1e-3, "{}", out[0]);
    }

    #[test]
    fn a_volume_survives_a_clear() {
        // a channel switch clears the room's people, not the person's volume
        let (input, mut m) = mixer(48_000, 200);
        input.set_gain("alex", Voice, 0.5);
        input.clear();
        input.add_source(input.generation(), "TR_mic", "alex", Voice);
        input.push("TR_mic", &[16384; 2]);
        let mut out = [0.0f32; 2];
        m.mix_into(&mut out, 1, 1.0);
        assert!((out[0] - 0.25).abs() < 1e-3, "{}", out[0]);
    }

    #[test]
    fn stream_audio_never_lights_the_speaking_ring() {
        let (input, _m) = alex_mic_and_game();
        input.add_source(input.generation(), "TR_sam_game", "sam", Stream);
        input.push("TR_mic", &[0; 480]);
        input.push("TR_game", &[16384; 480]);
        input.push("TR_sam_game", &[16384; 480]);
        assert_eq!(
            input.take_peer_levels(),
            vec![("alex".to_string(), 0.0)],
            "voice owners only, voice level only"
        );
    }

    #[test]
    fn re_adding_a_source_starts_it_empty() {
        let (input, mut m) = mixer_with(48_000, 200, &["a"]);
        input.push("a", &[1; 10]);
        input.add_source(input.generation(), "a", "a", Voice);
        assert_eq!(m.buffered("a"), 0);
    }

    /// A late frame from a receive task aborted mid-push (or a source never added) must not
    /// become a peer nobody feeds: it would count underrun for the rest of the call.
    #[test]
    fn a_push_for_an_unknown_source_is_ignored() {
        let (input, mut m) = mixer(48_000, 200);
        input.push("TR_gone", &[100; 480]);
        let mut out = vec![0.0f32; 480];
        m.mix_into(&mut out, 1, 1.0);
        assert_eq!(input.pushed_total(), 0);
        assert_eq!(input.take_underrun_samples(), 0);
        assert!(input.take_peer_levels().is_empty());
    }

    /// The last room's events task, still mid-handler when a switch clears the mixer.
    #[test]
    fn a_source_added_for_the_last_room_is_ignored() {
        let (input, mut m) = mixer(48_000, 200);
        let old = input.generation();
        input.clear();
        input.add_source(old, "TR_mic", "alex", Voice);
        input.push_in(old, "TR_mic", &[100; 480]);
        let mut out = vec![0.0f32; 480];
        m.mix_into(&mut out, 1, 1.0);
        assert_eq!(
            input.take_underrun_samples(),
            0,
            "a source nobody feeds was added"
        );
        assert!(input.take_peer_levels().is_empty());
        assert_eq!(m.buffered("TR_mic"), 0);
    }

    /// The same person in both rooms of a switch: the old room's "left" isn't about the new one.
    #[test]
    fn a_remove_for_the_last_room_leaves_the_new_one_alone() {
        let (input, mut m) = mixer(48_000, 200);
        let old = input.generation();
        input.clear();
        input.add_source(input.generation(), "TR_mic2", "alex", Voice);
        input.push("TR_mic2", &[1; 10]);
        input.remove_owner(old, "alex");
        input.remove_source(old, "TR_mic2");
        assert_eq!(m.buffered("TR_mic2"), 10);
    }

    use std::f64::consts::TAU;

    fn tone(i: usize) -> i16 {
        (10_000.0 * (i as f64 * 437.0 * TAU / 48_000.0).sin()) as i16
    }

    /// One sender, one speaker: per 10 ms pulled, the sender delivers 480 × (1 + ppm/10⁶) samples
    /// (fraction carried). Returns what played, as i16-scaled values.
    fn drifting(
        input: &MixerInput,
        m: &mut Mixer,
        secs: usize,
        ppm: f64,
        next: &mut usize,
    ) -> Vec<i32> {
        let (mut owed, mut out, mut played) = (0.0f64, vec![0.0f32; 480], Vec::new());
        for _ in 0..secs * 100 {
            owed += 480.0 * (1.0 + ppm / 1e6);
            let n = owed as usize;
            owed -= n as f64;
            let chunk: Vec<i16> = (*next..*next + n).map(tone).collect();
            *next += n;
            input.push("a", &chunk);
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
        let (input, mut m) = mixer_with(48_000, 200, &["a"]);
        let mut next = 0;
        let played = drifting(&input, &mut m, 60, 1000.0, &mut next);
        assert!(
            m.buffered("a") <= (TARGET_MS + 12) * 48,
            "queue grew to {} samples",
            m.buffered("a")
        );
        assert_eq!(input.take_underrun_samples(), 0);
        assert!(
            max_step(&played) < 1500,
            "a click: step of {}",
            max_step(&played)
        );
        assert!(
            input.take_adjusted_samples() > 0,
            "drift was corrected sample by sample"
        );
    }

    /// The sender's clock runs slow: once settled, the speakers must never run dry (today they
    /// do, a few samples at a time, forever).
    #[test]
    fn a_slow_sender_never_underruns_once_settled() {
        let (input, mut m) = mixer_with(48_000, 200, &["a"]);
        let mut next = 0;
        drifting(&input, &mut m, 10, -1000.0, &mut next);
        input.take_underrun_samples();
        let played = drifting(&input, &mut m, 50, -1000.0, &mut next);
        assert_eq!(input.take_underrun_samples(), 0, "ran dry while settled");
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
        let (input, mut m) = mixer_with(48_000, 200, &["a"]);
        let mut next = 0;
        drifting(&input, &mut m, 1, 0.0, &mut next);
        let mut out = vec![0.0f32; 480];
        for _ in 0..5 {
            m.mix_into(&mut out, 1, 1.0); // 50 ms with nothing arriving
        }
        assert!(input.take_underrun_samples() > 0);
        drifting(&input, &mut m, 1, 0.0, &mut next);
        assert!(
            m.buffered("a") >= TARGET_MS * 48,
            "resumed with {} samples of margin",
            m.buffered("a")
        );
    }

    /// A queue a little over the target (start-up) is eased down, not cut.
    #[test]
    fn a_small_excess_is_eased_out_without_a_cut() {
        let (input, mut m) = mixer_with(48_000, 200, &["a"]);
        let pre: Vec<i16> = (0..48 * 45).map(tone).collect(); // 45 ms
        input.push("a", &pre);
        let mut next = pre.len();
        let played = drifting(&input, &mut m, 3, 0.0, &mut next);
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
            let (input, mut m) = mixer_with(48_000, 200, &["a"]);
            let mut out = vec![0.0f32; 2048];
            let (mut next_push, mut next_pull) = (0.0f64, 0);
            let mut i = 0;
            for t in 0..48_000 * 30 {
                while next_push <= t as f64 {
                    let chunk: Vec<i16> = (i..i + 480).map(tone).collect();
                    i += 480;
                    input.push("a", &chunk);
                    next_push += 480.0 * (1.0 + ppm / 1e6);
                }
                if t == next_pull {
                    m.mix_into(&mut out, 1, 1.0);
                    next_pull += out.len();
                }
                if t == 48_000 {
                    assert!(input.take_underrun_samples() > 0, "the first pull ran dry");
                }
            }
            assert_eq!(
                input.take_underrun_samples(),
                0,
                "ran dry again at {ppm} ppm"
            );
            assert!(m.buffered("a") <= 4096, "{} queued", m.buffered("a"));
        }
    }

    /// A sender stall with big device callbacks (2048 frames): the refill must cover a whole
    /// pull, or the first pull after it runs dry again mid-way (two gaps for one stall).
    #[test]
    fn a_stall_with_big_callbacks_is_one_gap() {
        for pull in [1024, 2048] {
            let (input, mut m) = mixer_with(48_000, 200, &["a"]);
            let mut out = vec![0.0f32; pull];
            let (mut next_push, mut next_pull) = (0, 0);
            let stall = 48_000 * 5..48_000 * 5 + 4_800; // 100 ms with nothing arriving
            let mut played = Vec::new();
            for t in 0..48_000 * 8 {
                while next_push <= t {
                    if !stall.contains(&next_push) {
                        input.push("a", &[8192; 480]);
                    }
                    next_push += 480;
                }
                if t == next_pull {
                    m.mix_into(&mut out, 1, 1.0);
                    next_pull += out.len();
                    if t >= stall.start - 48_000 {
                        played.extend_from_slice(&out);
                    }
                }
            }
            // gaps = runs of silence in a constant signal
            let gaps = played
                .windows(2)
                .filter(|w| w[0] != 0.0 && w[1] == 0.0)
                .count();
            assert_eq!(gaps, 1, "{pull}-frame pulls: {gaps} gaps for one stall");
        }
    }

    /// Big callbacks beat against the sender's 10 ms frames, so each window's lowest fill swings
    /// by up to a frame with no drift at all: that's noise, not a reason to drop and repeat.
    #[test]
    fn big_callbacks_without_drift_are_barely_adjusted() {
        let (input, mut m) = mixer_with(48_000, 200, &["a"]);
        let mut out = vec![0.0f32; 2048];
        let (mut next_push, mut next_pull, mut i) = (0, 0, 0);
        for t in 0..48_000 * 40 {
            while next_push <= t {
                let chunk: Vec<i16> = (i..i + 480).map(tone).collect();
                i += 480;
                input.push("a", &chunk);
                next_push += 480;
            }
            if t == next_pull {
                m.mix_into(&mut out, 1, 1.0);
                next_pull += out.len();
            }
            if t == 48_000 * 5 {
                input.take_adjusted_samples(); // start-up settled
                input.take_underrun_samples();
            }
        }
        let per_window = input.take_adjusted_samples() / (35 * 4);
        assert!(per_window < 10, "{per_window} adjusted per window");
        assert_eq!(input.take_underrun_samples(), 0, "ran dry after start-up");
    }
}
