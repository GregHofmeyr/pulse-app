//! Ogg Opus → 48 kHz mono f32, in pure Rust. 48 kHz is the voice mixer's rate, so no resampling.

use std::sync::Arc;

use opus_decoder::OpusDecoder;
use pulse_protocol::sound::{OggOpus, parse_ogg_opus};

use super::pcm::Pcm;

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

/// How much audio a streamed decode has ready before playback starts (100 ms): a head start so
/// the audio callback never catches up with the decoder (which runs ~7x faster than real time).
pub const START_SAMPLES: usize = 4800;

/// Decode on a blocking thread, and resolve as soon as [`START_SAMPLES`] are ready (or the whole
/// sound, when it is shorter): the rest keeps decoding into the returned [`Pcm`] while it plays.
/// The whole of a 6 s sound takes ~0.8 s in `opus-decoder`, which is what a first play waited for.
pub async fn decode_streaming(bytes: Vec<u8>) -> Result<Arc<Pcm>, String> {
    let duration_ms = parse_ogg_opus(&bytes)
        .map_err(|e| e.to_string())?
        .duration_ms;
    let pcm = Arc::new(Pcm::with_capacity(duration_ms as usize * 48));
    let (tx, rx) = tokio::sync::oneshot::channel::<Result<(), String>>();
    let out = pcm.clone();
    tokio::task::spawn_blocking(move || {
        // parsed again here: the packets borrow the bytes (parsing is cheap next to decoding)
        let Ok(ogg) = parse_ogg_opus(&bytes) else {
            out.finish();
            let _ = tx.send(Err(BAD_FILE.to_string()));
            return;
        };
        let mut tx = Some(tx);
        // ends the sound even if the decoder panics, so it can't stay "still decoding" for ever
        let _ended = Ended(out.clone());
        let mut skip = ogg.pre_skip as usize;
        let res = decode_packets(&ogg, |mono| {
            // the start-up samples the encoder asked to skip; the capacity cuts the padded end
            let s = skip.min(mono.len());
            skip -= s;
            out.push(&mono[s..]);
            if out.ready() >= START_SAMPLES
                && let Some(tx) = tx.take()
            {
                let _ = tx.send(Ok(()));
            }
            !out.is_full()
        });
        out.finish();
        if let Some(tx) = tx.take() {
            // shorter than the head start, or it failed: say which
            let _ = tx.send(match res {
                Err(e) if out.ready() == 0 => Err(e),
                _ => Ok(()),
            });
        }
    });
    rx.await.map_err(|_| BAD_FILE.to_string())??;
    Ok(pcm)
}

/// Marks a streamed sound finished when dropped (unwinding included).
struct Ended(Arc<Pcm>);

impl Drop for Ended {
    fn drop(&mut self) {
        self.0.finish();
    }
}

/// The packets decoded and downmixed, pre-skip and padding still in. Stops once the file's
/// stated length is reached: the server checks only the last granule, so a hostile file's
/// packets could hold far more audio than that, and every listener would decode it all.
fn decode_untrimmed(ogg: &OggOpus) -> Result<Vec<f32>, String> {
    let want = ogg.pre_skip as usize + ogg.duration_ms as usize * 48;
    // room for the last packet's overshoot, so the buffer never grows
    let mut out: Vec<f32> = Vec::with_capacity(want + MAX_PACKET_SAMPLES);
    decode_packets(ogg, |mono| {
        out.extend_from_slice(mono);
        out.len() < want
    })?;
    Ok(out)
}

/// Decode `ogg`'s packets in order, handing each one's mono samples to `sink`, until it returns
/// false (enough) or the packets run out.
fn decode_packets(ogg: &OggOpus, mut sink: impl FnMut(&[f32]) -> bool) -> Result<(), String> {
    let ch = ogg.channels as usize;
    let mut dec = OpusDecoder::new(48_000, ch).map_err(|_| BAD_FILE.to_string())?;
    let mut pcm = vec![0f32; MAX_PACKET_SAMPLES * ch];
    let mut mono = Vec::with_capacity(MAX_PACKET_SAMPLES);
    for packet in &ogg.packets {
        let n = dec
            .decode_float(packet, &mut pcm, false)
            .map_err(|_| BAD_FILE.to_string())?;
        mono.clear();
        mono.extend(
            pcm[..n * ch]
                .chunks_exact(ch)
                .map(|f| f.iter().sum::<f32>() / ch as f32),
        );
        if !sink(&mono) {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A `secs` long 440 Hz mono sine as Ogg Opus, encoded by the system ffmpeg; None (with a
    /// note) when it's missing.
    pub(crate) fn sine_ogg_secs(secs: u32) -> Option<Vec<u8>> {
        let dir = crate::clips::ffmpeg::dev_override().unwrap_or_else(|| "/usr/bin".into());
        let ffmpeg = dir.join("ffmpeg");
        if !ffmpeg.exists() {
            eprintln!("skipping: no ffmpeg at {}", ffmpeg.display());
            return None;
        }
        let out = std::process::Command::new(ffmpeg)
            .args(["-v", "error", "-f", "lavfi", "-i"])
            .arg(format!("sine=frequency=440:duration={secs}"))
            .args([
                "-ac", "1", "-ar", "48000", "-c:a", "libopus", "-b:a", "64k", "-f", "ogg", "pipe:1",
            ])
            .output()
            .ok()?;
        out.status.success().then_some(out.stdout)
    }

    /// A 1 s sine.
    fn sine_ogg() -> Option<Vec<u8>> {
        sine_ogg_secs(1)
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

    #[tokio::test]
    async fn streaming_starts_early_and_matches_a_full_decode() {
        let Some(ogg) = sine_ogg_secs(6) else { return };
        let t = std::time::Instant::now();
        let pcm = decode_streaming(ogg.clone()).await.unwrap();
        let start_ms = t.elapsed().as_secs_f64() * 1000.0;
        assert!(pcm.ready() >= START_SAMPLES, "{} ready", pcm.ready());
        pcm.wait_done().await;
        let full_ms = t.elapsed().as_secs_f64() * 1000.0;
        eprintln!(
            "soundboard streaming decode (6 s): audio after {start_ms:.1} ms, all after {full_ms:.1} ms"
        );
        assert_eq!(pcm.to_vec(), decode(&ogg).unwrap().to_vec());
        assert!(start_ms < full_ms / 4.0, "it starts long before the end");
    }

    #[tokio::test]
    async fn streaming_stops_at_the_stated_length_and_refuses_garbage() {
        let Some(ogg) = sine_ogg() else { return };
        let pcm = decode_streaming(claiming(&ogg, 200)).await.unwrap();
        pcm.wait_done().await;
        assert_eq!(pcm.to_vec().len(), 200 * 48);
        assert!(decode_streaming(b"not ogg".to_vec()).await.is_err());
    }

    #[test]
    fn decode_refuses_garbage() {
        assert!(decode(b"this is not an ogg file at all").is_err());
        assert!(decode(&[]).is_err());
    }
}
