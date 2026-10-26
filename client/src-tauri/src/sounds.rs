//! UI sounds (join/leave/mute…), played by the Rust core through cpal — NOT through the webview:
//! WebKitGTK plays <audio> via GStreamer, and without its audio-sink plugin the web process aborts.

use cpal::traits::{DeviceTrait, StreamTrait};

pub struct Sound {
    pub rate: u32,
    pub samples: Vec<f32>,
}

/// Every set × event is embedded from `sounds/<set>/<name>.wav`. `SETS` comes from the same
/// invocation as the lookup table, so the list and the table cannot drift.
macro_rules! sound_table {
    ($($set:literal),+ $(,)?) => {
        pub const SETS: &[&str] = &[$($set),+];

        fn bytes(set: &str, name: &str) -> Option<&'static [u8]> {
            sound_table!(@set set, name; $($set),+; [
                "join", "leave", "mute", "unmute", "deafen", "undeafen", "message", "live", "live_end"
            ])
        }
    };
    (@set $s:ident, $n:ident; $($set:literal),+; $names:tt) => {
        match $s {
            $($set => sound_table!(@name $set, $n; $names),)+
            _ => None,
        }
    };
    (@name $set:literal, $n:ident; [$($name:literal),+]) => {
        match $n {
            $($name => Some(include_bytes!(concat!("../sounds/", $set, "/", $name, ".wav")) as &[u8]),)+
            _ => None,
        }
    };
}

sound_table!(
    "classic",
    "twostep-bright",
    "twostep-warm",
    "twostep-deep",
    "minimal-soft",
    "minimal-wood",
    "minimal-glass",
);

/// The events, in the order the settings list shows them.
pub const NAMES: &[&str] = &[
    "join", "leave", "mute", "unmute", "deafen", "undeafen", "message", "live", "live_end",
];

pub const DEFAULT_SET: &str = "classic";

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

/// An unknown set plays as `DEFAULT_SET`; an unknown name is `None`.
pub fn load(set: &str, name: &str) -> Option<Sound> {
    let mut s = bytes(set, name)
        .or_else(|| bytes(DEFAULT_SET, name))
        .and_then(decode_wav)?;
    if name == "message" {
        let lead = (s.rate as usize * LEAD_IN_MS as usize) / 1000;
        s.samples.splice(0..0, std::iter::repeat_n(0.0, lead));
    }
    Some(s)
}

const VOLUME: f32 = 0.6;

/// The user's sound volume (0.0-1.0) from a command argument: missing or NaN is full, the rest is clamped.
pub fn gain(volume: Option<f32>) -> f32 {
    match volume {
        Some(v) if !v.is_nan() => v.clamp(0.0, 1.0),
        _ => 1.0,
    }
}

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
/// `volume` (0.0-1.0, clamped) scales the fixed gain; 0 plays nothing.
pub fn play(set: &str, name: &str, volume: f32, output: Option<String>) {
    let volume = gain(Some(volume));
    if volume <= 0.0 {
        return;
    }
    let Some(sound) = load(set, name) else { return };
    let _ = std::thread::Builder::new()
        .name("pulse-sound".into())
        .spawn(move || {
            use crate::voice::devices::find_device;
            // cache only the device actually asked for: a missing one falls back to the default
            // (uncached), so it's picked up again once it's back
            let exact = |n: Option<&str>| {
                find_device(false, n).ok().filter(|d| {
                    n.is_none() || crate::voice::devices::device_name(d).as_deref() == n
                })
            };
            let chosen = DEVICES
                .get(output.as_deref(), exact)
                .or_else(|| find_device(false, None).ok());
            if let Some(dev) = chosen
                && play_on(&dev, &sound, volume)
            {
                return;
            }
            // the cached device failed (unplugged): forget it, and play on the default instead
            DEVICES.forget();
            if output.is_some()
                && let Ok(dev) = find_device(false, None)
            {
                play_on(&dev, &sound, volume);
            }
        });
}

