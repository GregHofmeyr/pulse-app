//! A periodic log line with what WebRTC measures about the voice path, so "it feels laggy" can be
//! split into network (round trip, jitter, loss) versus our own playout.

use std::collections::HashMap;
use std::fmt;

use livekit::webrtc::stats::{IceCandidatePairState, InboundRtpStats, RtcStats};

/// How often a sample is taken (the voice loop logs one `voice: path` line per window).
pub const WINDOW: std::time::Duration = std::time::Duration::from_secs(5);

/// One inbound audio stream's counters. WebRTC's jitter-buffer figures are running totals since
/// the stream started, so a window's numbers are the difference between two samples of the SAME
/// stream (by SSRC). Summing across peers breaks when one leaves and another talks.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
struct Inbound {
    buffer_delay_s: f64,
    target_delay_s: f64,
    min_delay_s: f64,
    emitted: u64,
    received: u64,
    concealed: u64,
    silent_concealed: u64,
    accelerated: u64,
    decelerated: u64,
}

impl Inbound {
    fn of(r: &InboundRtpStats) -> Self {
        let i = &r.inbound;
        Self {
            buffer_delay_s: i.jitter_buffer_delay,
            target_delay_s: i.jitter_buffer_target_delay,
            min_delay_s: i.jitter_buffer_minimum_delay,
            emitted: i.jitter_buffer_emitted_count,
            received: i.total_samples_received,
            concealed: i.concealed_samples,
            silent_concealed: i.silent_concealed_samples,
            accelerated: i.removed_samples_for_acceleration,
            decelerated: i.inserted_samples_for_deceleration,
        }
    }

    /// This window's share: `self` minus the same stream's previous sample. A stream not seen
    /// before, or one whose counters went backwards (restarted), counts in full.
    fn since(&self, before: Option<&Inbound>) -> Inbound {
        let Some(b) = before else { return *self };
        if self.emitted < b.emitted
            || self.received < b.received
            || self.buffer_delay_s < b.buffer_delay_s
        {
            return *self;
        }
        Inbound {
            buffer_delay_s: self.buffer_delay_s - b.buffer_delay_s,
            target_delay_s: (self.target_delay_s - b.target_delay_s).max(0.0),
            min_delay_s: (self.min_delay_s - b.min_delay_s).max(0.0),
            emitted: self.emitted - b.emitted,
            received: self.received - b.received,
            concealed: self.concealed.saturating_sub(b.concealed),
            silent_concealed: self.silent_concealed.saturating_sub(b.silent_concealed),
            accelerated: self.accelerated.saturating_sub(b.accelerated),
            decelerated: self.decelerated.saturating_sub(b.decelerated),
        }
    }

    fn add(&mut self, d: &Inbound) {
        self.buffer_delay_s += d.buffer_delay_s;
        self.target_delay_s += d.target_delay_s;
        self.min_delay_s += d.min_delay_s;
        self.emitted += d.emitted;
        self.received += d.received;
        self.concealed += d.concealed;
        self.silent_concealed += d.silent_concealed;
        self.accelerated += d.accelerated;
        self.decelerated += d.decelerated;
    }
}

/// Each stream's counters at the previous sample.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Totals {
    inbound: HashMap<u32, Inbound>,
    packets_sent: u64,
}

/// Opus runs NetEq at 48 kHz: samples per millisecond.
const SAMPLES_PER_MS: f64 = 48.0;

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
    /// Audio NetEq cut (acceleration) or stretched (deceleration) this window, in ms: a standing
    /// excess over target shows up as steady acceleration.
    pub accel_ms: Option<f64>,
    pub decel_ms: Option<f64>,
    /// Share of received audio that was silent concealment (DTX / comfort noise).
    pub silent_pct: Option<f64>,
}

