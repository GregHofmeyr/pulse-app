//! UI sounds (join/leave/mute…), played by the Rust core through cpal — NOT through the webview:
//! WebKitGTK plays <audio> via GStreamer, and without its audio-sink plugin the web process aborts.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

pub struct Sound {
    pub rate: u32,
    pub samples: Vec<f32>,
}

fn bytes(name: &str) -> Option<&'static [u8]> {
    Some(match name {
        "join" => include_bytes!("../../ui/public/sounds/join.wav"),
        "leave" => include_bytes!("../../ui/public/sounds/leave.wav"),
        "mute" => include_bytes!("../../ui/public/sounds/mute.wav"),
        "unmute" => include_bytes!("../../ui/public/sounds/unmute.wav"),
        "deafen" => include_bytes!("../../ui/public/sounds/deafen.wav"),
        "undeafen" => include_bytes!("../../ui/public/sounds/undeafen.wav"),
        "message" => include_bytes!("../../ui/public/sounds/message.wav"),
        _ => return None,
    })
}

/// Minimal PCM16 WAV reader (our own generated files: RIFF/WAVE, fmt, data; mono or stereo).
pub fn decode_wav(b: &[u8]) -> Option<Sound> {
    if b.len() < 12 || &b[0..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        return None;
    }
    let (mut rate, mut channels, mut bits) = (0u32, 0u16, 0u16);
    let mut i = 12;
    while i + 8 <= b.len() {
        let id = &b[i..i + 4];
        let len = u32::from_le_bytes(b[i + 4..i + 8].try_into().ok()?) as usize;
        let body = b.get(i + 8..i + 8 + len)?;
        match id {
            b"fmt " if len >= 16 => {
                channels = u16::from_le_bytes(body[2..4].try_into().ok()?);
                rate = u32::from_le_bytes(body[4..8].try_into().ok()?);
                bits = u16::from_le_bytes(body[14..16].try_into().ok()?);
            }
            b"data" => {
                if bits != 16 || channels == 0 || rate == 0 {
                    return None;
                }
                let samples = body
                    .chunks_exact(2 * channels as usize)
                    .map(|f| {
                        f.as_chunks::<2>()
                            .0
                            .iter()
                            .map(|c| i16::from_le_bytes(*c) as f32 / i16::MAX as f32)
                            .sum::<f32>()
                            / channels as f32
                    })
                    .collect();
                return Some(Sound { rate, samples });
            }
            _ => {}
        }
        i += 8 + len + (len & 1);
    }
    None
}

pub fn load(name: &str) -> Option<Sound> {
    bytes(name).and_then(decode_wav)
}

const VOLUME: f32 = 0.6;

/// Fire-and-forget on a short-lived thread (cpal streams are !Send). Failures are silent: sounds are a nicety.
pub fn play(name: &str) {
    let Some(sound) = load(name) else { return };
    let _ = std::thread::Builder::new()
        .name("pulse-sound".into())
        .spawn(move || {
            let Some(dev) = cpal::default_host().default_output_device() else {
                return;
            };
            let Ok(cfg) = dev.default_output_config() else {
                return;
            };
            let (rate, ch) = (cfg.sample_rate().0, cfg.channels() as usize);
            let step = sound.rate as f64 / rate as f64;
            let total = (sound.samples.len() as f64 / step) as usize;
            let mut pos = 0usize;
            let next = move || -> Option<f32> {
                if pos >= total {
                    return None;
                }
                let src = pos as f64 * step;
                let (j, frac) = (src.floor() as usize, src.fract() as f32);
                let a = sound.samples[j.min(sound.samples.len() - 1)];
                let b = sound.samples[(j + 1).min(sound.samples.len() - 1)];
                pos += 1;
                Some((a + (b - a) * frac) * VOLUME)
            };
            let mut next = next;
            let stream = match cfg.sample_format() {
                cpal::SampleFormat::F32 => dev.build_output_stream(
                    &cfg.config(),
                    move |d: &mut [f32], _| {
                        for f in d.chunks_mut(ch) {
                            f.fill(next().unwrap_or(0.0));
                        }
                    },
                    |_| {},
                    None,
                ),
                cpal::SampleFormat::I16 => dev.build_output_stream(
                    &cfg.config(),
                    move |d: &mut [i16], _| {
                        for f in d.chunks_mut(ch) {
                            f.fill((next().unwrap_or(0.0) * i16::MAX as f32) as i16);
                        }
                    },
                    |_| {},
                    None,
                ),
                _ => return,
            };
            let Ok(stream) = stream else { return };
            if stream.play().is_ok() {
                // sound length + a little tail for the device buffer
                std::thread::sleep(std::time::Duration::from_millis(
                    (total as u64 * 1000 / rate as u64) + 150,
                ));
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_bundled_sound_decodes() {
        for name in [
            "join", "leave", "mute", "unmute", "deafen", "undeafen", "message",
        ] {
            let s = load(name).unwrap_or_else(|| panic!("{name} missing"));
            assert_eq!(s.rate, 22_050);
            assert!(s.samples.len() > 1000, "{name} too short");
            assert!(s.samples.iter().any(|v| v.abs() > 0.1), "{name} silent");
        }
        assert!(load("nope").is_none());
    }

    #[test]
    fn rejects_non_wav() {
        assert!(decode_wav(b"not a wav file at all, definitely not").is_none());
    }
}