/// Play `sound` on `dev` to the end (blocking). False if the device wouldn't open.
fn play_on(dev: &cpal::Device, sound: &Sound, volume: f32) -> bool {
    let Ok(cfg) = dev.default_output_config() else {
        return false;
    };
    let (rate, ch) = (cfg.sample_rate(), cfg.channels() as usize);
    let step = sound.rate as f64 / rate as f64;
    let total = (sound.samples.len() as f64 / step) as usize;
    let samples = sound.samples.clone();
    let mut pos = 0usize;
    let mut next = move || -> Option<f32> {
        if pos >= total {
            return None;
        }
        let src = pos as f64 * step;
        let (j, frac) = (src.floor() as usize, src.fract() as f32);
        let a = samples[j.min(samples.len() - 1)];
        let b = samples[(j + 1).min(samples.len() - 1)];
        pos += 1;
        Some((a + (b - a) * frac) * VOLUME * volume)
    };
    let stream = crate::voice::devices::build_output(dev, &cfg, move |d: &mut [f32], _| {
        for f in d.chunks_mut(ch) {
            f.fill(next().unwrap_or(0.0));
        }
    });
    let Ok(stream) = stream else { return false };
    if stream.play().is_err() {
        return false;
    }
    // sound length + a little tail for the device buffer
    std::thread::sleep(std::time::Duration::from_millis(
        (total as u64 * 1000 / rate as u64) + 150,
    ));
    true
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
    fn every_set_has_every_event() {
        for set in SETS {
            for name in NAMES {
                let s = load(set, name).unwrap_or_else(|| panic!("{set}/{name} missing"));
                assert_eq!(s.rate, 22_050, "{set}/{name}");
                assert!(
                    s.samples.iter().any(|v| v.abs() > 0.1),
                    "{set}/{name} silent"
                );
            }
        }
    }

    fn sounds_dir() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("sounds")
    }

    /// (set dir name, sorted file names without extension) for every set on disk.
    fn sets_on_disk() -> Vec<(String, Vec<String>)> {
        let mut sets: Vec<_> = std::fs::read_dir(sounds_dir())
            .unwrap()
            .map(|e| e.unwrap())
            .map(|e| {
                assert!(e.path().is_dir(), "stray file {:?} in sounds/", e.path());
                let mut files: Vec<String> = std::fs::read_dir(e.path())
                    .unwrap()
                    .map(|f| f.unwrap().file_name().into_string().unwrap())
                    .collect();
                files.sort();
                (e.file_name().into_string().unwrap(), files)
            })
            .collect();
        sets.sort();
        sets
    }

    #[test]
    fn wav_files_are_mono_pcm16_22050() {
        for (set, files) in sets_on_disk() {
            for f in files {
                let b = std::fs::read(sounds_dir().join(&set).join(&f)).unwrap();
                assert!(
                    b.len() > 44 && &b[0..4] == b"RIFF" && &b[8..12] == b"WAVE",
                    "{set}/{f}"
                );
                let channels = u16::from_le_bytes(b[22..24].try_into().unwrap());
                let rate = u32::from_le_bytes(b[24..28].try_into().unwrap());
                let bits = u16::from_le_bytes(b[34..36].try_into().unwrap());
                assert_eq!((channels, bits, rate), (1, 16, 22_050), "{set}/{f} header");
            }
        }
    }

    #[test]
    fn every_sound_dir_is_a_set_and_holds_exactly_the_events() {
        let on_disk = sets_on_disk();
        let names: Vec<&str> = on_disk.iter().map(|(s, _)| s.as_str()).collect();
        let mut want: Vec<&str> = SETS.to_vec();
        want.sort();
        assert_eq!(names, want, "sets on disk vs SETS");
        let mut events: Vec<String> = NAMES.iter().map(|n| format!("{n}.wav")).collect();
        events.sort();
        for (set, files) in on_disk {
            assert_eq!(files, events, "files in {set}");
        }
    }

    #[test]
    fn durations_are_within_bounds() {
        for set in SETS {
            let max = if set.starts_with("minimal-") {
                0.2
            } else if *set == "goofy" {
                0.5
            } else {
                0.35 // classic, twostep-*
            };
            for name in NAMES {
                // measure the file itself, not the message lead-in
                let s = bytes(set, name).and_then(decode_wav).unwrap();
                let secs = s.samples.len() as f32 / s.rate as f32;
                assert!(secs >= 0.03, "{set}/{name} is {secs}s, under 0.03s");
                assert!(secs <= max, "{set}/{name} is {secs}s, over {max}s");
            }
        }
    }

    #[test]
    fn sounds_fit_the_size_budget() {
        let total: u64 = sets_on_disk()
            .iter()
            .flat_map(|(s, fs)| fs.iter().map(move |f| sounds_dir().join(s).join(f)))
            .map(|p| std::fs::metadata(p).unwrap().len())
            .sum();
        assert!(total <= 1_200_000, "sounds total {total} bytes");
    }

    #[test]
    fn unknown_set_falls_back_to_classic() {
        let a = load("nope", "join").unwrap();
        let b = load(DEFAULT_SET, "join").unwrap();
        assert_eq!(a.samples, b.samples);
        assert!(load("classic", "nope").is_none());
    }

    #[test]
    fn gain_clamps() {
        assert_eq!(gain(None), 1.0);
        assert_eq!(gain(Some(2.0)), 1.0);
        assert_eq!(gain(Some(-1.0)), 0.0);
        assert_eq!(gain(Some(f32::NAN)), 1.0);
        assert_eq!(gain(Some(0.5)), 0.5);
    }

    #[test]
    fn sounds_ship_once_in_the_binary_not_again_as_webview_assets() {
        // Played by the core (cpal), never by the page: a copy under ui/public would be bundled twice.
        let public = concat!(env!("CARGO_MANIFEST_DIR"), "/../ui/public/sounds");
        assert!(
            !std::path::Path::new(public).exists(),
            "{public} is bundled into the webview assets too"
        );
    }

    /// The message ping often plays to an idle output: a sleeping Bluetooth device takes a few hundred
    /// ms to wake and swallows anything shorter, so it starts with silence (other sounds play while
    /// voice keeps the device awake).
    #[test]
    fn message_sound_starts_with_wake_up_silence() {
        for set in SETS {
            let m = load(set, "message").unwrap();
            let lead = (m.rate as usize * LEAD_IN_MS as usize) / 1000;
            assert!(
                m.samples[..lead].iter().all(|v| *v == 0.0),
                "{set}: lead-in must be silent"
            );
            assert!(
                m.samples[lead..].iter().any(|v| v.abs() > 0.1),
                "{set}: the ping follows"
            );
            let j = load(set, "join").unwrap();
            assert!(
                j.samples[..200].iter().any(|v| v.abs() > 0.0001) || j.samples.len() < lead,
                "{set}: join has no lead-in"
            );
        }
    }

    /// The file's samples, without the message lead-in.
    fn raw(set: &str, name: &str) -> Vec<f32> {
        bytes(set, name).and_then(decode_wav).unwrap().samples
    }

    /// The samples from the first to the last one with |s| >= 2 % of the peak.
    fn active(s: &[f32]) -> &[f32] {
        let peak = s.iter().fold(0f32, |m, v| m.max(v.abs()));
        let on = |v: &&f32| v.abs() >= 0.02 * peak;
        let first = s.iter().position(|v| on(&v)).unwrap();
        let last = s.iter().rposition(|v| on(&v)).unwrap();
        &s[first..=last]
    }

    fn active_rms(s: &[f32]) -> f32 {
        let a = active(s);
        (a.iter().map(|v| v * v).sum::<f32>() / a.len() as f32).sqrt()
    }

    /// Sign changes per sample (exact zeros don't count as a sign).
    fn zcr(s: &[f32]) -> f32 {
        let (mut last, mut n) = (0f32, 0u32);
        for v in s.iter().filter(|v| **v != 0.0) {
            if last != 0.0 && (*v > 0.0) != (last > 0.0) {
                n += 1;
            }
            last = *v;
        }
        n as f32 / s.len() as f32
    }

    fn db(a: f32, b: f32) -> f32 {
        20.0 * (a / b).log10()
    }

    #[test]
    fn loudness_matches_classic() {
        for set in SETS.iter().filter(|s| **s != "classic") {
            // (offset from Classic in dB, tolerance in dB)
            let (offset, tol) = if set.starts_with("minimal-") {
                (-6.0, 1.5)
            } else if *set == "goofy" {
                (0.0, 3.0)
            } else {
                (0.0, 1.5)
            };
            for name in NAMES {
                let s = raw(set, name);
                let delta = db(active_rms(&s), active_rms(&raw("classic", name)));
                assert!(
                    (delta - offset).abs() <= tol,
                    "{set}/{name} is {delta:.2} dB vs Classic, want {offset} +/- {tol}"
                );
                let peak = s.iter().fold(0f32, |m, v| m.max(v.abs()));
                assert!(peak <= 0.98, "{set}/{name} peaks at {peak}");
            }
        }
    }

    #[test]
    fn twostep_rises_for_on_and_falls_for_off() {
        for set in SETS.iter().filter(|s| s.starts_with("twostep-")) {
            for (name, rises) in [
                ("join", true),
                ("unmute", true),
                ("undeafen", true),
                ("live", true),
                ("leave", false),
                ("mute", false),
                ("deafen", false),
                ("live_end", false),
            ] {
                let s = raw(set, name);
                let a = active(&s);
                let k = a.len() * 2 / 5;
                let (first, last) = (zcr(&a[..k]), zcr(&a[a.len() - k..]));
                if rises {
                    assert!(
                        last > first * 1.1,
                        "{set}/{name} should rise: {first} -> {last}"
                    );
                } else {
                    assert!(
                        first > last * 1.1,
                        "{set}/{name} should fall: {first} -> {last}"
                    );
                }
            }
        }
    }

    #[test]
    fn minimal_on_events_are_higher_than_off() {
        for set in SETS.iter().filter(|s| s.starts_with("minimal-")) {
            for (on, off) in [
                ("join", "leave"),
                ("unmute", "mute"),
                ("undeafen", "deafen"),
                ("live", "live_end"),
            ] {
                let (a, b) = (raw(set, on), raw(set, off));
                let (a, b) = (zcr(active(&a)), zcr(active(&b)));
                assert!(
                    a > b * 1.05,
                    "{set}: {on} ({a}) should be above {off} ({b})"
                );
            }
        }
    }

    #[test]
    fn no_two_events_in_a_set_are_identical() {
        for set in SETS {
            for (i, a) in NAMES.iter().enumerate() {
                for b in &NAMES[i + 1..] {
                    assert_ne!(
                        bytes(set, a),
                        bytes(set, b),
                        "{set}: {a} and {b} are identical"
                    );
                }
            }
        }
    }

    #[test]
    fn rejects_non_wav() {
        assert!(decode_wav(b"not a wav file at all, definitely not").is_none());
    }
}
