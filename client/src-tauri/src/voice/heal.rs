//! Self-heal for a subscribed track WebRTC stops handing frames to while its packets still arrive
//! (#216, #217). The known workaround was an app restart: rebuilding the process-wide WebRTC
//! runtime. This decides when to do that for the user, at most once per channel per
//! `HEAL_EVERY`, and never again in this run once a heal didn't help.
//!
//! A stalled receive task (`rx::Flow::Stalled`) starts a probe (`watch_stall`): every
//! `PROBE_EVERY` it reads the track's own receive stats. Packets arriving at `MIN_PPS` or more,
//! from a peer still there and not muted, is a dead receive: heal. A muted or unpublished track,
//! a peer who left, or a trickle (DTX: one packet each 400 ms while silent) is not.

use std::collections::HashMap;
use std::future::Future;
use std::hash::Hash;
use std::time::Duration;

use tokio::sync::watch;
use tokio::time::Instant;

/// Packets per second that count as "arriving". Opus DTX sends ~2.5/s while someone is silent;
/// speech is 50/s. A stall with fewer than this is someone quiet whose packets dried up, not a
/// receive WebRTC stopped pulling.
pub const MIN_PPS: f64 = 10.0;
/// How often a stalled track's stats are read.
pub const PROBE_EVERY: Duration = Duration::from_secs(2);
/// At most one heal per channel this often.
pub const HEAL_EVERY: Duration = Duration::from_secs(120);

/// A track's receive counters (its `inbound-rtp` stats).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counters {
    /// RTP packets received.
    pub packets: u64,
    /// Samples NetEq put out (`totalSamplesReceived`): moves only while WebRTC pulls the track.
    pub neteq_samples: u64,
}

/// One look at a stalled track.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Probe {
    /// The peer is still in the room with this track published.
    pub present: bool,
    /// The track is muted (the peer muted their mic, or it was unpublished midway).
    pub muted: bool,
    /// `None`: the stats couldn't be read this time.
    pub counters: Option<Counters>,
}

/// What a heal is based on: packets in versus audio out, over one probe window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Evidence {
    pub packets_in: u64,
    pub pps: f64,
    /// Audio NetEq put out over the window, in ms (0: WebRTC isn't pulling the track).
    pub neteq_ms: f64,
    pub window: Duration,
}

/// A stalled track, judged.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Judged {
    /// Packets arrive but no frames come out: rebuild the runtime.
    Heal(Evidence),
    /// Muted: no frames is expected. Look again later.
    Muted,
    /// The peer left or unpublished: nothing to heal, stop looking.
    Gone,
    /// Too few packets (DTX, or nothing arriving at all): look again later.
    Quiet { pps: f64 },
    /// No stats this time: look again later.
    Unknown,
}

/// `samples` per ms at the stats' 48 kHz.
const SAMPLES_PER_MS: f64 = 48.0;

/// Judges a stalled track from `now` and the counters `window` before (`None`: no earlier look).
pub fn judge(before: Option<Counters>, now: &Probe, window: Duration) -> Judged {
    if !now.present {
        return Judged::Gone;
    }
    if now.muted {
        return Judged::Muted;
    }
    let (Some(b), Some(n)) = (before, now.counters) else {
        return Judged::Unknown;
    };
    let packets_in = n.packets.saturating_sub(b.packets);
    let pps = packets_in as f64 / window.as_secs_f64();
    if pps < MIN_PPS {
        return Judged::Quiet { pps };
    }
    Judged::Heal(Evidence {
        packets_in,
        pps,
        neteq_ms: n.neteq_samples.saturating_sub(b.neteq_samples) as f64 / SAMPLES_PER_MS,
        window,
    })
}

/// Whether a stall may be healed now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    Go,
    /// This channel was healed `ago`: within `HEAL_EVERY`.
    RateLimited {
        ago: Duration,
    },
    /// The last heal stalled again for the same peer: this run stops healing.
    DidntHelp,
    /// A heal didn't help earlier in this run.
    GaveUp,
}

struct Verifying<K> {
    channel: K,
    peer: String,
    at: Instant,
}

/// Heals so far, per channel (`K`), and whether the last one helped.
pub struct Healer<K> {
    last: HashMap<K, Instant>,
    verifying: Option<Verifying<K>>,
    given_up: bool,
    every: Duration,
}

