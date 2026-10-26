//! Link figures for the status panel: ping, jitter, loss and connection type on OUR path to the
//! voice server, plus a 60 s ping history. Sampled from the stats the voice loop already takes;
//! reported to the UI only while its panel is open.

use std::collections::VecDeque;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use livekit::webrtc::stats::{IceCandidatePairState, IceCandidateType, RtcStats};
use serde::Serialize;
use tokio::sync::Notify;
use tokio::time::Instant;

use super::netstats;

/// How often a watched panel is refreshed.
pub const LINK_EVERY: Duration = Duration::from_secs(1);
/// How much ping history is kept (the sparkline).
pub const HISTORY: Duration = Duration::from_secs(60);
/// Loss is measured over this much recent audio.
pub const LOSS_WINDOW: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Transport {
    Udp,
    Tcp,
    Turn,
}

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct LinkSample {
    pub rtt_ms: Option<f64>,
    pub jitter_ms: Option<f64>,
    pub loss_pct: Option<f64>,
    pub transport: Option<Transport>,
}

/// Cumulative counters at one sample, oldest first, trimmed to `LOSS_WINDOW`.
#[derive(Debug, Default)]
pub struct LossWindow(VecDeque<(Instant, i64, u64, u64)>);

impl LossWindow {
    /// Add this sample's counters; returns the loss over the window (None until it has a
    /// denominator).
    fn push(&mut self, now: Instant, lost: i64, received: u64, sent: u64) -> Option<f64> {
        // counters that went backwards mean a restarted stream: start over
        if let Some(&(_, l, r, s)) = self.0.back()
            && (lost < l || received < r || sent < s)
        {
            self.0.clear();
        }
        self.0.push_back((now, lost, received, sent));
        while self
            .0
            .front()
            .is_some_and(|(t, ..)| now.duration_since(*t) > LOSS_WINDOW)
        {
            self.0.pop_front();
        }
        let (_, l0, r0, s0) = *self.0.front()?;
        let (lost, received, sent) = ((lost - l0).max(0) as f64, received - r0, sent - s0);
        let denom = if received > 0 {
            lost + received as f64
        } else {
            sent as f64
        };
        (denom > 0.0).then(|| lost / denom * 100.0)
    }
}

pub fn sample(publisher: &[RtcStats], loss: &mut LossWindow, now: Instant) -> LinkSample {
    let mut out = LinkSample::default();
    let mut local_id = None;
    let (mut lost, mut received, mut sent) = (0i64, 0u64, 0u64);
    for s in publisher {
        match s {
            RtcStats::CandidatePair(p) => {
                let c = &p.candidate_pair;
                if c.nominated && c.state == Some(IceCandidatePairState::Succeeded) {
                    local_id = Some(c.local_candidate_id.as_str());
                    if c.current_round_trip_time > 0.0 {
                        out.rtt_ms = Some(c.current_round_trip_time * 1000.0);
                    }
                }
            }
            RtcStats::RemoteInboundRtp(r) if r.stream.kind == "audio" => {
                let j = r.received.jitter * 1000.0;
                out.jitter_ms = Some(out.jitter_ms.map_or(j, |x: f64| x.max(j)));
                lost += r.received.packets_lost;
                received += r.received.packets_received;
            }
            RtcStats::OutboundRtp(o) if o.stream.kind == "audio" => {
                sent += o.sent.packets_sent;
            }
            _ => {}
        }
    }
    if let Some(id) = local_id {
        out.transport = publisher.iter().find_map(|s| match s {
            RtcStats::LocalCandidate(c) if c.rtc.id == id => {
                let c = &c.local_candidate;
                Some(if c.candidate_type == Some(IceCandidateType::Relay) {
                    Transport::Turn
                } else if c.protocol.eq_ignore_ascii_case("tcp") {
                    Transport::Tcp
                } else {
                    Transport::Udp
                })
            }
            _ => None,
        });
    }
    out.loss_pct = loss.push(now, lost, received, sent);
    out
}

#[derive(Debug, Default)]
pub struct LinkHistory(VecDeque<(Instant, f64)>);

impl LinkHistory {
    pub fn push(&mut self, now: Instant, rtt_ms: f64) {
        self.0.push_back((now, rtt_ms));
        self.trim(now);
    }

    fn trim(&mut self, now: Instant) {
        while self
            .0
            .front()
            .is_some_and(|(t, _)| now.duration_since(*t) > HISTORY)
        {
            self.0.pop_front();
        }
    }

    /// (ms ago, rtt ms), oldest first.
    pub fn points(&self, now: Instant) -> Vec<(u32, f64)> {
        self.0
            .iter()
            .filter(|(t, _)| now.duration_since(*t) <= HISTORY)
            .map(|(t, v)| (now.duration_since(*t).as_millis() as u32, *v))
            .collect()
    }
}

