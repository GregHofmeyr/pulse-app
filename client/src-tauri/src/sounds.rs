//! UI sounds (join/leave/mute…), played by the Rust core through cpal — NOT through the webview:
//! WebKitGTK plays <audio> via GStreamer, and without its audio-sink plugin the web process aborts.

use cpal::traits::{DeviceTrait, StreamTrait};

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

/// Silence before the message ping: an idle (suspended) output, typically Bluetooth, needs a few
/// hundred ms to wake, and would swallow the short ping otherwise.
pub const LEAD_IN_MS: u32 = 450;

pub fn load(name: &str) -> Option<Sound> {
    let mut s = bytes(name).and_then(decode_wav)?;
    if name == "message" {
        let lead = (s.rate as usize * LEAD_IN_MS as usize) / 1000;
        s.samples.splice(0..0, std::iter::repeat_n(0.0, lead));
    }
    Some(s)
}

const VOLUME: f32 = 0.6;

/// The last device looked up by name: finding one by name enumerates every device (~20 ms on
/// ALSA), per sound. The system default isn't cached: it can change under us.
pub struct NameCache<D>(std::sync::Mutex<Option<(String, D)>>);

impl<D> Default for NameCache<D> {
    fn default() -> Self {
        Self(std::sync::Mutex::new(None))
    }
}

impl<D: Clone> NameCache<D> {
    pub fn get(&self, name: Option<&str>, find: impl Fn(Option<&str>) -> Option<D>) -> Option<D> {
        let Some(name) = name else { return find(None) };
        let mut c = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((n, d)) = c.as_ref()
            && n == name
        {
            return Some(d.clone());
        }
        let d = find(Some(name))?;
        *c = Some((name.to_string(), d.clone()));
        Some(d)
    }

    /// The cached device failed (unplugged): look it up again next time.
    pub fn forget(&self) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

static DEVICES: std::sync::LazyLock<NameCache<cpal::Device>> =
    std::sync::LazyLock::new(NameCache::default);

/// Fire-and-forget on a short-lived thread (cpal streams are !Send). Failures are silent: sounds are a nicety.
/// On `output` (a device name from settings; missing or unplugged = the system default).
pub fn play(name: &str, output: Option<String>) {
    let Some(sound) = load(name) else { return };
    let _ = std::thread::Builder::new()
        .name("pulse-sound".into())
        .spawn(move || {
            use crate::voice::devices::find_device;
            // cache only the device actually asked for: a missing one falls back to the default
            // (uncached), so it's picked up again once it's back
            let exact = |n: Option<&str>| {
                find_device(false, n)
                    .ok()
                    .filter(|d| n.is_none() || d.name().ok().as_deref() == n)
            };
            let Some(dev) = DEVICES
                .get(output.as_deref(), exact)
                .or_else(|| find_device(false, None).ok())
            else {
                return;
            };
            let Ok(cfg) = dev.default_output_config() else {
                DEVICES.forget();
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
            let stream = crate::voice::devices::build_output(&dev, &cfg, move |d: &mut [f32]| {
                for f in d.chunks_mut(ch) {
                    f.fill(next().unwrap_or(0.0));
                }
            });
            let Ok(stream) = stream else {
                DEVICES.forget();
                return;
            };
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
    fn a_named_device_is_looked_up_once_until_forgotten() {
        let c: NameCache<u32> = NameCache::default();
        let lookups = std::cell::Cell::new(0);
        let find = |_: Option<&str>| {
            lookups.set(lookups.get() + 1);
            Some(7)
        };
        assert_eq!(c.get(Some("Arctis"), find), Some(7));
        assert_eq!(c.get(Some("Arctis"), find), Some(7));
        assert_eq!(lookups.get(), 1, "cached");
        assert_eq!(c.get(Some("Speakers"), find), Some(7));
        assert_eq!(lookups.get(), 2, "another device: looked up");
        c.forget();
        c.get(Some("Speakers"), find);
        assert_eq!(
            lookups.get(),
            3,
            "forgotten after a failure: looked up again"
        );
        c.get(None, find);
        c.get(None, find);
        assert_eq!(lookups.get(), 5, "the default can move: never cached");
    }

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

    /// The message ping often plays to an idle output: a sleeping Bluetooth device takes a few hundred
    /// ms to wake and swallows anything shorter, so it starts with silence (other sounds play while
    /// voice keeps the device awake).
    #[test]
    fn message_sound_starts_with_wake_up_silence() {
        let m = load("message").unwrap();
        let lead = (m.rate as usize * LEAD_IN_MS as usize) / 1000;
        assert!(
            m.samples[..lead].iter().all(|v| *v == 0.0),
            "lead-in must be silent"
        );
        assert!(
            m.samples[lead..].iter().any(|v| v.abs() > 0.1),
            "the ping follows"
        );
        let j = load("join").unwrap();
        assert!(
            j.samples[..200].iter().any(|v| v.abs() > 0.0001) || j.samples.len() < lead,
            "join has no lead-in"
        );
    }

    #[test]
    fn rejects_non_wav() {
        assert!(decode_wav(b"not a wav file at all, definitely not").is_none());
    }
}