impl<K: Eq + Hash + Copy> Default for Healer<K> {
    fn default() -> Self {
        Self::new(HEAL_EVERY)
    }
}

impl<K: Eq + Hash + Copy> Healer<K> {
    pub fn new(every: Duration) -> Self {
        Self {
            last: HashMap::new(),
            verifying: None,
            given_up: false,
            every,
        }
    }

    /// `peer`'s track in `channel` stalled with packets arriving: may it be healed now? A `Go`
    /// is recorded as a heal (rate limit, and `peer` is watched for frames coming back).
    pub fn request(&mut self, channel: K, peer: &str, now: Instant) -> Gate {
        if self.given_up {
            return Gate::GaveUp;
        }
        if self
            .verifying
            .as_ref()
            .is_some_and(|v| v.channel == channel && v.peer == peer)
        {
            self.verifying = None;
            self.given_up = true;
            return Gate::DidntHelp;
        }
        if let Some(last) = self.last.get(&channel) {
            let ago = now.saturating_duration_since(*last);
            if ago < self.every {
                return Gate::RateLimited { ago };
            }
        }
        self.last.insert(channel, now);
        self.verifying = Some(Verifying {
            channel,
            peer: peer.to_string(),
            at: now,
        });
        Gate::Go
    }

    /// The heal's rejoin failed outright (no room): nothing to verify, the rate limit stands.
    pub fn rejoin_failed(&mut self) {
        self.verifying = None;
    }

    /// A frame from `peer` in `channel`: `Some(after)` the heal when it is the first since the
    /// heal that was waiting for them (it helped).
    pub fn frames_from(&mut self, channel: K, peer: &str, now: Instant) -> Option<Duration> {
        let v = self.verifying.as_ref()?;
        if v.channel != channel || v.peer != peer {
            return None;
        }
        let after = now.saturating_duration_since(v.at);
        self.verifying = None;
        Some(after)
    }

    /// Whether this run has stopped healing.
    pub fn given_up(&self) -> bool {
        self.given_up
    }
}