#[derive(Debug, Default)]
pub struct LinkState {
    watching: AtomicBool,
    wake: Notify,
    pub history: Mutex<LinkHistory>,
    pub server: Mutex<Option<String>>,
}

impl LinkState {
    /// The status panel opened (`true`) or closed. Opening wakes the tick loop for an
    /// immediate report.
    pub fn set_watching(&self, on: bool) {
        self.watching.store(on, Ordering::SeqCst);
        if on {
            self.wake.notify_one();
        }
    }

    pub fn is_watching(&self) -> bool {
        self.watching.load(Ordering::SeqCst)
    }

    /// A call ended: its history and server are not the next call's.
    pub fn clear(&self) {
        *self.history.lock().unwrap() = LinkHistory::default();
        *self.server.lock().unwrap() = None;
    }

    pub fn report(&self, s: LinkSample, now: Instant) -> LinkReport {
        LinkReport {
            rtt_ms: s.rtt_ms,
            jitter_ms: s.jitter_ms,
            loss_pct: s.loss_pct,
            transport: s.transport,
            server: self.server.lock().unwrap().clone(),
            version: env!("CARGO_PKG_VERSION"),
            history: self.history.lock().unwrap().points(now),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LinkReport {
    pub rtt_ms: Option<f64>,
    pub jitter_ms: Option<f64>,
    pub loss_pct: Option<f64>,
    pub transport: Option<Transport>,
    pub server: Option<String>,
    pub version: &'static str,
    /// (ms ago, rtt ms), oldest first.
    pub history: Vec<(u32, f64)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tick {
    /// Take the `voice: path` log sample.
    pub log: bool,
    /// Send a `Link` report to the UI.
    pub report: bool,
}

/// Drives the stats task: a log tick every `netstats::WINDOW`, plus a report tick every
/// `LINK_EVERY` while the panel is watched (the first one immediately).
pub async fn run_ticks<F, Fut>(link: Arc<LinkState>, mut on_tick: F)
where
    F: FnMut(Tick) -> Fut,
    Fut: Future<Output = ()>,
{
    let start = Instant::now();
    let mut log = tokio::time::interval_at(start + netstats::WINDOW, netstats::WINDOW);
    let mut every = tokio::time::interval_at(start + LINK_EVERY, LINK_EVERY);
    every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut last_report: Option<Instant> = None;
    loop {
        tokio::select! {
            _ = log.tick() => {
                let report = link.is_watching();
                if report {
                    last_report = Some(Instant::now());
                }
                on_tick(Tick { log: true, report }).await;
            }
            _ = every.tick(), if link.is_watching() => {
                // closed since the guard was read; or a log tick that just reported covers this one
                if !link.is_watching() {
                    continue;
                }
                if last_report.is_some_and(|t| t.elapsed() < LINK_EVERY / 2) {
                    continue;
                }
                last_report = Some(Instant::now());
                on_tick(Tick { log: false, report: true }).await;
            }
            _ = link.wake.notified() => {
                if link.is_watching() {
                    every.reset();
                    last_report = Some(Instant::now());
                    on_tick(Tick { log: false, report: true }).await;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use livekit::webrtc::stats::{
        CandidatePairStats, LocalCandidateStats, OutboundRtpStats, RemoteInboundRtpStats,
    };

    fn pair(nominated: bool, rtt_s: f64, local: &str) -> RtcStats {
        let mut p = CandidatePairStats::default();
        p.candidate_pair.nominated = nominated;
        p.candidate_pair.state = Some(IceCandidatePairState::Succeeded);
        p.candidate_pair.current_round_trip_time = rtt_s;
        p.candidate_pair.local_candidate_id = local.into();
        RtcStats::CandidatePair(p)
    }

    fn remote_in(jitter_s: f64, lost: i64, received: u64) -> RtcStats {
        let mut r = RemoteInboundRtpStats::default();
        r.stream.kind = "audio".into();
        r.received.jitter = jitter_s;
        r.received.packets_lost = lost;
        r.received.packets_received = received;
        RtcStats::RemoteInboundRtp(r)
    }

    fn sent(packets: u64) -> RtcStats {
        let mut o = OutboundRtpStats::default();
        o.stream.kind = "audio".into();
        o.sent.packets_sent = packets;
        RtcStats::OutboundRtp(o)
    }

    fn local(id: &str, kind: IceCandidateType, protocol: &str) -> RtcStats {
        let mut c = LocalCandidateStats::default();
        c.rtc.id = id.into();
        c.local_candidate.candidate_type = Some(kind);
        c.local_candidate.protocol = protocol.into();
        RtcStats::LocalCandidate(c)
    }

    fn at(base: Instant, secs: u64) -> Instant {
        base + Duration::from_secs(secs)
    }

    #[test]
    fn rtt_from_the_active_pair_only() {
        let s = sample(
            &[pair(false, 0.9, "a"), pair(true, 0.042, "b")],
            &mut LossWindow::default(),
            Instant::now(),
        );
        assert_eq!(s.rtt_ms.map(f64::round), Some(42.0));
    }

    #[test]
    fn jitter_from_remote_inbound_audio() {
        let s = sample(
            &[remote_in(0.018, 0, 0)],
            &mut LossWindow::default(),
            Instant::now(),
        );
        assert_eq!(s.jitter_ms.map(f64::round), Some(18.0));
    }

    #[test]
    fn loss_over_the_window() {
        let t0 = Instant::now();
        let mut w = LossWindow::default();
        assert_eq!(sample(&[remote_in(0.0, 0, 100)], &mut w, t0).loss_pct, None);
        let s = sample(&[remote_in(0.0, 2, 198)], &mut w, at(t0, 1));
        assert!((s.loss_pct.unwrap() - 2.0).abs() < 1e-9);
        // the first sample is now older than the window: only the last two count
        let s = sample(&[remote_in(0.0, 2, 298)], &mut w, at(t0, 6));
        assert_eq!(s.loss_pct, Some(0.0));
        // and with nothing recent to compare against there is no figure
        let s = sample(&[remote_in(0.0, 2, 398)], &mut w, at(t0, 20));
        assert_eq!(s.loss_pct, None);
    }

    #[test]
    fn loss_falls_back_to_packets_sent() {
        let t0 = Instant::now();
        let mut w = LossWindow::default();
        sample(&[remote_in(0.0, 0, 0), sent(0)], &mut w, t0);
        let s = sample(&[remote_in(0.0, 1, 0), sent(100)], &mut w, at(t0, 1));
        assert!((s.loss_pct.unwrap() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn transport_turn_tcp_udp() {
        let kind = |k, proto| {
            sample(
                &[pair(true, 0.01, "l"), local("l", k, proto)],
                &mut LossWindow::default(),
                Instant::now(),
            )
            .transport
        };
        assert_eq!(kind(IceCandidateType::Relay, "udp"), Some(Transport::Turn));
        assert_eq!(kind(IceCandidateType::Host, "tcp"), Some(Transport::Tcp));
        assert_eq!(kind(IceCandidateType::Srflx, "udp"), Some(Transport::Udp));
    }

    #[test]
    fn history_keeps_sixty_seconds() {
        let t0 = Instant::now();
        let mut h = LinkHistory::default();
        h.push(t0, 10.0);
        h.push(at(t0, 30), 20.0);
        h.push(at(t0, 61), 30.0);
        let p = h.points(at(t0, 61));
        assert_eq!(p, vec![(31_000, 20.0), (0, 30.0)]);
    }

    #[tokio::test(start_paused = true)]
    async fn run_ticks_only_reports_while_watched() {
        let link = Arc::new(LinkState::default());
        let seen = Arc::new(Mutex::new(Vec::<Tick>::new()));
        let task = {
            let (link, seen) = (link.clone(), seen.clone());
            tokio::spawn(run_ticks(link, move |t| {
                let seen = seen.clone();
                async move { seen.lock().unwrap().push(t) }
            }))
        };
        let take = || std::mem::take(&mut *seen.lock().unwrap());

        tokio::time::sleep(Duration::from_millis(20_500)).await;
        let t = take();
        assert_eq!(t.len(), 4);
        assert!(t.iter().all(|t| t.log && !t.report));

        link.set_watching(true);
        tokio::time::sleep(Duration::from_millis(1)).await;
        assert_eq!(
            take(),
            vec![Tick {
                log: false,
                report: true
            }]
        );
        tokio::time::sleep(Duration::from_secs(10)).await;
        let t = take();
        assert!(t.iter().filter(|t| t.report).count() >= 9, "{t:?}");

        link.set_watching(false);
        tokio::time::sleep(Duration::from_millis(1)).await;
        take();
        tokio::time::sleep(Duration::from_secs(10)).await;
        let t = take();
        assert!(t.iter().all(|t| t.log && !t.report), "{t:?}");
        task.abort();
    }

    #[test]
    fn link_event_serialises_for_the_ui() {
        let t0 = Instant::now();
        let link = LinkState::default();
        *link.server.lock().unwrap() = Some("voice.example.test".into());
        link.history.lock().unwrap().push(t0, 42.0);
        let report = link.report(
            LinkSample {
                rtt_ms: Some(42.0),
                transport: Some(Transport::Turn),
                ..Default::default()
            },
            t0,
        );
        let v = serde_json::to_value(super::super::VoiceEvent::Link(report)).unwrap();
        assert_eq!(v["kind"], "link");
        assert_eq!(v["transport"], "turn");
        assert_eq!(v["server"], "voice.example.test");
        assert_eq!(v["history"], serde_json::json!([[0, 42.0]]));
        assert!(v["version"].is_string());
    }
}
