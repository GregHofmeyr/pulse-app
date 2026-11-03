//! Ogg Opus → 48 kHz mono f32, in pure Rust. 48 kHz is the voice mixer's rate, so no resampling.

use std::sync::Arc;

use opus_decoder::OpusDecoder;
use pulse_protocol::sound::parse_ogg_opus;

/// One Opus packet is at most 120 ms.
const MAX_PACKET_SAMPLES: usize = 5760;

const BAD_FILE: &str = "That sound file couldn't be decoded.";

pub fn decode(bytes: &[u8]) -> Result<Arc<[f32]>, String> {
    let ogg = parse_ogg_opus(bytes).map_err(|e| e.to_string())?;
    let ch = ogg.channels as usize;
    let mut dec = OpusDecoder::new(48_000, ch).map_err(|_| BAD_FILE.to_string())?;
    let mut pcm = vec![0f32; MAX_PACKET_SAMPLES * ch];
    let mut out: Vec<f32> = Vec::new();
    for packet in &ogg.packets {
        let n = dec
            .decode_float(packet, &mut pcm, false)
            .map_err(|_| BAD_FILE.to_string())?;
        for frame in pcm[..n * ch].chunks_exact(ch) {
            out.push(frame.iter().sum::<f32>() / ch as f32);
        }
    }
    // the start-up samples the encoder asked to skip, then cut to the length the file's last
    // granule position gives (the final packet is padded)
    let skip = (ogg.pre_skip as usize).min(out.len());
    out.drain(..skip);
    out.truncate(ogg.duration_ms as usize * 48);
    Ok(out.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 1 s 440 Hz mono sine encoded by the system ffmpeg; None (with a note) when it's missing.
    fn sine_ogg() -> Option<Vec<u8>> {
        let dir = crate::clips::ffmpeg::dev_override().unwrap_or_else(|| "/usr/bin".into());
        let ffmpeg = dir.join("ffmpeg");
        if !ffmpeg.exists() {
            eprintln!("skipping: no ffmpeg at {}", ffmpeg.display());
            return None;
        }
        let out = std::process::Command::new(ffmpeg)
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=1",
                "-ac",
                "1",
                "-ar",
                "48000",
                "-c:a",
                "libopus",
                "-b:a",
                "64k",
                "-f",
                "ogg",
                "pipe:1",
            ])
            .output()
            .ok()?;
        out.status.success().then_some(out.stdout)
    }

    #[test]
    fn decodes_a_sine_to_the_right_length_and_level() {
        let Some(ogg) = sine_ogg() else { return };
        let s = decode(&ogg).unwrap();
        assert!(
            (s.len() as i64 - 48_000).abs() <= 960,
            "length {} samples",
            s.len()
        );
        let rms = (s.iter().map(|v| v * v).sum::<f32>() / s.len() as f32).sqrt();
        // lavfi's sine has amplitude 1/8, so RMS 0.125 / sqrt(2)
        let want = 0.125 / 2f32.sqrt();
        assert!((rms - want).abs() <= want * 0.2, "rms {rms}, want {want}");
    }

    #[test]
    fn decode_refuses_garbage() {
        assert!(decode(b"this is not an ogg file at all").is_err());
        assert!(decode(&[]).is_err());
    }
}