/// Watches one track's stall flag (`stalled`, set by its receive loop) and, while it is stalled,
/// probes it every `PROBE_EVERY`; calls `heal` once a probe says so, then waits for the stall to
/// end before looking again. Ends when the probe finds the peer or track gone, or `stalled`
/// closes.
pub async fn watch_stall<P, F>(
    mut stalled: watch::Receiver<bool>,
    mut probe: P,
    mut heal: impl FnMut(Evidence),
) where
    P: FnMut() -> F,
    F: Future<Output = Probe>,
{
    loop {
        // frames flowing: wait for a stall
        while !*stalled.borrow_and_update() {
            if stalled.changed().await.is_err() {
                return;
            }
        }
        let mut before = probe().await.counters;
        let mut tick = tokio::time::interval_at(Instant::now() + PROBE_EVERY, PROBE_EVERY);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut last = Instant::now();
        // stalled: probe until it heals, ends, or the track is gone
        let healed = loop {
            tokio::select! {
                r = stalled.changed() => {
                    if r.is_err() {
                        return;
                    }
                    if !*stalled.borrow_and_update() {
                        break false;
                    }
                }
                _ = tick.tick() => {
                    let p = probe().await;
                    let now = Instant::now();
                    match judge(before, &p, now.saturating_duration_since(last)) {
                        Judged::Gone => return,
                        Judged::Heal(e) => {
                            heal(e);
                            break true;
                        }
                        _ => {}
                    }
                    // a look without stats keeps the last one's as the window's start
                    if p.counters.is_some() {
                        (before, last) = (p.counters, now);
                    }
                }
            }
        };
        if healed {
            // once per stall: wait for it to end
            while *stalled.borrow_and_update() {
                if stalled.changed().await.is_err() {
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    const W: Duration = PROBE_EVERY;

    fn at(packets: u64, neteq_samples: u64) -> Probe {
        Probe {
            present: true,
            muted: false,
            counters: Some(Counters {
                packets,
                neteq_samples,
            }),
        }
    }

    fn c(packets: u64) -> Option<Counters> {
        Some(Counters {
            packets,
            neteq_samples: 0,
        })
    }

    /// #216's signature: speech packets arriving (50/s), nothing played.
    #[test]
    fn packets_arriving_on_a_stalled_track_heal() {
        match judge(c(1_000), &at(1_100, 0), W) {
            Judged::Heal(e) => {
                assert_eq!(e.packets_in, 100);
                assert_eq!(e.pps, 50.0);
                assert_eq!(e.neteq_ms, 0.0);
                assert_eq!(e.window, W);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_muted_track_is_not_healed() {
        let p = Probe {
            muted: true,
            ..at(1_100, 0)
        };
        assert_eq!(judge(c(1_000), &p, W), Judged::Muted);
    }

    #[test]
    fn a_peer_who_left_is_not_healed() {
        let p = Probe {
            present: false,
            ..at(1_100, 0)
        };
        assert_eq!(judge(c(1_000), &p, W), Judged::Gone);
    }

    /// DTX: a packet each 400 ms while silent (2.5/s) is someone quiet, not a dead receive.
    #[test]
    fn a_dtx_trickle_is_not_healed() {
        assert_eq!(
            judge(c(1_000), &at(1_005, 0), W),
            Judged::Quiet { pps: 2.5 }
        );
        assert_eq!(
            judge(c(1_000), &at(1_000, 0), W),
            Judged::Quiet { pps: 0.0 }
        );
    }

    /// The first look has nothing to compare with; a look without stats says nothing either.
    #[test]
    fn no_window_or_no_stats_is_unknown() {
        assert_eq!(judge(None, &at(1_100, 0), W), Judged::Unknown);
        let p = Probe {
            counters: None,
            ..at(0, 0)
        };
        assert_eq!(judge(c(1_000), &p, W), Judged::Unknown);
    }

    /// The NetEq output over the window is part of the evidence (0 = WebRTC isn't pulling).
    #[test]
    fn the_evidence_carries_neteq_output() {
        let before = Some(Counters {
            packets: 0,
            neteq_samples: 48_000,
        });
        match judge(before, &at(100, 96_000), W) {
            Judged::Heal(e) => assert_eq!(e.neteq_ms, 1000.0),
            other => panic!("{other:?}"),
        }
    }

    fn t0() -> Instant {
        Instant::now()
    }

    #[tokio::test(start_paused = true)]
    async fn one_heal_per_channel_per_two_minutes() {
        let mut h = Healer::<u8>::default();
        let now = t0();
        assert_eq!(h.request(1, "sam", now), Gate::Go);
        assert_eq!(
            h.frames_from(1, "sam", now + Duration::from_secs(1)),
            Some(Duration::from_secs(1))
        );
        let later = now + Duration::from_secs(60);
        assert_eq!(
            h.request(1, "kim", later),
            Gate::RateLimited {
                ago: Duration::from_secs(60)
            }
        );
        // another channel has its own budget
        assert_eq!(h.request(2, "kim", later), Gate::Go);
        // and two minutes on, this one may heal again
        assert_eq!(h.request(1, "kim", now + HEAL_EVERY), Gate::Go);
        assert!(!h.given_up());
    }

    /// The healed peer's new track stalls too: the rebuild didn't help, and this run stops trying
    /// (however long it waits), so a heal can never loop.
    #[tokio::test(start_paused = true)]
    async fn no_second_attempt_after_a_heal_that_didnt_help() {
        let mut h = Healer::<u8>::default();
        let now = t0();
        assert_eq!(h.request(1, "sam", now), Gate::Go);
        assert_eq!(
            h.request(1, "sam", now + Duration::from_secs(8)),
            Gate::DidntHelp
        );
        assert!(h.given_up());
        let much_later = now + HEAL_EVERY * 10;
        assert_eq!(h.request(1, "sam", much_later), Gate::GaveUp);
        assert_eq!(h.request(2, "kim", much_later), Gate::GaveUp);
    }

    /// Frames came back after the heal: a later stall of the same peer (past the rate limit) is
    /// a new one, healed again.
    #[tokio::test(start_paused = true)]
    async fn a_heal_that_helped_allows_another_later() {
        let mut h = Healer::<u8>::default();
        let now = t0();
        assert_eq!(h.request(1, "sam", now), Gate::Go);
        assert!(
            h.frames_from(1, "kim", now).is_none(),
            "someone else's frames say nothing"
        );
        assert!(
            h.frames_from(2, "sam", now).is_none(),
            "nor frames in another channel"
        );
        assert!(h.frames_from(1, "sam", now).is_some());
        assert!(h.frames_from(1, "sam", now).is_none(), "reported once");
        assert_eq!(h.request(1, "sam", now + HEAL_EVERY), Gate::Go);
    }

    /// A heal whose rejoin failed has nothing to verify; the rate limit still holds.
    #[tokio::test(start_paused = true)]
    async fn a_failed_rejoin_is_not_a_failed_heal() {
        let mut h = Healer::<u8>::default();
        let now = t0();
        assert_eq!(h.request(1, "sam", now), Gate::Go);
        h.rejoin_failed();
        assert!(h.frames_from(1, "sam", now).is_none());
        assert!(matches!(
            h.request(1, "sam", now + Duration::from_secs(5)),
            Gate::RateLimited { .. }
        ));
        assert!(!h.given_up());
    }

    /// A scripted track: probes come from `script` in turn (the last repeats).
    fn scripted(
        script: Vec<Probe>,
    ) -> (impl FnMut() -> std::future::Ready<Probe>, Arc<Mutex<usize>>) {
        let n = Arc::new(Mutex::new(0));
        let k = n.clone();
        let probe = move || {
            let mut i = k.lock().unwrap();
            let p = script[(*i).min(script.len() - 1)];
            *i += 1;
            std::future::ready(p)
        };
        (probe, n)
    }

    type Heals = Arc<Mutex<Vec<Evidence>>>;

    fn spawn_watch(
        script: Vec<Probe>,
    ) -> (
        watch::Sender<bool>,
        Heals,
        Arc<Mutex<usize>>,
        tokio::task::JoinHandle<()>,
    ) {
        let (tx, rx) = watch::channel(false);
        let (probe, probes) = scripted(script);
        let heals: Heals = Default::default();
        let h = heals.clone();
        let task = tokio::spawn(watch_stall(rx, probe, move |e| h.lock().unwrap().push(e)));
        (tx, heals, probes, task)
    }

    async fn wait(d: Duration) {
        tokio::time::sleep(d).await;
        tokio::task::yield_now().await;
    }

    #[tokio::test(start_paused = true)]
    async fn nothing_is_probed_while_frames_flow() {
        let (_tx, heals, probes, _t) = spawn_watch(vec![at(0, 0)]);
        wait(PROBE_EVERY * 5).await;
        assert_eq!(*probes.lock().unwrap(), 0);
        assert!(heals.lock().unwrap().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn a_stall_with_packets_arriving_heals_once() {
        let script = (0..20).map(|i| at(i * 100, 0)).collect();
        let (tx, heals, _probes, _t) = spawn_watch(script);
        tx.send(true).unwrap();
        wait(PROBE_EVERY + Duration::from_millis(10)).await;
        assert_eq!(heals.lock().unwrap().len(), 1, "after one window");
        wait(PROBE_EVERY * 5).await;
        assert_eq!(heals.lock().unwrap().len(), 1, "once per stall");
        // frames came back, then stalled again: a new stall may heal again
        tx.send(false).unwrap();
        wait(Duration::from_millis(10)).await;
        tx.send(true).unwrap();
        wait(PROBE_EVERY * 2).await;
        assert_eq!(heals.lock().unwrap().len(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn a_muted_stall_waits_and_heals_once_unmuted_packets_flow() {
        let muted = |n| Probe {
            muted: true,
            ..at(n, 0)
        };
        let script = vec![muted(0), muted(0), muted(0), at(0, 0), at(100, 0)];
        let (tx, heals, _probes, _t) = spawn_watch(script);
        tx.send(true).unwrap();
        wait(PROBE_EVERY * 3).await;
        assert!(heals.lock().unwrap().is_empty(), "muted: no heal");
        wait(PROBE_EVERY * 2).await;
        assert_eq!(heals.lock().unwrap().len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn a_dtx_stall_never_heals() {
        let script = (0..50).map(|i| at(i * 5, 0)).collect();
        let (tx, heals, _probes, _t) = spawn_watch(script);
        tx.send(true).unwrap();
        wait(PROBE_EVERY * 20).await;
        assert!(heals.lock().unwrap().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn a_peer_who_left_ends_the_watch() {
        let gone = Probe {
            present: false,
            ..at(0, 0)
        };
        let (tx, heals, _probes, task) = spawn_watch(vec![gone]);
        tx.send(true).unwrap();
        wait(PROBE_EVERY * 2).await;
        assert!(task.is_finished());
        assert!(heals.lock().unwrap().is_empty());
    }
}
