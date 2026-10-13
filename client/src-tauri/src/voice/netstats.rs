//! A periodic log line with what WebRTC measures about the voice path, so "it feels laggy" can be
//! split into network (round trip, jitter, loss) versus our own playout.

use std::fmt;

use livekit::webrtc::stats::{IceCandidatePairState, RtcStats};

/// How often a sample is taken (the voice loop logs one `voice: path` line per window).
pub const WINDOW: std::time::Duration = std::time::Duration::from_secs(5);

/// Running totals from the previous sample: WebRTC's jitter-buffer figures are cumulative.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Totals {
    buffer_delay_s: f64,
    target_delay_s: f64,
    min_delay_s: f64,
    emitted: u64,
    concealed: u64,
    received: u64,
    packets_sent: u64,
}

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct PathStats {
    /// Round trip to the LiveKit server, from the active candidate pair.
    pub rtt_ms: Option<f64>,
    /// Inbound audio jitter, worst across peers.
    pub jitter_ms: Option<f64>,
    /// Average time incoming audio sat in WebRTC's jitter buffer since the last sample.
    pub buffer_ms: Option<f64>,
    /// What NetEq aims the buffer at (it shrinks towards this), and the least it would accept.
    pub target_ms: Option<f64>,
    pub min_ms: Option<f64>,
    /// Share of received audio that was made up (loss, late packets, underruns).
    pub concealed_pct: Option<f64>,
    /// Packets lost so far, summed over peers.
    pub lost: i64,
    /// Our mic's packets per second (50 = 20 ms packets).
    pub send_pps: Option<f64>,
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
                totals.target_delay_s += r.inbound.jitter_buffer_target_delay;
                totals.min_delay_s += r.inbound.jitter_buffer_minimum_delay;
                totals.emitted += r.inbound.jitter_buffer_emitted_count;
                totals.concealed += r.inbound.concealed_samples;
                totals.received += r.inbound.total_samples_received;
            }
            RtcStats::OutboundRtp(o) if o.stream.kind == "audio" => {
                totals.packets_sent += o.sent.packets_sent;
            }
            _ => {}
        }
    }
    // A peer leaving makes the totals shrink; skip that sample rather than report nonsense.
    if totals.emitted > prev.emitted && totals.buffer_delay_s >= prev.buffer_delay_s {
        let per_sample = |now: f64, before: f64| {
            (now - before) / (totals.emitted - prev.emitted) as f64 * 1000.0
        };
        out.buffer_ms = Some(per_sample(totals.buffer_delay_s, prev.buffer_delay_s));
        out.target_ms = Some(per_sample(totals.target_delay_s, prev.target_delay_s));
        out.min_ms = Some(per_sample(totals.min_delay_s, prev.min_delay_s));
    }
    if totals.received > prev.received && totals.concealed >= prev.concealed {
        let concealed = (totals.concealed - prev.concealed) as f64;
        out.concealed_pct = Some(concealed / (totals.received - prev.received) as f64 * 100.0);
    }
    if totals.packets_sent > prev.packets_sent && prev.packets_sent > 0 {
        let sent = (totals.packets_sent - prev.packets_sent) as f64;
        out.send_pps = Some(sent / WINDOW.as_secs_f64());
    }
    (out, totals)
}

/// Our mic's codec as WebRTC negotiated it: "audio/opus ch2 minptime=10;useinbandfec=1;…".
pub fn send_codec(publisher: &[RtcStats]) -> Option<String> {
    publisher.iter().find_map(|s| match s {
        RtcStats::Codec(c) if c.codec.mime_type.starts_with("audio/") => Some(format!(
            "{} ch{} {}",
            c.codec.mime_type, c.codec.channels, c.codec.sdp_fmtp_line
        )),
        _ => None,
    })
}

impl fmt::Display for PathStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let ms = |v: Option<f64>| v.map_or("-".to_string(), |v| format!("{v:.0}ms"));
        let pct = |v: Option<f64>| v.map_or("-".to_string(), |v| format!("{v:.1}%"));
        let pps = self.send_pps.map_or("-".to_string(), |v| format!("{v:.0}"));
        write!(
            f,
            "rtt {} jitter {} buffer {} target {} min {} concealed {} lost {} send {} pkt/s",
            ms(self.rtt_ms),
            ms(self.jitter_ms),
            ms(self.buffer_ms),
            ms(self.target_ms),
            ms(self.min_ms),
            pct(self.concealed_pct),
            self.lost,
            pps
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
    fn the_send_codec_says_mono_or_stereo() {
        let mut c = livekit::webrtc::stats::CodecStats::default();
        c.codec.mime_type = "audio/opus".into();
        c.codec.channels = 2;
        c.codec.sdp_fmtp_line = "stereo=1;useinbandfec=1".into();
        assert_eq!(
            send_codec(&[RtcStats::Codec(c)]).as_deref(),
            Some("audio/opus ch2 stereo=1;useinbandfec=1")
        );
        assert_eq!(send_codec(&[]), None);
    }

    #[test]
    fn nothing_measured_yet_prints_dashes() {
        let (s, _) = summarize(&[], &[], Totals::default());
        assert_eq!(
            s.to_string(),
            "rtt - jitter - buffer - target - min - concealed - lost 0 send - pkt/s"
        );
    }

    fn audio_in_full(
        delay_s: f64,
        target_s: f64,
        min_s: f64,
        emitted: u64,
        concealed: u64,
        total: u64,
    ) -> RtcStats {
        let mut r = InboundRtpStats::default();
        r.stream.kind = "audio".into();
        r.inbound.jitter_buffer_delay = delay_s;
        r.inbound.jitter_buffer_target_delay = target_s;
        r.inbound.jitter_buffer_minimum_delay = min_s;
        r.inbound.jitter_buffer_emitted_count = emitted;
        r.inbound.concealed_samples = concealed;
        r.inbound.total_samples_received = total;
        RtcStats::InboundRtp(r)
    }

    fn audio_out(packets: u64) -> RtcStats {
        let mut o = livekit::webrtc::stats::OutboundRtpStats::default();
        o.stream.kind = "audio".into();
        o.sent.packets_sent = packets;
        RtcStats::OutboundRtp(o)
    }

    /// NetEq's own aim (target/minimum delay) says whether the jitter buffer could be smaller;
    /// concealment says how much was made up; packets/s says the packet size (50 = 20 ms).
    #[test]
    fn jitter_buffer_aim_concealment_and_send_rate_since_the_last_sample() {
        let (_, first) = summarize(
            &[audio_out(1000)],
            &[audio_in_full(1.0, 2.0, 1.0, 48_000, 0, 48_000)],
            Totals::default(),
        );
        // 4800 samples emitted; target summed 288 s → 60 ms each; minimum 144 s → 30 ms; 480 of
        // 4800 received samples concealed → 10 %; 250 packets in 5 s → 50/s
        let (s, _) = summarize(
            &[audio_out(1250)],
            &[audio_in_full(1.0, 290.0, 145.0, 52_800, 480, 52_800)],
            first,
        );
        assert_eq!(s.target_ms.map(f64::round), Some(60.0));
        assert_eq!(s.min_ms.map(f64::round), Some(30.0));
        assert_eq!(s.concealed_pct.map(f64::round), Some(10.0));
        assert_eq!(s.send_pps.map(f64::round), Some(50.0));
    }
}
