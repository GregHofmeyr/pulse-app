//! An Opus encode → decode round trip for the mic test's "Hear yourself as others do" (#659): your processed mic
//! through the same codec, at the same settings, as the mic track a call sends.
//!
//! No new dependency: libwebrtc (linked through livekit's `webrtc-sys`) carries libopus and exports its C API on
//! Linux and Windows, so the encoder is declared here against that copy (the very encoder a call uses), and the
//! decoder is the soundboard's (`soundboard::opus`, #143), which links the same way.

use std::ffi::c_int;

use crate::soundboard::opus::{Decoder, MAX_PACKET_SAMPLES};

/// 48 kHz mono, as the mic track (#587).
const RATE: i32 = 48_000;
/// The packet length a call sends: libwebrtc's default 20 ms Opus frame (the SDP's `minptime=10` only says what a
/// receiver accepts; `latency::OPUS_MS` counts the same 20 ms).
pub const FRAME_MS: usize = 20;
const FRAME: usize = RATE as usize / 1000 * FRAME_MS;
/// Opus's own ceiling on one packet; far above anything 128 kbps makes in 20 ms (320 bytes).
const MAX_PACKET: usize = 1275;
/// The decoded samples waiting to go out: never more than the primed frame plus one decoded frame.
const OUT_CAP: usize = 4 * FRAME;

// libopus's API (opus.h / opus_defines.h).
const OPUS_OK: c_int = 0;
const OPUS_APPLICATION_VOIP: c_int = 2048;
const OPUS_SET_BITRATE_REQUEST: c_int = 4002;
const OPUS_SET_COMPLEXITY_REQUEST: c_int = 4010;
const OPUS_SET_INBAND_FEC_REQUEST: c_int = 4012;
const OPUS_SET_PACKET_LOSS_PERC_REQUEST: c_int = 4014;
const OPUS_SET_DTX_REQUEST: c_int = 4016;
const OPUS_SET_SIGNAL_REQUEST: c_int = 4024;
const OPUS_GET_LOOKAHEAD_REQUEST: c_int = 4027;
const OPUS_SIGNAL_VOICE: c_int = 3001;

#[repr(C)]
struct OpusEncoder {
    _private: [u8; 0],
}

unsafe extern "C" {
    fn opus_encoder_create(
        fs: i32,
        channels: c_int,
        application: c_int,
        error: *mut c_int,
    ) -> *mut OpusEncoder;
    fn opus_encoder_ctl(st: *mut OpusEncoder, request: c_int, ...) -> c_int;
    fn opus_encode(
        st: *mut OpusEncoder,
        pcm: *const i16,
        frame_size: c_int,
        data: *mut u8,
        max_data_bytes: i32,
    ) -> i32;
    fn opus_encoder_destroy(st: *mut OpusEncoder);
}

/// One encoder and one decoder, back to back. Fixed size: nothing is allocated after `new`, and no lock is taken.
pub struct OpusRoundTrip {
    enc: *mut OpusEncoder,
    dec: Decoder,
    /// Input gathered towards the next whole frame.
    pending: [i16; FRAME],
    filled: usize,
    packet: [u8; MAX_PACKET],
    decoded: [f32; MAX_PACKET_SAMPLES],
    /// Decoded audio not yet handed out (a ring: `start` .. `start + len`).
    out: [i16; OUT_CAP],
    start: usize,
    len: usize,
    lookahead: usize,
}

// The codec states are owned and only touched through `&mut self`.
unsafe impl Send for OpusRoundTrip {}

