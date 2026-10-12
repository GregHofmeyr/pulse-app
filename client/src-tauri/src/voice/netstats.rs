//! A periodic log line with what WebRTC measures about the voice path, so "it feels laggy" can be
//! split into network (round trip, jitter, loss) versus our own playout.

use std::fmt;

use livekit::webrtc::stats::{IceCandidatePairState, RtcStats};

/// Running totals from the previous sample: WebRTC's jitter-buffer figures are cumulative.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Totals {
    buffer_delay_s: f64,
    emitted: u64,
}

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct PathStats {
    /// Round trip to the LiveKit server, from the active candidate pair.
    pub rtt_ms: Option<f64>,
    /// Inbound audio jitter, worst across peers.
    pub jitter_ms: Option<f64>,
    /// Average time incoming audio sat in WebRTC's jitter buffer since the last sample.
    pub buffer_ms: Option<f64>,
    /// Packets lost so far, summed over peers.
    pub lost: i64,
}

pub fn summarize(
    publisher: &[RtcStats],
    subscriber: &[RtcStats],
    prev: Totals,
) -> (PathStats, Totals) {
    let mut out = PathStats::default();
    let mut totals = Totals::default();
    for s in publisher.iter().chain(subscriber) {
        match s {
            RtcStats::CandidatePair(p) => {
                let p = &p.candidate_pair;
                if p.nominated
                    && p.state == Some(IceCandidatePairState::Succeeded)
                    && p.current_round_trip_time > 0.0
                {
                    let rtt = p.current_round_trip_time * 1000.0;
                    out.rtt_ms = Some(out.rtt_ms.map_or(rtt, |r: f64| r.max(rtt)));
                }
            }
            RtcStats::InboundRtp(r) if r.stream.kind == "audio" => {
                let jitter = r.received.jitter * 1000.0;
                out.jitter_ms = Some(out.jitter_ms.map_or(jitter, |j: f64| j.max(jitter)));
                out.lost += r.received.packets_lost;
                totals.buffer_delay_s += r.inbound.jitter_buffer_delay;
                totals.emitted += r.inbound.jitter_buffer_emitted_count;
            }
            _ => {}
        }
    }
    // A peer leaving makes the totals shrink; skip that sample rather than report nonsense.
    if totals.emitted > prev.emitted && totals.buffer_delay_s >= prev.buffer_delay_s {
        let delay = totals.buffer_delay_s - prev.buffer_delay_s;
        out.buffer_ms = Some(delay / (totals.emitted - prev.emitted) as f64 * 1000.0);
    }
    (out, totals)
}

impl fmt::Display for PathStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let ms = |v: Option<f64>| v.map_or("-".to_string(), |v| format!("{v:.0}ms"));
        write!(
            f,
            "rtt {} jitter {} buffer {} lost {}",
            ms(self.rtt_ms),
            ms(self.jitter_ms),
            ms(self.buffer_ms),
            self.lost
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use livekit::webrtc::stats::{CandidatePairStats, InboundRtpStats};

    fn pair(nominated: bool, rtt_s: f64) -> RtcStats {
        let mut p = CandidatePairStats::default();
        p.candidate_pair.nominated = nominated;
        p.candidate_pair.state = Some(IceCandidatePairState::Succeeded);
        p.candidate_pair.current_round_trip_time = rtt_s;
        RtcStats::CandidatePair(p)
    }

    fn audio_in(jitter_s: f64, lost: i64, delay_s: f64, emitted: u64) -> RtcStats {
        let mut r = InboundRtpStats::default();
        r.stream.kind = "audio".into();
        r.received.jitter = jitter_s;
        r.received.packets_lost = lost;
        r.inbound.jitter_buffer_delay = delay_s;
        r.inbound.jitter_buffer_emitted_count = emitted;
        RtcStats::InboundRtp(r)
    }

    #[test]
    fn reads_rtt_from_the_active_pair_only() {
        let (s, _) = summarize(
            &[pair(false, 0.9), pair(true, 0.042)],
            &[],
            Totals::default(),
        );
        assert_eq!(s.rtt_ms.map(f64::round), Some(42.0));
    }

    #[test]
    fn jitter_is_the_worst_peer_and_loss_is_summed() {
        let peers = [audio_in(0.004, 1, 0.0, 0), audio_in(0.012, 2, 0.0, 0)];
        let (s, _) = summarize(&[], &peers, Totals::default());
        assert_eq!(s.jitter_ms.map(f64::round), Some(12.0));
        assert_eq!(s.lost, 3);
    }

    #[test]
    fn buffer_delay_is_the_average_since_the_last_sample() {
        // 4800 new samples that waited 2400 s in total (the delay is summed per sample) → 500 ms each
        let (_, first) = summarize(&[], &[audio_in(0.0, 0, 1.0, 48_000)], Totals::default());
        let (s, _) = summarize(&[], &[audio_in(0.0, 0, 2401.0, 52_800)], first);
        assert_eq!(s.buffer_ms.map(f64::round), Some(500.0));
    }

    #[test]
    fn nothing_measured_yet_prints_dashes() {
        let (s, _) = summarize(&[], &[], Totals::default());
        assert_eq!(s.to_string(), "rtt - jitter - buffer - lost 0");
    }
}
