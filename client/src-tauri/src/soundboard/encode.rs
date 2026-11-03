//! Turning a user's audio file into a sound: a waveform for the trim UI, then trim + loudness
//! normalise + Ogg Opus with ffmpeg.

use std::ffi::OsString;
use std::path::Path;
use std::time::Duration;

use pulse_protocol::sound::{SoundFileError, parse_ogg_opus};
use serde::Serialize;

use crate::clips::ffmpeg::{command, output};

const BAD_FILE: &str = "Pulse couldn't read that audio file.";
const TOO_LONG: &str = "Sounds can be at most 6 seconds.";
/// Source files above this are refused before ffmpeg ever sees them.
const MAX_SOURCE_BYTES: u64 = 50 * 1024 * 1024;
const MAX_SPAN_MS: u32 = 6000;
const PEAKS: usize = 200;
/// How much of a long source the trim UI can show.
const WAVEFORM_MAX_SECS: u32 = 600;
const WAVEFORM_RATE: u32 = 8000;
const TIMEOUT: Duration = Duration::from_secs(60);
/// Loudness target for a soundboard clip: loud enough to cut through voice, with headroom.
const LOUDNORM: &str = "loudnorm=I=-16:TP=-1.5:LRA=11";

#[derive(Debug, Clone, Serialize)]
pub struct Waveform {
    pub duration_ms: u32,
    /// 200 buckets of peak level, 0..=1.
    pub peaks: Vec<f32>,
}

fn check_source(file: &Path) -> Result<(), String> {
    let meta = std::fs::metadata(file).map_err(|_| BAD_FILE.to_string())?;
    if !meta.is_file() || meta.len() > MAX_SOURCE_BYTES {
        return Err(BAD_FILE.into());
    }
    Ok(())
}

/// Run ffmpeg to completion, within the time limit. None on any failure.
async fn run(ffmpeg: &Path, args: Vec<OsString>) -> Option<std::process::Output> {
    let out = tokio::time::timeout(TIMEOUT, output(command(ffmpeg, args)))
        .await
        .ok()?
        .ok()?;
    out.status.success().then_some(out)
}

fn os(args: &[&str]) -> Vec<OsString> {
    args.iter().map(OsString::from).collect()
}

/// The first audio stream only (cover art and subtitles are ignored), trimmed on the input side.
fn input_args(file: &Path, start_ms: u32, dur_ms: Option<u32>) -> Vec<OsString> {
    let mut a = os(&["-v", "info", "-nostdin"]);
    if start_ms > 0 {
        a.extend(os(&["-ss", &format!("{:.3}", start_ms as f64 / 1000.0)]));
    }
    if let Some(d) = dur_ms {
        a.extend(os(&["-t", &format!("{:.3}", d as f64 / 1000.0)]));
    }
    a.extend([OsString::from("-i"), file.into()]);
    a.extend(os(&["-map", "0:a:0", "-vn", "-sn", "-dn"]));
    a
}

/// Peaks and length of the first 10 minutes of `file`, for drawing the trim UI.
pub async fn waveform(ffmpeg: &Path, file: &Path) -> Result<Waveform, String> {
    check_source(file)?;
    let mut args = input_args(file, 0, None);
    args.extend(os(&["-t", &WAVEFORM_MAX_SECS.to_string()]));
    args.extend(os(&["-ac", "1", "-ar", "8000", "-f", "s16le", "pipe:1"]));
    let out = run(ffmpeg, args).await.ok_or(BAD_FILE)?;
    let samples: Vec<i16> = out
        .stdout
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| i16::from_le_bytes(*b))
        .collect();
    if samples.is_empty() {
        return Err(BAD_FILE.into());
    }
    let n = samples.len();
    let mut peaks = vec![0f32; PEAKS];
    for (i, s) in samples.iter().enumerate() {
        let bucket = i * PEAKS / n;
        let level = (f32::from(*s) / 32768.0).abs().min(1.0);
        if level > peaks[bucket] {
            peaks[bucket] = level;
        }
    }
    Ok(Waveform {
        duration_ms: (n as u64 * 1000 / u64::from(WAVEFORM_RATE)) as u32,
        peaks,
    })
}

/// `from_ms..to_ms` of `file` (at most 6 s) as 48 kHz mono f32, as it is (no loudness change):
/// what the trimmer's play button plays, to find the bit you want in a long source.
pub async fn decode_span(
    ffmpeg: &Path,
    file: &Path,
    from_ms: u32,
    to_ms: u32,
) -> Result<Vec<f32>, String> {
    check_source(file)?;
    let dur = to_ms.saturating_sub(from_ms).min(MAX_SPAN_MS);
    if dur == 0 {
        return Err(BAD_FILE.into());
    }
    let mut args = input_args(file, from_ms, Some(dur));
    args.extend(os(&["-ac", "1", "-ar", "48000", "-f", "f32le", "pipe:1"]));
    let out = run(ffmpeg, args).await.ok_or(BAD_FILE)?;
    let pcm: Vec<f32> = out
        .stdout
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect();
    if pcm.is_empty() {
        return Err(BAD_FILE.into());
    }
    Ok(pcm)
}

