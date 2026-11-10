//! libopus's decoder, for soundboard sounds (#143). Not a new dependency: libwebrtc (linked for
//! calls through livekit's `webrtc-sys`) carries libopus and exports its C API on Linux and Windows,
//! so these declarations link against that copy. It decodes a 6 s sound in ~4 ms where the
//! pure-Rust `opus-decoder` it replaces took ~800 ms (measured on the same file, optimised), and it
//! is the decoder WebRTC already runs on every peer's packets (fuzzed upstream by OSS-Fuzz).
//!
//! If a future libwebrtc build stops exporting these, linking fails (it can't misbehave quietly).

use std::ffi::{c_int, c_void};
use std::ptr::NonNull;

unsafe extern "C" {
    fn opus_decoder_create(fs: i32, channels: c_int, error: *mut c_int) -> *mut c_void;
    fn opus_decode_float(
        st: *mut c_void,
        data: *const u8,
        len: i32,
        pcm: *mut f32,
        frame_size: c_int,
        decode_fec: c_int,
    ) -> c_int;
    fn opus_decoder_destroy(st: *mut c_void);
}

/// One Opus packet is at most 120 ms: the most samples per channel one decode gives.
pub const MAX_PACKET_SAMPLES: usize = 5760;

/// A 48 kHz libopus decoder for a mono or stereo stream.
pub struct Decoder {
    st: NonNull<c_void>,
    channels: usize,
    /// Samples per channel of the last packet decoded (20 ms before any).
    last: usize,
}

impl Decoder {
    /// None unless `channels` is 1 or 2 (all an Ogg Opus sound may have, `parse_ogg_opus`).
    pub fn new(channels: usize) -> Option<Self> {
        if !(1..=2).contains(&channels) {
            return None;
        }
        let mut err: c_int = 0;
        // SAFETY: plain arguments and a valid out-pointer; a null result (with `err` set) is
        // handled below.
        let st = unsafe { opus_decoder_create(48_000, channels as c_int, &mut err) };
        let st = NonNull::new(st).filter(|_| err == 0)?;
        Some(Self {
            st,
            channels,
            last: 960,
        })
    }

    /// Decode one packet into `pcm` (interleaved). Returns the samples per channel written, or
    /// None for a packet libopus refuses. `pcm` must hold [`MAX_PACKET_SAMPLES`] per channel.
    /// An empty packet is a lost one: libopus conceals it (as the Ogg Opus spec has it).
    pub fn decode(&mut self, packet: &[u8], pcm: &mut [f32]) -> Option<usize> {
        assert!(
            pcm.len() / self.channels >= MAX_PACKET_SAMPLES,
            "room for the longest packet"
        );
        let len = i32::try_from(packet.len()).ok()?;
        // libopus conceals as many samples as it's given room for: a lost packet is taken to be
        // as long as the last one, so a run of empty packets can't make 120 ms each
        let (data, frame) = if packet.is_empty() {
            (std::ptr::null(), self.last)
        } else {
            (packet.as_ptr(), MAX_PACKET_SAMPLES)
        };
        // SAFETY: `st` is a live decoder made by `opus_decoder_create` and used only through
        // `&mut self`; `data` points at `len` readable bytes (or is null for a lost packet);
        // `pcm` has room for at least `frame` samples per channel (asserted above), which is the
        // `frame_size` passed, so libopus writes within it.
        let n = unsafe {
            opus_decode_float(
                self.st.as_ptr(),
                data,
                len,
                pcm.as_mut_ptr(),
                frame as c_int,
                0,
            )
        };
        let n = usize::try_from(n).ok().filter(|&n| n <= frame)?;
        if n > 0 {
            self.last = n;
        }
        Some(n)
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        // SAFETY: made by `opus_decoder_create`, destroyed once, here.
        unsafe { opus_decoder_destroy(self.st.as_ptr()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_odd_channel_counts_and_garbage_without_crashing() {
        assert!(Decoder::new(0).is_none());
        assert!(Decoder::new(3).is_none());
        let mut d = Decoder::new(2).unwrap();
        let mut pcm = vec![0.0; MAX_PACKET_SAMPLES * 2];
        // a TOC claiming a code-3 packet with no frame count byte: invalid
        assert!(d.decode(&[0x03], &mut pcm).is_none());
        // a lost packet before anything was decoded: concealed (silence), not an error
        assert!(d.decode(&[], &mut pcm).is_some());
    }
}
