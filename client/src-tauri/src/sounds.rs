//! UI sounds (join/leave/mute…), played by the Rust core through cpal — NOT through the webview:
//! WebKitGTK plays <audio> via GStreamer, and without its audio-sink plugin the web process aborts.

use cpal::traits::{DeviceTrait, StreamTrait};

pub struct Sound {
    pub rate: u32,
    pub samples: Vec<f32>,
}

/// Every set × event is embedded from `sounds/<set>/<name>.wav`. `SETS` and `NAMES` come from the
/// same invocation as the lookup table, so the lists and the table cannot drift.
macro_rules! sound_table {
    (sets: [$($set:literal),+ $(,)?]; names: $names:tt) => {
        pub const SETS: &[&str] = &[$($set),+];

        sound_table!(@names $names);

        fn bytes(set: &str, name: &str) -> Option<&'static [u8]> {
            sound_table!(@set set, name; $($set),+; $names)
        }
    };
    (@names [$($name:literal),+ $(,)?]) => {
        /// The events, in the order the settings list shows them.
        pub const NAMES: &[&str] = &[$($name),+];
    };
    (@set $s:ident, $n:ident; $($set:literal),+; $names:tt) => {
        match $s {
            $($set => sound_table!(@name $set, $n; $names),)+
            _ => None,
        }
    };
    (@name $set:literal, $n:ident; [$($name:literal),+ $(,)?]) => {
        match $n {
            $($name => Some(include_bytes!(concat!("../sounds/", $set, "/", $name, ".wav")) as &[u8]),)+
            _ => None,
        }
    };
}

sound_table!(
    sets: [
        "classic",
        "twostep-bright",
        "twostep-warm",
        "twostep-deep",
        "minimal-soft",
        "minimal-wood",
        "minimal-glass",
        "goofy",
    ];
    names: [
        "join", "leave", "mute", "unmute", "deafen", "undeafen", "muted_talk", "message", "live",
        "live_end", "badge", "wheel_tick", "wheel_result",
    ]
);

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

/// The incoming-call ringtones you can pick in Settings (#50, #174), one cycle each, generated like
/// the sets (`scripts/gen_sounds.py`); [`ring`] loops the chosen one while a call rings. They don't
/// belong to a sound set. Classic is the original `ring.wav`.
pub const RINGTONES: &[(&str, &[u8])] = &[
    ("classic", include_bytes!("../ringtone/ring.wav")),
    ("marimba", include_bytes!("../ringtone/marimba.wav")),
    ("chiptune", include_bytes!("../ringtone/chiptune.wav")),
    ("phone", include_bytes!("../ringtone/phone.wav")),
];

pub const DEFAULT_RINGTONE: &str = "classic";

/// A ringtone's file; an unknown id is [`DEFAULT_RINGTONE`].
pub fn ringtone(id: &str) -> &'static [u8] {
    RINGTONES
        .iter()
        .find(|(i, _)| *i == id)
        .unwrap_or(&RINGTONES[0])
        .1
}

/// One cycle of a ringtone (an unknown id is Classic).
pub fn load_ringtone(id: &str) -> Option<Sound> {
    decode_wav(ringtone(id))
}