pub fn summarize(
    publisher: &[RtcStats],
    subscriber: &[RtcStats],
    prev: &Totals,
) -> (PathStats, Totals) {
    let mut out = PathStats::default();
    let mut totals = Totals::default();
    let mut window = Inbound::default();
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
                let now = Inbound::of(r);
                window.add(&now.since(prev.inbound.get(&r.stream.ssrc)));
                totals.inbound.insert(r.stream.ssrc, now);
            }
            RtcStats::OutboundRtp(o) if o.stream.kind == "audio" => {
                totals.packets_sent += o.sent.packets_sent;
            }
            _ => {}
        }
    }
    if window.emitted > 0 {
        let per_sample = |s: f64| s / window.emitted as f64 * 1000.0;
        out.buffer_ms = Some(per_sample(window.buffer_delay_s));
        out.target_ms = Some(per_sample(window.target_delay_s));
        out.min_ms = Some(per_sample(window.min_delay_s));
        out.accel_ms = Some(window.accelerated as f64 / SAMPLES_PER_MS);
        out.decel_ms = Some(window.decelerated as f64 / SAMPLES_PER_MS);
    }
    if window.received > 0 {
        let pct = |n: u64| n as f64 / window.received as f64 * 100.0;
        out.concealed_pct = Some(pct(window.concealed));
        out.silent_pct = Some(pct(window.silent_concealed));
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
            "rtt {} jitter {} buffer {} target {} min {} concealed {} lost {} send {} pkt/s accel {} decel {} silent {}",
            ms(self.rtt_ms),
            ms(self.jitter_ms),
            ms(self.buffer_ms),
            ms(self.target_ms),
            ms(self.min_ms),
            pct(self.concealed_pct),
            self.lost,
            pps,
            ms(self.accel_ms),
            ms(self.decel_ms),
            pct(self.silent_pct),
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
            &Totals::default(),
        );
        assert_eq!(s.rtt_ms.map(f64::round), Some(42.0));
    }

    #[test]
    fn jitter_is_the_worst_peer_and_loss_is_summed() {
        let peers = [audio_in(0.004, 1, 0.0, 0), audio_in(0.012, 2, 0.0, 0)];
        let (s, _) = summarize(&[], &peers, &Totals::default());
        assert_eq!(s.jitter_ms.map(f64::round), Some(12.0));
        assert_eq!(s.lost, 3);
    }

    #[test]
    fn buffer_delay_is_the_average_since_the_last_sample() {
        // 4800 new samples that waited 2400 s in total (the delay is summed per sample) → 500 ms each
        let (_, first) = summarize(&[], &[audio_in(0.0, 0, 1.0, 48_000)], &Totals::default());
        let (s, _) = summarize(&[], &[audio_in(0.0, 0, 2401.0, 52_800)], &first);
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
        let (s, _) = summarize(&[], &[], &Totals::default());
        assert_eq!(
            s.to_string(),
            "rtt - jitter - buffer - target - min - concealed - lost 0 send - pkt/s accel - decel - silent -"
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
            &Totals::default(),
        );
        // 4800 samples emitted; target summed 288 s → 60 ms each; minimum 144 s → 30 ms; 480 of
        // 4800 received samples concealed → 10 %; 250 packets in 5 s → 50/s
        let (s, _) = summarize(
            &[audio_out(1250)],
            &[audio_in_full(1.0, 290.0, 145.0, 52_800, 480, 52_800)],
            &first,
        );
        assert_eq!(s.target_ms.map(f64::round), Some(60.0));
        assert_eq!(s.min_ms.map(f64::round), Some(30.0));
        assert_eq!(s.concealed_pct.map(f64::round), Some(10.0));
        assert_eq!(s.send_pps.map(f64::round), Some(50.0));
    }

    fn inbound(ssrc: u32, set: impl FnOnce(&mut InboundRtpStats)) -> RtcStats {
        let mut r = InboundRtpStats::default();
        r.stream.kind = "audio".into();
        r.stream.ssrc = ssrc;
        set(&mut r);
        RtcStats::InboundRtp(r)
    }

    fn jb(ssrc: u32, delay_s: f64, target_s: f64, emitted: u64) -> RtcStats {
        inbound(ssrc, |r| {
            r.inbound.jitter_buffer_delay = delay_s;
            r.inbound.jitter_buffer_target_delay = target_s;
            r.inbound.jitter_buffer_emitted_count = emitted;
        })
    }

    /// R5 (a): a window's buffer is per stream. One peer leaving while another talks must not skew
    /// it (the summed totals read 34 ms here instead of 20).
    #[test]
    fn buffer_is_a_per_stream_window_delta_across_peer_churn() {
        let (_, first) = summarize(
            &[],
            &[jb(1, 10.0, 5.0, 100_000), jb(2, 1.0, 0.5, 10_000)],
            &Totals::default(),
        );
        // peer 1 left; peer 2 emitted 240 000 more samples that waited 20 ms each (aim 10 ms)
        let (s, _) = summarize(&[], &[jb(2, 4801.0, 2400.5, 250_000)], &first);
        assert_eq!(s.buffer_ms.map(f64::round), Some(20.0));
        assert_eq!(s.target_ms.map(f64::round), Some(10.0));
    }

    /// A peer that rejoins arrives as a new stream (new SSRC) whose counters start over: all of
    /// them belong to this window (summed totals "shrank" and the window was skipped).
    #[test]
    fn a_new_stream_counts_from_zero() {
        let (_, first) = summarize(&[], &[jb(1, 100.0, 50.0, 48_000)], &Totals::default());
        let (s, _) = summarize(&[], &[jb(3, 4800.0, 2400.0, 240_000)], &first);
        assert_eq!(s.buffer_ms.map(f64::round), Some(20.0));
    }

    /// R5 (b)/(c): time-stretching and comfort noise per window. Is NetEq draining a standing
    /// excess (acceleration), or is the peer mostly silent (DTX)?
    #[test]
    fn acceleration_and_silent_share_per_window() {
        let at = |emitted, accel, silent, received| {
            inbound(7, |r| {
                r.inbound.jitter_buffer_emitted_count = emitted;
                r.inbound.removed_samples_for_acceleration = accel;
                r.inbound.silent_concealed_samples = silent;
                r.inbound.total_samples_received = received;
            })
        };
        let (_, first) = summarize(&[], &[at(48_000, 0, 0, 48_000)], &Totals::default());
        // 5 s later: 2 400 samples (50 ms at 48 kHz) cut by acceleration; 60 000 of 240 000 silent
        let (s, _) = summarize(&[], &[at(288_000, 2_400, 60_000, 288_000)], &first);
        assert_eq!(s.accel_ms.map(f64::round), Some(50.0));
        assert_eq!(s.decel_ms.map(f64::round), Some(0.0));
        assert_eq!(s.silent_pct.map(f64::round), Some(25.0));
    }
}