/// What loudnorm's first pass measured.
struct Measured {
    i: f64,
    tp: f64,
    lra: f64,
    thresh: f64,
    offset: f64,
}

/// Parse the JSON block loudnorm prints at the end of stderr. None for silence (`-inf`) or
/// anything unreadable, which makes the caller skip normalising.
fn parse_measured(stderr: &str) -> Option<Measured> {
    let start = stderr.rfind('{')?;
    let end = start + stderr[start..].find('}')?;
    let v: serde_json::Value = serde_json::from_str(&stderr[start..=end]).ok()?;
    let num = |k: &str| -> Option<f64> {
        let n: f64 = v.get(k)?.as_str()?.trim().parse().ok()?;
        n.is_finite().then_some(n)
    };
    Some(Measured {
        i: num("input_i")?,
        tp: num("input_tp")?,
        lra: num("input_lra")?,
        thresh: num("input_thresh")?,
        offset: num("target_offset")?,
    })
}

/// Trim `file` to `start_ms..end_ms` (at most 6 s: a longer span is cut short), loudness-normalise
/// it in two passes and encode mono 48 kHz Ogg Opus.
pub async fn encode(
    ffmpeg: &Path,
    file: &Path,
    start_ms: u32,
    end_ms: u32,
) -> Result<Vec<u8>, String> {
    check_source(file)?;
    let dur = end_ms.saturating_sub(start_ms).min(MAX_SPAN_MS);
    if dur == 0 {
        return Err(BAD_FILE.into());
    }

    // pass 1: measure
    let mut args = input_args(file, start_ms, Some(dur));
    args.extend(os(&[
        "-af",
        &format!("{LOUDNORM}:print_format=json"),
        "-f",
        "null",
        "-",
    ]));
    let first = run(ffmpeg, args).await.ok_or(BAD_FILE)?;
    let measured = parse_measured(&String::from_utf8_lossy(&first.stderr));

    // pass 2: normalise with the measured values (linear), or plain for silence
    let mut args = input_args(file, start_ms, Some(dur));
    if let Some(m) = measured {
        args.extend(os(&[
            "-af",
            &format!(
                "{LOUDNORM}:measured_I={:.2}:measured_TP={:.2}:measured_LRA={:.2}:measured_thresh={:.2}:offset={:.2}:linear=true",
                m.i, m.tp, m.lra, m.thresh, m.offset
            ),
        ]));
    }
    args.extend(os(&[
        "-ac", "1", "-ar", "48000", "-c:a", "libopus", "-b:a", "64k", "-f", "ogg", "pipe:1",
    ]));
    let bytes = run(ffmpeg, args).await.ok_or(BAD_FILE)?.stdout;
    match parse_ogg_opus(&bytes) {
        Ok(_) => Ok(bytes),
        Err(SoundFileError::TooLong) => Err(TOO_LONG.into()),
        Err(_) => Err(BAD_FILE.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::soundboard::decode::decode;
    use pulse_protocol::sound::parse_ogg_opus;
    use std::path::PathBuf;

    fn ffmpeg() -> Option<PathBuf> {
        let dir = crate::clips::ffmpeg::dev_override().unwrap_or_else(|| "/usr/bin".into());
        let p = dir.join("ffmpeg");
        if !p.exists() {
            eprintln!("skipping: no ffmpeg at {}", p.display());
            return None;
        }
        Some(p)
    }

    /// Generate a fixture with ffmpeg: `lavfi` input description, extra output args, file name.
    fn fixture(ff: &Path, dir: &Path, name: &str, lavfi: &str, extra: &[&str]) -> PathBuf {
        let out = dir.join(name);
        let st = std::process::Command::new(ff)
            .args(["-v", "error", "-f", "lavfi", "-i", lavfi])
            .args(extra)
            .arg(&out)
            .status()
            .unwrap();
        assert!(st.success());
        out
    }

    fn rms(s: &[f32]) -> f32 {
        (s.iter().map(|v| v * v).sum::<f32>() / s.len() as f32).sqrt()
    }

    #[tokio::test]
    async fn waveform_has_200_peaks_and_the_duration() {
        let Some(ff) = ffmpeg() else { return };
        let d = tempfile::tempdir().unwrap();
        let f = fixture(&ff, d.path(), "a.wav", "sine=frequency=440:duration=3", &[]);
        let w = waveform(&ff, &f).await.unwrap();
        assert_eq!(w.peaks.len(), 200);
        assert!(
            (w.duration_ms as i64 - 3000).abs() <= 50,
            "{}",
            w.duration_ms
        );
        assert!(w.peaks.iter().all(|p| (0.0..=1.0).contains(p)));
        assert!(w.peaks.iter().any(|p| *p > 0.05));
    }

    #[tokio::test]
    async fn encode_trims_to_the_span() {
        let Some(ff) = ffmpeg() else { return };
        let d = tempfile::tempdir().unwrap();
        let f = fixture(
            &ff,
            d.path(),
            "a.wav",
            "sine=frequency=440:duration=10",
            &[],
        );
        let ogg = encode(&ff, &f, 2000, 5000).await.unwrap();
        let ms = parse_ogg_opus(&ogg).unwrap().duration_ms;
        assert!((ms as i64 - 3000).abs() <= 40, "{ms}");
    }

    #[tokio::test]
    async fn decode_span_is_the_span() {
        let Some(ff) = ffmpeg() else { return };
        let d = tempfile::tempdir().unwrap();
        let f = fixture(
            &ff,
            d.path(),
            "a.wav",
            "sine=frequency=440:duration=10",
            &["-ar", "44100", "-ac", "2"],
        );
        let pcm = decode_span(&ff, &f, 2000, 3500).await.unwrap();
        let ms = pcm.len() as i64 / 48;
        assert!((ms - 1500).abs() <= 20, "{ms} ms");
        assert!(rms(&pcm) > 0.05, "it is the sine, as 48 kHz mono");
        // at most 6 s, and an empty span is refused
        let long = decode_span(&ff, &f, 0, 9000).await.unwrap();
        assert!((long.len() as i64 / 48 - 6000).abs() <= 20);
        assert!(decode_span(&ff, &f, 3000, 3000).await.is_err());
    }

    #[tokio::test]
    async fn encode_clamps_to_six_seconds() {
        let Some(ff) = ffmpeg() else { return };
        let d = tempfile::tempdir().unwrap();
        let f = fixture(
            &ff,
            d.path(),
            "a.wav",
            "sine=frequency=440:duration=10",
            &[],
        );
        let ogg = encode(&ff, &f, 0, 9000).await.unwrap();
        let ms = parse_ogg_opus(&ogg).unwrap().duration_ms;
        assert!((ms as i64 - 6000).abs() <= 40, "{ms}");
    }

    #[tokio::test]
    async fn a_quiet_clip_comes_out_louder() {
        let Some(ff) = ffmpeg() else { return };
        let d = tempfile::tempdir().unwrap();
        // 0.01 amplitude = -40 dBFS
        let f = fixture(
            &ff,
            d.path(),
            "q.wav",
            "sine=frequency=440:duration=3",
            &["-af", "volume=-32dB"],
        );
        let input = {
            let o = std::process::Command::new(&ff)
                .args(["-v", "error", "-i"])
                .arg(&f)
                .args(["-ac", "1", "-ar", "48000", "-f", "f32le", "pipe:1"])
                .output()
                .unwrap();
            let v: Vec<f32> = o
                .stdout
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| f32::from_le_bytes(*b))
                .collect();
            rms(&v)
        };
        let ogg = encode(&ff, &f, 0, 3000).await.unwrap();
        let out = rms(&decode(&ogg).unwrap());
        assert!(out >= input * 10.0, "in {input}, out {out}");
    }

    #[tokio::test]
    async fn silence_encodes() {
        let Some(ff) = ffmpeg() else { return };
        let d = tempfile::tempdir().unwrap();
        let f = fixture(
            &ff,
            d.path(),
            "s.wav",
            "anullsrc=r=44100:cl=mono",
            &["-t", "2"],
        );
        let ogg = encode(&ff, &f, 0, 2000).await.unwrap();
        assert!(parse_ogg_opus(&ogg).unwrap().duration_ms > 1900);
    }

    #[tokio::test]
    async fn stereo_44k_becomes_mono_48k() {
        let Some(ff) = ffmpeg() else { return };
        let d = tempfile::tempdir().unwrap();
        let f = fixture(
            &ff,
            d.path(),
            "st.wav",
            "sine=frequency=440:duration=2:sample_rate=44100",
            &["-ac", "2"],
        );
        let ogg = encode(&ff, &f, 0, 2000).await.unwrap();
        assert_eq!(parse_ogg_opus(&ogg).unwrap().channels, 1);
    }

    #[tokio::test]
    async fn not_audio_is_refused() {
        let Some(ff) = ffmpeg() else { return };
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("x.mp3");
        std::fs::write(&f, b"definitely not audio").unwrap();
        assert_eq!(
            encode(&ff, &f, 0, 1000).await.unwrap_err(),
            "Pulse couldn't read that audio file."
        );
        assert_eq!(
            waveform(&ff, &f).await.unwrap_err(),
            "Pulse couldn't read that audio file."
        );
    }
}
