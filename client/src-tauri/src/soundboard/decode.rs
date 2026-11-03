//! Ogg Opus → 48 kHz mono f32, in pure Rust. 48 kHz is the voice mixer's rate, so no resampling.

use std::sync::Arc;

use opus_decoder::OpusDecoder;
use pulse_protocol::sound::{OggOpus, parse_ogg_opus};

/// One Opus packet is at most 120 ms.
const MAX_PACKET_SAMPLES: usize = 5760;

const BAD_FILE: &str = "That sound file couldn't be decoded.";

pub fn decode(bytes: &[u8]) -> Result<Arc<[f32]>, String> {
    let ogg = parse_ogg_opus(bytes).map_err(|e| e.to_string())?;
    let mut out = decode_untrimmed(&ogg)?;
    // the start-up samples the encoder asked to skip, then cut to the length the file's last
    // granule position gives (the final packet is padded)
    let skip = (ogg.pre_skip as usize).min(out.len());
    out.drain(..skip);
    out.truncate(ogg.duration_ms as usize * 48);
    Ok(out.into())
}

/// [`decode`] on a blocking thread: up to 6 s of Opus is real CPU work, kept off the async runtime.
pub async fn decode_blocking(bytes: Vec<u8>) -> Result<Arc<[f32]>, String> {
    tokio::task::spawn_blocking(move || decode(&bytes))
        .await
        .map_err(|_| BAD_FILE.to_string())?
}

/// The packets decoded and downmixed, pre-skip and padding still in. Stops once the file's
/// stated length is reached: the server checks only the last granule, so a hostile file's
/// packets could hold far more audio than that, and every listener would decode it all.
fn decode_untrimmed(ogg: &OggOpus) -> Result<Vec<f32>, String> {
    let ch = ogg.channels as usize;
    let mut dec = OpusDecoder::new(48_000, ch).map_err(|_| BAD_FILE.to_string())?;
    let mut pcm = vec![0f32; MAX_PACKET_SAMPLES * ch];
    let want = ogg.pre_skip as usize + ogg.duration_ms as usize * 48;
    // room for the last packet's overshoot, so the buffer never grows
    let mut out: Vec<f32> = Vec::with_capacity(want + MAX_PACKET_SAMPLES);
    for packet in &ogg.packets {
        if out.len() >= want {
            break;
        }
        let n = dec
            .decode_float(packet, &mut pcm, false)
            .map_err(|_| BAD_FILE.to_string())?;
        for frame in pcm[..n * ch].chunks_exact(ch) {
            out.push(frame.iter().sum::<f32>() / ch as f32);
        }
    }
    Ok(out)
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

    /// Ogg's CRC-32 (polynomial 0x04c11db7, no reflection), to write test pages.
    fn crc(data: &[u8]) -> u32 {
        let mut crc = 0u32;
        for &b in data {
            crc ^= u32::from(b) << 24;
            for _ in 0..8 {
                crc = if crc & 0x8000_0000 != 0 {
                    (crc << 1) ^ 0x04c1_1db7
                } else {
                    crc << 1
                };
            }
        }
        crc
    }

    fn page(header_type: u8, granule: i64, seq: u32, packets: &[&[u8]]) -> Vec<u8> {
        let mut lacing = Vec::new();
        for p in packets {
            lacing.extend(std::iter::repeat_n(255u8, p.len() / 255));
            lacing.push((p.len() % 255) as u8);
        }
        let mut out = b"OggS".to_vec();
        out.extend_from_slice(&[0, header_type]);
        out.extend_from_slice(&granule.to_le_bytes());
        out.extend_from_slice(&7u32.to_le_bytes());
        out.extend_from_slice(&seq.to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.push(lacing.len() as u8);
        out.extend_from_slice(&lacing);
        for p in packets {
            out.extend_from_slice(p);
        }
        let c = crc(&out);
        out[22..26].copy_from_slice(&c.to_le_bytes());
        out
    }

    /// `ogg`'s packets rewritten under a last granule that claims only `ms` of audio.
    fn claiming(ogg: &[u8], ms: u32) -> Vec<u8> {
        let o = parse_ogg_opus(ogg).unwrap();
        let mut head = b"OpusHead".to_vec();
        head.extend_from_slice(&[1, o.channels]);
        head.extend_from_slice(&o.pre_skip.to_le_bytes());
        head.extend_from_slice(&48_000u32.to_le_bytes());
        head.extend_from_slice(&[0, 0, 0]);
        let tags = [&b"OpusTags"[..], &[0; 8]].concat();
        let mut out = page(2, 0, 0, &[&head]);
        out.extend(page(0, 0, 1, &[&tags]));
        let packets: Vec<&[u8]> = o.packets.iter().map(|p| &p[..]).collect();
        let granule = i64::from(o.pre_skip) + i64::from(ms) * 48;
        out.extend(page(4, granule, 2, &packets));
        out
    }

    #[test]
    fn decoding_stops_once_the_stated_length_is_reached() {
        let Some(ogg) = sine_ogg() else { return };
        // a second of packets that says it is 200 ms: only ~200 ms is worth decoding
        let short = claiming(&ogg, 200);
        let o = parse_ogg_opus(&short).unwrap();
        assert_eq!(o.duration_ms, 200);
        let pcm = decode_untrimmed(&o).unwrap();
        let cap = o.pre_skip as usize + 200 * 48;
        assert!(
            pcm.len() < cap + MAX_PACKET_SAMPLES,
            "decoded {} samples for a cap of {cap}",
            pcm.len()
        );
        assert!(pcm.capacity() <= cap + MAX_PACKET_SAMPLES);
        assert_eq!(decode(&short).unwrap().len(), 200 * 48);
    }

    #[test]
    fn decode_refuses_garbage() {
        assert!(decode(b"this is not an ogg file at all").is_err());
        assert!(decode(&[]).is_err());
    }
}