impl OpusRoundTrip {
    /// An encoder set up as a call's mic track: mono voice (#587), `bitrate` bps, in-band FEC on (the SDP's
    /// `useinbandfec=1`), libwebrtc's desktop complexity (9). DTX is left off: here it would only drop silence the
    /// test plays anyway.
    pub fn new(bitrate: i32) -> Result<Self, String> {
        let mut err: c_int = 0;
        // SAFETY: plain libopus calls; every pointer is checked before use and freed in `Drop`.
        unsafe {
            let enc = opus_encoder_create(RATE, 1, OPUS_APPLICATION_VOIP, &mut err);
            if enc.is_null() || err != OPUS_OK {
                return Err(format!("opus encoder: error {err}"));
            }
            let Some(dec) = Decoder::new(1) else {
                opus_encoder_destroy(enc);
                return Err("opus decoder: couldn't create one".into());
            };
            let mut rt = Self {
                enc,
                dec,
                pending: [0; FRAME],
                filled: 0,
                packet: [0; MAX_PACKET],
                decoded: [0.0; MAX_PACKET_SAMPLES],
                // one frame of silence ahead: what a sender waits to fill a packet, and enough that a frame is
                // always decoded before its samples are due, however the input is cut up
                out: [0; OUT_CAP],
                start: 0,
                len: FRAME,
                lookahead: 0,
            };
            for (req, v) in [
                (OPUS_SET_BITRATE_REQUEST, bitrate),
                (OPUS_SET_COMPLEXITY_REQUEST, 9),
                (OPUS_SET_INBAND_FEC_REQUEST, 1),
                (OPUS_SET_PACKET_LOSS_PERC_REQUEST, 0),
                (OPUS_SET_DTX_REQUEST, 0),
                (OPUS_SET_SIGNAL_REQUEST, OPUS_SIGNAL_VOICE),
            ] {
                let r = opus_encoder_ctl(rt.enc, req, v as c_int);
                if r != OPUS_OK {
                    return Err(format!("opus encoder setting {req}: error {r}"));
                }
            }
            let mut la: c_int = 0;
            if opus_encoder_ctl(rt.enc, OPUS_GET_LOOKAHEAD_REQUEST, &mut la as *mut c_int)
                == OPUS_OK
            {
                rt.lookahead = la.max(0) as usize;
            }
            Ok(rt)
        }
    }

    /// The decoded output's delay behind the input, in samples: one packet's wait plus the codec's lookahead
    /// (26.5 ms at 20 ms frames), what `latency::OPUS_MS` counts for a call's send side.
    pub fn delay_samples(&self) -> usize {
        self.lookahead + FRAME
    }

    /// `samples` (48 kHz mono, any length) through the codec in place: what comes out is the decoded audio,
    /// `delay_samples` behind (silence at first).
    pub fn process(&mut self, samples: &mut [i16]) {
        // a long block at once (not the 10 ms the processor sends) goes through a frame at a time
        for chunk in samples.chunks_mut(FRAME) {
            self.process_chunk(chunk);
        }
    }

    /// Up to one frame: all of it in (encoding a frame the moment it's whole), then as much out, from the oldest
    /// decoded audio (the frame primed in `new` keeps enough there; silence if it ever weren't).
    fn process_chunk(&mut self, chunk: &mut [i16]) {
        for &s in chunk.iter() {
            self.pending[self.filled] = s;
            self.filled += 1;
            if self.filled == FRAME {
                self.filled = 0;
                self.round_trip();
            }
        }
        let silence = chunk.len().saturating_sub(self.len);
        let (pad, rest) = chunk.split_at_mut(silence);
        pad.fill(0);
        for s in rest {
            *s = self.out[self.start];
            self.start = (self.start + 1) % OUT_CAP;
            self.len -= 1;
        }
    }

    fn round_trip(&mut self) {
        // SAFETY: `enc` is live (made in `new`, used only through `&mut self`); `pending` holds the `FRAME` samples
        // passed and `packet` has room for the `MAX_PACKET` bytes allowed.
        let bytes = unsafe {
            opus_encode(
                self.enc,
                self.pending.as_ptr(),
                FRAME as c_int,
                self.packet.as_mut_ptr(),
                MAX_PACKET as i32,
            )
        };
        // a failed encode decodes as a lost packet (concealment), as a receiver would
        let packet = &self.packet[..usize::try_from(bytes).unwrap_or(0)];
        let n = self.dec.decode(packet, &mut self.decoded).unwrap_or(0);
        for &v in &self.decoded[..n] {
            if self.len == OUT_CAP {
                // can't happen with `process`'s framing; drop the oldest rather than write past the ring
                self.start = (self.start + 1) % OUT_CAP;
                self.len -= 1;
            }
            self.out[(self.start + self.len) % OUT_CAP] =
                (v * 32768.0).round().clamp(-32768.0, 32767.0) as i16;
            self.len += 1;
        }
    }
}

