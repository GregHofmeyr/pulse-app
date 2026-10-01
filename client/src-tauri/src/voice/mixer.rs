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

pub struct Mixer {
    cap: usize,
    peers: HashMap<String, Peer>,
    /// Total samples ever pushed (diagnostics: intake must track real time, see FINDINGS).
    pushed: u64,
}

struct Peer {
    queue: VecDeque<i16>,
    gain: f32,
}

impl Mixer {
    pub fn new(sample_rate: u32, cap_ms: u32) -> Self {
        Self {
            cap: (sample_rate as usize * cap_ms as usize / 1000).max(1),
            peers: HashMap::new(),
            pushed: 0,
        }
    }

    fn peer(&mut self, id: &str) -> &mut Peer {
        self.peers.entry(id.to_string()).or_insert_with(|| Peer {
            queue: VecDeque::new(),
            gain: 1.0,
        })
    }

    /// Queue received samples (mono i16). Beyond the cap, the oldest are dropped (bounded latency).
    pub fn push(&mut self, peer: &str, samples: &[i16]) {
        let cap = self.cap;
        self.pushed += samples.len() as u64;
        let q = &mut self.peer(peer).queue;
        q.extend(samples.iter().copied());
        if q.len() > cap {
            let excess = q.len() - cap;
            q.drain(..excess);
        }
    }

    pub fn set_gain(&mut self, peer: &str, gain: f32) {
        self.peer(peer).gain = gain;
    }

    pub fn remove(&mut self, peer: &str) {
        self.peers.remove(peer);
    }

    pub fn pushed_total(&self) -> u64 {
        self.pushed
    }

    pub fn buffered(&self, peer: &str) -> usize {
        self.peers.get(peer).map_or(0, |p| p.queue.len())
    }

    /// Fill interleaved `out` (`channels` per frame), one sample per peer per frame.
    pub fn mix_into(&mut self, out: &mut [f32], channels: usize, master: f32) {
        for frame in out.chunks_mut(channels.max(1)) {
            let mut acc = 0.0f32;
            for p in self.peers.values_mut() {
                if let Some(s) = p.queue.pop_front() {
                    acc += s as f32 / i16::MAX as f32 * p.gain;
                }
            }
            frame.fill(soft_limit(acc * master));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let mut out = [0.0f32; 1];
        m.mix_into(&mut out, 1, 1.0);
        // the oldest 15 samples were dropped, so the first one played is sample value 15
        assert!((out[0] - 15.0 / i16::MAX as f32).abs() < 1e-7, "{}", out[0]);
    }

    #[test]
    fn remove_peer_drops_queue() {
        let mut m = Mixer::new(48_000, 200);
        m.push("a", &[1; 10]);
        m.remove("a");
        assert_eq!(m.buffered("a"), 0);
    }
}