/// An unknown set plays as `DEFAULT_SET`; an unknown name is `None`. `ringing` is one cycle of the
/// default ringtone, whatever the set ([`play_ringtone_once`] plays a chosen one).
pub fn load(set: &str, name: &str) -> Option<Sound> {
    if name == "ringing" {
        return load_ringtone(DEFAULT_RINGTONE);
    }
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

/// The volume mixer's linear gain for a sound (section × item) from a command argument: missing or
/// NaN is full, the rest is clamped to 0..=`max_gain()` (+12 dB).
pub fn gain(volume: Option<f32>) -> f32 {
    match volume {
        Some(v) if !v.is_nan() => v.clamp(0.0, crate::voice::mixer::max_gain()),
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

/// Fire-and-forget on a short-lived thread (cpal streams are !Send). A failure only logs a line: sounds are a nicety.
/// On `output` (a device name from settings; missing or unplugged = the system default).
/// `volume` (0..=`max_gain()`, clamped) scales the fixed gain; 0 plays nothing.
pub fn play(set: &str, name: &str, volume: f32, output: Option<String>) {
    let volume = gain(Some(volume));
    if volume <= 0.0 {
        return;
    }
    let Some(sound) = load(set, name) else {
        tracing::warn!(name = %crate::logging::one_line(name, 32), "sound: no such sound");
        return;
    };
    spawn_play(
        sound,
        VOLUME * volume,
        output,
        crate::logging::one_line(name, 32),
        None,
    );
}

/// One cycle of ringtone `id` (an unknown id is Classic), like [`play`]: the mixer's Ringing ▶
/// and the ringtone picker's preview (#174).
pub fn play_ringtone_once(id: &str, volume: f32, output: Option<String>) {
    let volume = gain(Some(volume));
    if volume <= 0.0 {
        return;
    }
    if let Some(sound) = load_ringtone(id) {
        spawn_play(sound, VOLUME * volume, output, "ringing".to_string(), None);
    }
}

/// Play a soundboard sample on the output device when no call is running (in a call it goes
/// through the mixer instead). `volume` is the final linear gain, without the UI-sound factor.
pub fn play_samples(sound: Sound, volume: f32, output: Option<String>) {
    if volume.is_nan() || volume <= 0.0 {
        return;
    }
    spawn_play(sound, volume, output, "soundboard".to_string(), None);
}

/// [`play_samples`] that ends early once `stop` is set (the soundboard trimmer's pause).
pub fn play_samples_stoppable(
    sound: Sound,
    volume: f32,
    output: Option<String>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    if volume.is_nan() || volume <= 0.0 {
        return;
    }
    spawn_play(sound, volume, output, "soundboard".to_string(), Some(stop));
}

type StopFlag = Option<std::sync::Arc<std::sync::atomic::AtomicBool>>;

fn stopped(stop: &StopFlag) -> bool {
    stop.as_ref()
        .is_some_and(|s| s.load(std::sync::atomic::Ordering::Relaxed))
}

/// The device lookup and fallback shared by UI sounds and soundboard samples; `volume` is final.
fn spawn_play(sound: Sound, volume: f32, output: Option<String>, name: String, stop: StopFlag) {
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
                && play_on(&dev, &sound, volume, &stop)
            {
                return;
            }
            // the cached device failed (unplugged): forget it, and play on the default instead
            DEVICES.forget();
            let played = output.is_some()
                && find_device(false, None).is_ok_and(|dev| play_on(&dev, &sound, volume, &stop));
            if !played {
                // asked for (the `sound` line) but not heard: tells the two apart in a report
                tracing::warn!(name, "sound: no output device would play it");
            }
        });
}

/// One output sample at the final gain `volume`, soft-limited like the call mixer (the volume mixer
/// can boost up to +12 dB). Below the knee it is exactly `x * volume`: UI sounds at <= 100 % peak
/// under it and play as they always did.
fn shape(x: f32, volume: f32) -> f32 {
    crate::voice::mixer::soft_limit(x * volume)
}

/// Play `sound` on `dev` to the end (blocking) at the final linear gain `volume`. False if the device wouldn't open.
fn play_on(dev: &cpal::Device, sound: &Sound, volume: f32, stop: &StopFlag) -> bool {
    let Ok(cfg) = dev.default_output_config() else {
        return false;
    };
    let (rate, ch) = (cfg.sample_rate(), cfg.channels() as usize);
    let step = sound.rate as f64 / rate as f64;
    let total = (sound.samples.len() as f64 / step) as usize;
    let samples = sound.samples.clone();
    let mut pos = 0usize;
    let halt = stop.clone();
    let mut next = move || -> Option<f32> {
        if pos >= total || stopped(&halt) {
            return None;
        }
        let src = pos as f64 * step;
        let (j, frac) = (src.floor() as usize, src.fract() as f32);
        let a = samples[j.min(samples.len() - 1)];
        let b = samples[(j + 1).min(samples.len() - 1)];
        pos += 1;
        Some(shape(a + (b - a) * frac, volume))
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
    // sound length + a little tail for the device buffer, or until stopped
    let end = std::time::Instant::now()
        + std::time::Duration::from_millis((total as u64 * 1000 / rate as u64) + 150);
    while std::time::Instant::now() < end && !stopped(stop) {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    true
}

/// The incoming-call ringtone (#50): one of [`RINGTONES`] on loop through the chosen output,
/// until [`stop`]. One at a time: starting again replaces it.
pub mod ring {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use cpal::traits::{DeviceTrait, StreamTrait};

    /// A ring longer than this stops by itself: the UI ends it at 30 s, so this only matters if
    /// its stop never came.
    const MAX: Duration = Duration::from_secs(45);

    static CURRENT: Mutex<Option<Arc<AtomicBool>>> = Mutex::new(None);

    /// Ring ringtone `id` (unknown = Classic) at the final linear gain `volume` (0 rings nothing)
    /// on `output` (a device name from settings; missing or unplugged = the system default).
    pub fn start(id: &str, volume: f32, output: Option<String>) {
        stop();
        if volume <= 0.0 || volume.is_nan() {
            return;
        }
        let Some(sound) = super::load_ringtone(id) else {
            return;
        };
        let flag = Arc::new(AtomicBool::new(false));
        *CURRENT.lock().unwrap_or_else(|e| e.into_inner()) = Some(flag.clone());
        let _ = std::thread::Builder::new()
            .name("pulse-ring".into())
            .spawn(move || {
                use crate::voice::devices::find_device;
                let named = output
                    .as_deref()
                    .and_then(|n| find_device(false, Some(n)).ok());
                let played = named.is_some_and(|d| play(&d, &sound, volume, &flag))
                    || find_device(false, None).is_ok_and(|d| play(&d, &sound, volume, &flag));
                if !played {
                    tracing::warn!("ringtone: no output device would play it");
                }
            });
    }

    /// Stop the ring, if one is playing; whether one was.
    pub fn stop() -> bool {
        match CURRENT.lock().unwrap_or_else(|e| e.into_inner()).take() {
            Some(flag) => {
                flag.store(true, Ordering::Relaxed);
                true
            }
            None => false,
        }
    }

    /// `cycle` over and over, one sample per call, until `stop` is set.
    pub fn looping(cycle: Vec<f32>, stop: Arc<AtomicBool>) -> impl FnMut() -> Option<f32> {
        let mut pos = 0usize;
        move || {
            if stop.load(Ordering::Relaxed) || cycle.is_empty() {
                return None;
            }
            let v = cycle[pos];
            pos = (pos + 1) % cycle.len();
            Some(v)
        }
    }

    /// `sound` at `rate`, by linear interpolation (once, before looping).
    fn resample(sound: &super::Sound, rate: u32) -> Vec<f32> {
        let step = sound.rate as f64 / rate as f64;
        let n = (sound.samples.len() as f64 / step) as usize;
        let s = &sound.samples;
        (0..n)
            .map(|i| {
                let src = i as f64 * step;
                let (j, frac) = (src.floor() as usize, src.fract() as f32);
                let a = s[j.min(s.len() - 1)];
                let b = s[(j + 1).min(s.len() - 1)];
                a + (b - a) * frac
            })
            .collect()
    }

    /// Ring on `dev` until stopped or [`MAX`] (blocking). False if the device wouldn't open.
    fn play(dev: &cpal::Device, sound: &super::Sound, volume: f32, stop: &Arc<AtomicBool>) -> bool {
        let Ok(cfg) = dev.default_output_config() else {
            return false;
        };
        let ch = cfg.channels() as usize;
        let mut next = looping(resample(sound, cfg.sample_rate()), stop.clone());
        let gain = super::VOLUME * volume;
        let stream = crate::voice::devices::build_output(dev, &cfg, move |d: &mut [f32], _| {
            for f in d.chunks_mut(ch) {
                f.fill(next().map_or(0.0, |x| super::shape(x, gain)));
            }
        });
        let Ok(stream) = stream else { return false };
        if stream.play().is_err() {
            return false;
        }
        let end = Instant::now() + MAX;
        while Instant::now() < end && !stop.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(20));
        }
        true
    }
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
    fn every_sound_is_soft_limited_and_quiet_audio_is_untouched() {
        use crate::voice::mixer::soft_limit;
        // boosted by the mixer (up to +12 dB): bounded like the call mixer, not hard-clipped
        assert_eq!(shape(0.9, 2.0), soft_limit(1.8));
        assert!(shape(0.9, 4.0) <= 1.0);
        assert!(shape(-0.9, 4.0) >= -1.0);
        // below the knee (every UI sound at <= 100 %: they peak under 0.6) nothing changes
        assert_eq!(shape(0.3, 1.0), 0.3);
        assert_eq!(shape(0.5, 0.4), 0.2);
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

    /// (channels, sample rate, bits per sample) from the `fmt ` chunk, found by walking the chunks
    /// (it need not sit at a fixed offset).
    fn wav_format(b: &[u8]) -> Option<(u16, u32, u16)> {
        if b.len() < 12 || &b[0..4] != b"RIFF" || &b[8..12] != b"WAVE" {
            return None;
        }
        let mut i = 12;
        while i + 8 <= b.len() {
            let len = u32::from_le_bytes(b[i + 4..i + 8].try_into().ok()?) as usize;
            let body = b.get(i + 8..i + 8 + len)?;
            if &b[i..i + 4] == b"fmt " && len >= 16 {
                return Some((
                    u16::from_le_bytes(body[2..4].try_into().ok()?),
                    u32::from_le_bytes(body[4..8].try_into().ok()?),
                    u16::from_le_bytes(body[14..16].try_into().ok()?),
                ));
            }
            i += 8 + len + (len & 1);
        }
        None
    }

    #[test]
    fn wav_format_finds_fmt_after_another_chunk() {
        let mut fmt = Vec::new();
        fmt.extend_from_slice(&1u16.to_le_bytes()); // PCM
        fmt.extend_from_slice(&2u16.to_le_bytes()); // channels
        fmt.extend_from_slice(&44_100u32.to_le_bytes());
        fmt.extend_from_slice(&176_400u32.to_le_bytes());
        fmt.extend_from_slice(&4u16.to_le_bytes());
        fmt.extend_from_slice(&16u16.to_le_bytes());
        let mut b = b"RIFF\0\0\0\0WAVE".to_vec();
        b.extend_from_slice(b"LIST");
        b.extend_from_slice(&3u32.to_le_bytes());
        b.extend_from_slice(b"abc\0"); // odd length + pad byte
        b.extend_from_slice(b"fmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&fmt);
        assert_eq!(wav_format(&b), Some((2, 44_100, 16)));
        assert_eq!(wav_format(b"RIFF\0\0\0\0WAVE"), None, "no fmt chunk");
    }

    #[test]
    fn wav_files_are_mono_pcm16_22050() {
        let mut n = 0;
        for (set, files) in sets_on_disk() {
            for f in files {
                let b = std::fs::read(sounds_dir().join(&set).join(&f)).unwrap();
                let (channels, rate, bits) =
                    wav_format(&b).unwrap_or_else(|| panic!("{set}/{f}: no RIFF/WAVE fmt chunk"));
                assert_eq!((channels, bits, rate), (1, 16, 22_050), "{set}/{f} header");
                n += 1;
            }
        }
        assert_eq!(n, SETS.len() * NAMES.len(), "every file checked");
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
        use crate::voice::mixer::max_gain;
        assert_eq!(gain(None), 1.0);
        assert_eq!(gain(Some(2.0)), 2.0, "the mixer can boost");
        assert_eq!(gain(Some(9.0)), max_gain(), "capped at the safe maximum");
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

    /// #219: a join is the sound you most need to notice, so Classic's join and leave (the level
    /// every set but Goofy is normalised to) are its loudest events.
    #[test]
    fn join_and_leave_are_classics_loudest_events() {
        for loud in ["join", "leave"] {
            let l = active_rms(&raw("classic", loud));
            for other in NAMES.iter().filter(|n| !["join", "leave"].contains(n)) {
                let d = db(l, active_rms(&raw("classic", other)));
                assert!(d >= 1.0, "classic/{loud} is only {d:.2} dB over {other}");
            }
        }
    }

    /// #219: a leave is not a join played backwards: it drops into a lower register, so the two
    /// tell apart at a glance (Goofy's are their own sounds).
    #[test]
    fn leave_sits_well_below_join() {
        for set in SETS.iter().filter(|s| **s != "goofy") {
            let (j, l) = (raw(set, "join"), raw(set, "leave"));
            let (j, l) = (zcr(active(&j)), zcr(active(&l)));
            assert!(
                l < j * 0.8,
                "{set}: leave ({l}) should sit well below join ({j})"
            );
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

    /// Runs of 5 ms windows whose peak is at least 15 % of the sound's: the separate strikes.
    fn strikes(s: &[f32]) -> usize {
        let peak = s.iter().fold(0f32, |m, v| m.max(v.abs()));
        let win = 22_050 * 5 / 1000;
        let loud: Vec<bool> = s
            .chunks(win)
            .map(|c| c.iter().fold(0f32, |m, v| m.max(v.abs())) >= 0.15 * peak)
            .collect();
        loud.iter()
            .enumerate()
            .filter(|(i, on)| **on && (*i == 0 || !loud[i - 1]))
            .count()
    }

    /// #252: "you're muted" is a double ding in every set: two strikes, apart.
    #[test]
    fn muted_talk_is_a_double_ding() {
        for set in SETS {
            assert_eq!(strikes(&raw(set, "muted_talk")), 2, "{set}/muted_talk");
        }
        assert_eq!(
            strikes(&raw("classic", "mute")),
            1,
            "the measure: mute is one glide"
        );
    }

    /// It has to be heard over your own voice: pitched well above speech (most of a voice sits
    /// under ~500 Hz), and in Classic within 2 dB of join, the loudest event.
    #[test]
    fn muted_talk_cuts_through_a_voice() {
        for set in SETS {
            // over the strikes only: the silent gap between them isn't a pitch
            let s = raw(set, "muted_talk");
            let peak = s.iter().fold(0f32, |m, v| m.max(v.abs()));
            let sounding: Vec<f32> = s.into_iter().filter(|v| v.abs() >= 0.02 * peak).collect();
            let hz = zcr(&sounding) * 22_050.0 / 2.0;
            assert!(hz >= 800.0, "{set}/muted_talk is around {hz:.0} Hz");
        }
        let d = db(
            active_rms(&raw("classic", "join")),
            active_rms(&raw("classic", "muted_talk")),
        );
        assert!(d <= 2.0, "classic/muted_talk is {d:.2} dB under join");
    }

    /// #271: the picker wheel's result is a chime that climbs (it ends higher than it starts) and
    /// rings out (its last tenth is well under its peak), in every set.
    #[test]
    fn wheel_result_is_a_rising_chime_that_rings_out() {
        for set in SETS {
            let s = raw(set, "wheel_result");
            let a = active(&s);
            let k = a.len() * 2 / 5;
            let (first, last) = (zcr(&a[..k]), zcr(&a[a.len() - k..]));
            assert!(
                last > first * 1.1,
                "{set}/wheel_result should rise: {first} -> {last}"
            );
            let peak = a.iter().fold(0f32, |m, v| m.max(v.abs()));
            let tail = a[a.len() * 9 / 10..]
                .iter()
                .fold(0f32, |m, v| m.max(v.abs()));
            assert!(
                tail < peak * 0.5,
                "{set}/wheel_result should ring out: tail {tail} of {peak}"
            );
        }
    }

    /// Heard with Pulse tabbed out, but not a call event: in Classic 2-4 dB under join.
    #[test]
    fn wheel_result_sits_under_join() {
        let d = db(
            active_rms(&raw("classic", "join")),
            active_rms(&raw("classic", "wheel_result")),
        );
        assert!(
            (2.0..=4.0).contains(&d),
            "classic/wheel_result is {d:.2} dB under join"
        );
    }

    #[test]
    fn rejects_non_wav() {
        assert!(decode_wav(b"not a wav file at all, definitely not").is_none());
    }

    /// The ringtones' level: Classic's join when they were made (#50, #174), before #219 lifted
    /// join and leave above it (`scripts/gen_sounds.py` RING_RMS).
    const RING_RMS: f32 = 0.1936;

    /// Every ringtone (#50, #174) is one cycle, looped while a call rings: our format, a couple of
    /// seconds, ending in a pause (so the loop breathes), at the ring level.
    #[test]
    fn every_ringtone_is_a_loopable_cycle() {
        for (id, bytes) in RINGTONES {
            assert_eq!(wav_format(bytes), Some((1, 22_050, 16)), "{id}");
            let s = decode_wav(bytes).unwrap();
            let secs = s.samples.len() as f32 / s.rate as f32;
            assert!((1.5..=3.0).contains(&secs), "{id}: {secs} s");
            let tail = &s.samples[s.samples.len() - s.rate as usize / 2..];
            assert!(
                tail.iter().all(|v| v.abs() < 0.001),
                "{id}: the last 0.5 s is quiet"
            );
            let d = db(active_rms(&s.samples), RING_RMS);
            assert!(d.abs() <= 1.5, "{id}: {d:.1} dB from the ring level");
            let peak = s.samples.iter().fold(0f32, |m, v| m.max(v.abs()));
            assert!(peak <= 0.98, "{id} peaks at {peak}");
            assert!(bytes.len() < 200_000, "{id}");
        }
    }

    /// A handful to pick from, each its own sound; Classic (the original `ring.wav`) is first and
    /// the default.
    #[test]
    fn the_ringtones_are_distinct_and_classic_is_the_original() {
        let ids: Vec<&str> = RINGTONES.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids, ["classic", "marimba", "chiptune", "phone"]);
        assert_eq!(DEFAULT_RINGTONE, "classic");
        assert_eq!(
            ringtone("classic"),
            include_bytes!("../ringtone/ring.wav") as &[u8]
        );
        for (i, (a, x)) in RINGTONES.iter().enumerate() {
            for (b, y) in &RINGTONES[i + 1..] {
                assert_ne!(x, y, "{a} and {b} are the same file");
            }
        }
        // the files on disk are exactly the table (a new wav can't be left out)
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/ringtone");
        let mut on_disk: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|f| f.unwrap().file_name().into_string().unwrap())
            .collect();
        on_disk.sort();
        assert_eq!(
            on_disk,
            ["chiptune.wav", "marimba.wav", "phone.wav", "ring.wav"]
        );
    }

    #[test]
    fn an_unknown_ringtone_rings_classic() {
        assert_eq!(ringtone("nope"), ringtone("classic"));
        assert_eq!(ringtone(""), ringtone("classic"));
        assert_ne!(ringtone("marimba"), ringtone("classic"));
    }

    /// The mixer's Ringing ▶ and the picker's ▶ play one cycle of the chosen ringtone, whatever
    /// the set.
    #[test]
    fn ringing_plays_one_cycle_of_the_chosen_ringtone() {
        for (id, bytes) in RINGTONES {
            let one = decode_wav(bytes).unwrap();
            assert_eq!(load_ringtone(id).unwrap().samples, one.samples, "{id}");
        }
        let classic = decode_wav(ringtone("classic")).unwrap();
        for set in SETS {
            assert_eq!(
                load(set, "ringing").unwrap().samples,
                classic.samples,
                "{set}"
            );
        }
        assert!(!NAMES.contains(&"ringing"), "not an event of the sets");
    }

    #[test]
    fn a_ring_loops_until_stopped() {
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut next = ring::looping(vec![0.5f32, -0.5, 0.25], stop.clone());
        let got: Vec<f32> = (0..7).map(|_| next().unwrap()).collect();
        assert_eq!(got, [0.5, -0.5, 0.25, 0.5, -0.5, 0.25, 0.5]);
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(next(), None);
    }
}