impl Drop for OpusRoundTrip {
    fn drop(&mut self) {
        // SAFETY: created in `new`, destroyed once here.
        unsafe {
            opus_encoder_destroy(self.enc);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const N: usize = RATE as usize;

    fn speechlike(n: usize) -> Vec<i16> {
        // a voice gliding 120 → 220 Hz with harmonics, at about −12 dBFS: never periodic, so only the true lag
        // lines it up
        let mut phase = 0f32;
        (0..n)
            .map(|i| {
                let f0 = 120.0 + 100.0 * i as f32 / n as f32;
                phase += std::f32::consts::TAU * f0 / RATE as f32;
                let v: f32 = (1..=8).map(|h| (phase * h as f32).sin() / h as f32).sum();
                (v * 0.12 * 32767.0) as i16
            })
            .collect()
    }

    /// The best-matching lag of `out` behind `inp` (within `max`), and the SNR (dB) at that lag, past the codec's
    /// first 100 ms.
    fn aligned_snr(inp: &[i16], out: &[i16], max: usize) -> (usize, f32) {
        let skip = N / 10;
        let score = |lag: usize| -> (f64, f64) {
            let (mut sig, mut err) = (0f64, 0f64);
            for i in skip..inp.len() - max {
                let (a, b) = (inp[i] as f64, out[i + lag] as f64);
                sig += a * a;
                err += (a - b) * (a - b);
            }
            (sig, err)
        };
        let lag = (0..max)
            .min_by(|a, b| score(*a).1.partial_cmp(&score(*b).1).unwrap())
            .unwrap();
        let (sig, err) = score(lag);
        (lag, (10.0 * (sig / err.max(1.0)).log10()) as f32)
    }

    /// The round trip keeps the voice: lined up by its delay, what comes out is close to what went in.
    #[test]
    fn the_round_trip_keeps_the_signal() {
        let mut rt = OpusRoundTrip::new(64_000).expect("libopus from libwebrtc");
        let inp = speechlike(N);
        let mut out = Vec::with_capacity(N);
        for chunk in inp.chunks(480) {
            let mut f = chunk.to_vec();
            rt.process(&mut f);
            assert_eq!(f.len(), chunk.len(), "as much out as in, every frame");
            out.extend_from_slice(&f);
        }
        let (lag, snr) = aligned_snr(&inp, &out, 2_000);
        // the decoder's filters shift it by a few samples either way
        assert!(
            lag.abs_diff(rt.delay_samples()) <= 16,
            "delayed by what it says: {lag} vs {}",
            rt.delay_samples()
        );
        // Opus is perceptual, not waveform-exact: ~8 dB here at 64 kbps; silence or a wrong lag scores 0 or less
        assert!(snr > 6.0, "SNR {snr:.1} dB");
    }

    /// Frames that don't divide the codec's frame (a device's 441, say) still come out whole and in order.
    #[test]
    fn odd_frame_sizes_come_out_whole() {
        let mut rt = OpusRoundTrip::new(64_000).unwrap();
        let inp = speechlike(N);
        let mut out = Vec::new();
        for chunk in inp.chunks(441) {
            let mut f = chunk.to_vec();
            rt.process(&mut f);
            out.extend_from_slice(&f);
        }
        assert_eq!(out.len(), inp.len());
        let (lag, snr) = aligned_snr(&inp, &out, 2_000);
        assert!(lag.abs_diff(rt.delay_samples()) <= 16, "{lag}");
        assert!(snr > 6.0, "SNR {snr:.1} dB");
    }

    /// The delay the page tells people about ("about 25 ms") stays true.
    #[test]
    fn the_added_delay_is_about_twenty_five_ms() {
        let rt = OpusRoundTrip::new(64_000).unwrap();
        let ms = rt.delay_samples() as f32 * 1000.0 / RATE as f32;
        assert!((22.0..=30.0).contains(&ms), "{ms} ms");
    }
}
