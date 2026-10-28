//! The export: the ffmpeg command for a clip's kept pieces at a preset, the GPU-to-CPU encoder fallback, and
//! the progress read from ffmpeg. The editor's preview encode uses the same fallback.

use std::future::Future;
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};

use super::ffmpeg::{self, FfmpegPaths};
use super::probe::Probe;
use super::progress::{self, ProgressLine};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Preset {
    Light,
    Sharp,
}

impl Preset {
    /// The cap on the short side of the picture.
    fn short_side(self) -> u32 {
        match self {
            Preset::Light => 720,
            Preset::Sharp => 1080,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoder {
    Nvenc,
    Amf,
    Qsv,
    Mf,
    OpenH264,
    X264,
}

/// Hardware first. The shipped LGPL build has no x264, so the last only matters on a dev machine.
pub const CHAIN: [Encoder; 6] = [
    Encoder::Nvenc,
    Encoder::Amf,
    Encoder::Qsv,
    Encoder::Mf,
    Encoder::OpenH264,
    Encoder::X264,
];

impl Encoder {
    pub fn name(self) -> &'static str {
        match self {
            Encoder::Nvenc => "h264_nvenc",
            Encoder::Amf => "h264_amf",
            Encoder::Qsv => "h264_qsv",
            Encoder::Mf => "h264_mf",
            Encoder::OpenH264 => "libopenh264",
            Encoder::X264 => "libx264",
        }
    }
}

/// A kept piece of the source, in source milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    pub start_ms: u64,
    pub end_ms: u64,
}

/// The size the export has: the short side capped for the preset with the aspect kept, never above the
/// source, both sides rounded down to even (H.264 4:2:0 needs it).
pub fn target_size(w: u32, h: u32, preset: Preset) -> (u32, u32) {
    let cap = u64::from(preset.short_side());
    let (w64, h64) = (u64::from(w.max(1)), u64::from(h.max(1)));
    let (tw, th) = if w64.min(h64) <= cap {
        (w64, h64)
    } else if w64 <= h64 {
        (cap, h64 * cap / w64)
    } else {
        (w64 * cap / h64, cap)
    };
    let even = |v: u64, max: u64| (v.min(max) & !1).max(2) as u32;
    (even(tw, w64), even(th, h64))
}

/// `Some(60)` when the source runs faster than 60 fps (with a little slack for 60000/1001-style rates).
pub fn target_fps(src_fps: f64) -> Option<u32> {
    (src_fps > 60.5).then_some(60)
}

/// `-b:v`, `-maxrate` and `-bufsize` of the preset.
pub fn video_bitrate(preset: Preset) -> (&'static str, &'static str, &'static str) {
    match preset {
        Preset::Light => ("4M", "6M", "8M"),
        Preset::Sharp => ("8M", "12M", "16M"),
    }
}

/// The start of every ffmpeg command we run: quiet, no stdin, machine-readable progress on stdout.
pub fn head() -> Vec<String> {
    "-hide_banner -nostdin -loglevel error -progress pipe:1 -nostats"
        .split(' ')
        .map(str::to_owned)
        .collect()
}

/// Milliseconds as ffmpeg seconds with 3 decimals.
fn secs(ms: u64) -> String {
    format!("{}.{:03}", ms / 1000, ms % 1000)
}

fn labels(prefix: &str, n: usize) -> String {
    (0..n).map(|i| format!("[{prefix}{i}]")).collect()
}

/// The `-filter_complex` graph and the `-map` target of the audio (`None`: the source has none).
/// `kept` is non-empty; its times are relative to the first start, which the input seek has applied.
fn graph(p: &Probe, kept: &[Span], preset: Preset) -> (String, Option<String>) {
    let k = kept.len();
    let n = p.audio.len();
    let base = kept[0].start_ms;
    let mut parts: Vec<String> = Vec::new();

    // the audio to cut: the one track, or the mix of all of them
    let mut audio_in = None;
    if n == 1 {
        audio_in = Some("[0:a:0]".to_owned());
    } else if n > 1 {
        let inputs: String = (0..n).map(|i| format!("[0:a:{i}]")).collect();
        let out = if k == 1 { "[ac]" } else { "[am]" };
        parts.push(format!("{inputs}amix=inputs={n}:normalize=0{out}"));
        audio_in = Some(out.to_owned());
    }

    let video_in = if k == 1 {
        "[0:v:0]"
    } else {
        parts.push(format!("[0:v:0]split={k}{}", labels("vs", k)));
        if let Some(a) = &audio_in {
            parts.push(format!("{a}asplit={k}{}", labels("as", k)));
        }
        for (i, s) in kept.iter().enumerate() {
            let (a, b) = (secs(s.start_ms - base), secs(s.end_ms - base));
            parts.push(format!(
                "[vs{i}]trim=start={a}:end={b},setpts=PTS-STARTPTS[v{i}]"
            ));
            if audio_in.is_some() {
                parts.push(format!(
                    "[as{i}]atrim=start={a}:end={b},asetpts=PTS-STARTPTS[a{i}]"
                ));
            }
        }
        let inputs: String = (0..k)
            .map(|i| match audio_in {
                Some(_) => format!("[v{i}][a{i}]"),
                None => format!("[v{i}]"),
            })
            .collect();
        let (a_flag, a_out) = if audio_in.is_some() {
            (1, "[ac]")
        } else {
            (0, "")
        };
        parts.push(format!("{inputs}concat=n={k}:v=1:a={a_flag}[vc]{a_out}"));
        "[vc]"
    };

    let (w, h) = target_size(p.width, p.height, preset);
    let mut tail = Vec::new();
    if (w, h) != (p.width, p.height) {
        tail.push(format!("scale={w}:{h}:flags=lanczos"));
    }
    if let Some(fps) = target_fps(p.fps) {
        tail.push(format!("fps={fps}"));
    }
    tail.push("format=yuv420p".to_owned());
    parts.push(format!("{video_in}{}[vout]", tail.join(",")));

    let audio_map = match (n, k) {
        (0, _) => None,
        (1, 1) => Some("0:a:0".to_owned()),
        _ => Some("[ac]".to_owned()),
    };
    (parts.join(";"), audio_map)
}

fn encoder_args(enc: Encoder, preset: Preset, gop: u32) -> Vec<String> {
    let (b, m, z) = video_bitrate(preset);
    let own: Vec<&str> = match enc {
        Encoder::Nvenc => vec![
            "-c:v",
            "h264_nvenc",
            "-preset",
            "p5",
            "-tune",
            "hq",
            "-rc",
            "vbr",
            "-b:v",
            b,
            "-maxrate",
            m,
            "-bufsize",
            z,
            "-profile:v",
            "high",
        ],
        Encoder::Amf => vec![
            "-c:v", "h264_amf", "-quality", "balanced", "-rc", "vbr_peak", "-b:v", b, "-maxrate",
            m, "-bufsize", z,
        ],
        Encoder::Qsv => vec![
            "-c:v", "h264_qsv", "-preset", "medium", "-b:v", b, "-maxrate", m, "-bufsize", z,
        ],
        Encoder::Mf => vec!["-c:v", "h264_mf", "-b:v", b],
        Encoder::OpenH264 => vec!["-c:v", "libopenh264", "-b:v", b, "-maxrate", m],
        Encoder::X264 => vec![
            "-c:v", "libx264", "-preset", "veryfast", "-b:v", b, "-maxrate", m, "-bufsize", z,
        ],
    };
    own.into_iter()
        .map(str::to_owned)
        .chain(["-g".to_owned(), gop.to_string()])
        .collect()
}

/// The whole ffmpeg argument list (without the program) that writes `out` from the `kept` pieces of `src`.
/// `kept` is sorted and non-overlapping (the UI keeps it so); empty means the whole source.
pub fn args(
    p: &Probe,
    src: &Path,
    kept: &[Span],
    preset: Preset,
    enc: Encoder,
    out: &Path,
) -> Vec<String> {
    let whole = [Span {
        start_ms: 0,
        end_ms: p.duration_ms,
    }];
    let kept = if kept.is_empty() { &whole } else { kept };
    let (from, to) = (kept[0].start_ms, kept[kept.len() - 1].end_ms);
    let (filter, audio_map) = graph(p, kept, preset);
    // a source with no usable frame rate (0 or NaN) gets a 60 fps GOP
    let src_fps = if p.fps >= 1.0 {
        p.fps.round() as u32
    } else {
        60
    };
    let out_fps = target_fps(p.fps).unwrap_or(src_fps);

    let mut a = head();
    a.extend(
        [
            "-ss",
            &secs(from),
            "-t",
            &secs(to.saturating_sub(from)),
            "-i",
            &src.to_string_lossy(),
            "-filter_complex",
            &filter,
            "-map",
            "[vout]",
        ]
        .map(str::to_owned),
    );
    if let Some(m) = &audio_map {
        a.extend(["-map".to_owned(), m.clone()]);
    }
    a.extend(encoder_args(enc, preset, out_fps * 2));
    if audio_map.is_some() {
        a.extend(["-c:a", "aac", "-b:a", "160k", "-ac", "2", "-ar", "48000"].map(str::to_owned));
    } else {
        a.push("-an".to_owned());
    }
    a.extend(
        [
            "-map_metadata",
            "-1",
            "-movflags",
            "+faststart",
            "-f",
            "mp4",
            "-y",
            &out.to_string_lossy(),
        ]
        .map(str::to_owned),
    );
    a
}

/// Why a run failed, with the end of ffmpeg's stderr for the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunFailed {
    pub stderr_tail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EncodeError {
    #[error("Pulse couldn't prepare that clip.")]
    Failed { stderr_tail: String },
}

/// Something that runs ffmpeg with these arguments and hands over each stdout line as it is printed.
pub trait Runner {
    fn run(
        &self,
        args: Vec<String>,
        on_line: &mut (dyn FnMut(&str) + Send),
    ) -> impl Future<Output = Result<(), RunFailed>> + Send;
}

/// The last `max` bytes of `s`, from a character boundary.
fn tail(s: &str, max: usize) -> String {
    let mut from = s.len().saturating_sub(max);
    while !s.is_char_boundary(from) {
        from += 1;
    }
    s[from..].to_owned()
}

pub struct FfmpegRunner(pub FfmpegPaths);

impl Runner for FfmpegRunner {
    async fn run(
        &self,
        args: Vec<String>,
        on_line: &mut (dyn FnMut(&str) + Send),
    ) -> Result<(), RunFailed> {
        let fail = |what: String| RunFailed { stderr_tail: what };
        let mut cmd = ffmpeg::command(&self.0.ffmpeg, args);
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = ffmpeg::spawn(&mut cmd).map_err(|e| fail(e.to_string()))?;
        let (stdout, mut stderr) = (
            child.stdout.take().expect("piped stdout"),
            child.stderr.take().expect("piped stderr"),
        );
        // Both pipes are read at once, or a chatty one would block ffmpeg.
        let lines = async {
            // bytes, not `lines()`: a non-UTF-8 line must not end the reading (ffmpeg would block on a full pipe)
            let mut reader = BufReader::new(stdout);
            let mut raw = Vec::new();
            loop {
                raw.clear();
                match reader.read_until(b'\n', &mut raw).await {
                    Ok(1..) => on_line(String::from_utf8_lossy(&raw).trim_end()),
                    _ => break,
                }
            }
        };
        let errors = async {
            let (mut kept, mut buf) = (Vec::<u8>::new(), [0u8; 4096]);
            while let Ok(n @ 1..) = stderr.read(&mut buf).await {
                kept.extend_from_slice(&buf[..n]);
                if kept.len() > 8192 {
                    kept.drain(..kept.len() - 4096);
                }
            }
            tail(&String::from_utf8_lossy(&kept), 2000)
        };
        let ((), stderr_tail) = tokio::join!(lines, errors);
        match child.wait().await {
            Ok(s) if s.success() => Ok(()),
            Ok(_) => Err(RunFailed { stderr_tail }),
            Err(e) => Err(fail(e.to_string())),
        }
    }
}

/// The encoder that worked last in this run of Pulse, so a machine without the first GPU encoder does not
/// probe it again for every clip. It only ever moves down `CHAIN`: a later init failure on the remembered
/// encoder falls further down, never back up. So a transient init failure (NVENC's session limit while
/// another app records, a busy GPU) demotes the encoder for the rest of the run, and exports stay slower
/// until Pulse restarts. That is on purpose (plan #14): re-probing the faster encoder before every clip
/// would cost each export a failed start on machines where it never works.
pub struct WorkingEncoder(AtomicUsize);

/// The process-wide store the export and the preview encode share.
pub static WORKING: WorkingEncoder = WorkingEncoder::new();

impl WorkingEncoder {
    pub const fn new() -> Self {
        Self(AtomicUsize::new(0))
    }

    /// The part of `CHAIN` to try: from the remembered encoder on.
    pub fn chain(&self) -> &'static [Encoder] {
        &CHAIN[self.0.load(Ordering::Relaxed).min(CHAIN.len() - 1)..]
    }

    pub fn remember(&self, enc: Encoder) {
        if let Some(i) = CHAIN.iter().position(|&e| e == enc) {
            self.0.fetch_max(i, Ordering::Relaxed);
        }
    }

    /// `run_with_fallback` from the remembered encoder, remembering the one that worked.
    pub async fn run<R: Runner>(
        &self,
        runner: &R,
        kept_ms: u64,
        build: impl Fn(Encoder) -> Vec<String>,
        on_progress: impl FnMut(f32) + Send,
    ) -> Result<Encoder, EncodeError> {
        let enc = run_with_fallback(runner, self.chain(), kept_ms, build, on_progress).await?;
        self.remember(enc);
        Ok(enc)
    }
}

impl Default for WorkingEncoder {
    fn default() -> Self {
        Self::new()
    }
}

/// Try each encoder of `start` in turn. A failed run moves on to the next only when it printed no progress
/// (the encoder could not start: no such GPU, busy); one that failed mid-encode is the end. Progress is
/// reported as the fraction of `kept_ms` of output written, 0 to 1. Returns the encoder that worked.
pub async fn run_with_fallback<R: Runner>(
    runner: &R,
    start: &[Encoder],
    kept_ms: u64,
    build: impl Fn(Encoder) -> Vec<String>,
    mut on_progress: impl FnMut(f32) + Send,
) -> Result<Encoder, EncodeError> {
    let total_us = kept_ms.max(1) as f64 * 1000.0;
    let mut last = RunFailed {
        stderr_tail: String::new(),
    };
    for &enc in start {
        let mut progressed = false;
        let mut on_line = |line: &str| {
            if let Some(ProgressLine::OutTimeUs(us @ 1..)) = progress::parse(line) {
                progressed = true;
                on_progress((us as f64 / total_us).clamp(0.0, 1.0) as f32);
            }
        };
        match runner.run(build(enc), &mut on_line).await {
            Ok(()) => return Ok(enc),
            Err(failed) if progressed => {
                return Err(EncodeError::Failed {
                    stderr_tail: failed.stderr_tail,
                });
            }
            Err(failed) => last = failed,
        }
    }
    Err(EncodeError::Failed {
        stderr_tail: last.stderr_tail,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clips::probe::AudioStream;
    use std::path::Path;
    use std::sync::Mutex;

    fn probe(w: u32, h: u32, fps: f64, audio: usize) -> Probe {
        Probe {
            duration_ms: 60_000,
            width: w,
            height: h,
            fps,
            video_codec: "h264".into(),
            audio: (0..audio)
                .map(|i| AudioStream {
                    index: i as u32 + 1,
                    codec: "aac".into(),
                })
                .collect(),
            format_name: "mov,mp4,m4a,3gp,3g2,mj2".into(),
            title: None,
            size: 1,
        }
    }

    fn span(a: u64, b: u64) -> Span {
        Span {
            start_ms: a,
            end_ms: b,
        }
    }

    fn two() -> Vec<Span> {
        vec![span(10_000, 25_000), span(40_000, 55_500)]
    }

    fn run(p: &Probe, kept: &[Span], preset: Preset, enc: Encoder) -> String {
        args(
            p,
            Path::new("/v/src.mp4"),
            kept,
            preset,
            enc,
            Path::new("/t/out.mp4"),
        )
        .join(" ")
    }

    #[test]
    fn light_caps_720_and_never_upscales() {
        assert_eq!(target_size(2560, 1440, Preset::Light), (1280, 720));
        assert_eq!(target_size(1280, 720, Preset::Light), (1280, 720));
        assert_eq!(target_size(854, 480, Preset::Light), (854, 480));
    }

    #[test]
    fn portrait_caps_the_short_side() {
        assert_eq!(target_size(1080, 1920, Preset::Light), (720, 1280));
    }

    #[test]
    fn odd_sizes_round_to_even() {
        assert_eq!(target_size(1366, 768, Preset::Light), (1280, 720));
        assert_eq!(target_size(1365, 767, Preset::Sharp), (1364, 766));
    }

    #[test]
    fn sixty_fps_or_less_is_kept() {
        for (p, preset) in [
            (probe(1920, 1080, 60.0, 1), Preset::Sharp),
            (probe(1920, 1080, 30.0, 1), Preset::Light),
        ] {
            assert_eq!(target_fps(p.fps), None);
            assert!(!run(&p, &two(), preset, Encoder::X264).contains("fps="));
        }
        assert_eq!(target_fps(60.4), None);
    }

    #[test]
    fn sharp_caps_1080_and_120_fps_to_60() {
        let p = probe(2560, 1440, 120.0, 1);
        assert_eq!(target_size(2560, 1440, Preset::Sharp), (1920, 1080));
        assert_eq!(target_fps(120.0), Some(60));
        let cmd = run(&p, &two(), Preset::Sharp, Encoder::X264);
        assert!(cmd.contains("scale=1920:1080:flags=lanczos,fps=60,format=yuv420p[vout]"));
        assert!(cmd.contains("-g 120"));
    }

    #[test]
    fn bitrates_follow_the_table() {
        assert_eq!(video_bitrate(Preset::Light), ("4M", "6M", "8M"));
        assert_eq!(video_bitrate(Preset::Sharp), ("8M", "12M", "16M"));
    }

    #[test]
    fn two_piece_light_nvenc_matches_the_example() {
        let expected = "-hide_banner -nostdin -loglevel error -progress pipe:1 -nostats -ss 10.000 -t 45.500 -i /v/src.mp4 \
-filter_complex [0:v:0]split=2[vs0][vs1];[0:a:0]asplit=2[as0][as1];[vs0]trim=start=0.000:end=15.000,setpts=PTS-STARTPTS[v0];[as0]atrim=start=0.000:end=15.000,asetpts=PTS-STARTPTS[a0];[vs1]trim=start=30.000:end=45.500,setpts=PTS-STARTPTS[v1];[as1]atrim=start=30.000:end=45.500,asetpts=PTS-STARTPTS[a1];[v0][a0][v1][a1]concat=n=2:v=1:a=1[vc][ac];[vc]scale=1280:720:flags=lanczos,format=yuv420p[vout] \
-map [vout] -map [ac] -c:v h264_nvenc -preset p5 -tune hq -rc vbr -b:v 4M -maxrate 6M -bufsize 8M -profile:v high -g 120 \
-c:a aac -b:a 160k -ac 2 -ar 48000 -map_metadata -1 -movflags +faststart -f mp4 -y /t/out.mp4";
        let p = probe(2560, 1440, 60.0, 1);
        assert_eq!(run(&p, &two(), Preset::Light, Encoder::Nvenc), expected);
    }

    #[test]
    fn one_piece_has_no_split_or_concat() {
        let p = probe(1920, 1080, 60.0, 1);
        let cmd = run(&p, &[span(5_000, 9_500)], Preset::Light, Encoder::X264);
        assert!(cmd.contains("-ss 5.000 -t 4.500 -i /v/src.mp4"));
        assert!(
            cmd.contains(
                "-filter_complex [0:v:0]scale=1280:720:flags=lanczos,format=yuv420p[vout] "
            )
        );
        assert!(cmd.contains("-map [vout] -map 0:a:0 "));
        for bad in ["split", "concat", "trim"] {
            assert!(!cmd.contains(bad), "{bad}");
        }
    }

    #[test]
    fn two_audio_tracks_are_mixed() {
        let p = probe(1920, 1080, 60.0, 2);
        let cmd = run(&p, &two(), Preset::Light, Encoder::X264);
        let mix = cmd
            .find("[0:a:0][0:a:1]amix=inputs=2:normalize=0[am];")
            .expect("mix");
        let split = cmd.find("[am]asplit=2[as0][as1]").expect("asplit");
        assert!(mix < split);
        // one piece: the mix is the audio
        let one = run(&p, &[span(0, 3_000)], Preset::Light, Encoder::X264);
        assert!(one.contains("[0:a:0][0:a:1]amix=inputs=2:normalize=0[ac]"));
        assert!(one.contains("-map [ac]"));
    }

    #[test]
    fn no_audio_maps_no_audio() {
        let p = probe(1920, 1080, 60.0, 0);
        for kept in [two(), vec![span(0, 3_000)]] {
            let cmd = run(&p, &kept, Preset::Light, Encoder::X264);
            assert!(cmd.contains(" -an "));
            assert!(!cmd.contains("[0:a"));
            assert!(!cmd.contains("-c:a") && !cmd.contains("-map 0:a"));
            assert!(!cmd.contains("atrim") && !cmd.contains("asplit"));
        }
        assert!(run(&p, &two(), Preset::Light, Encoder::X264).contains("concat=n=2:v=1:a=0[vc];"));
    }

    #[test]
    fn metadata_is_stripped_and_faststart_set() {
        let cmd = run(
            &probe(1920, 1080, 60.0, 1),
            &two(),
            Preset::Light,
            Encoder::X264,
        );
        assert!(cmd.ends_with("-map_metadata -1 -movflags +faststart -f mp4 -y /t/out.mp4"));
    }

    #[test]
    fn each_encoders_args_for_light() {
        let p = probe(1920, 1080, 30.0, 1);
        let enc = |e| {
            let cmd = run(&p, &[span(0, 1000)], Preset::Light, e);
            let from = cmd.find("-c:v").unwrap();
            let to = cmd.find("-c:a").unwrap();
            cmd[from..to].trim().to_owned()
        };
        assert_eq!(
            enc(Encoder::Nvenc),
            "-c:v h264_nvenc -preset p5 -tune hq -rc vbr -b:v 4M -maxrate 6M -bufsize 8M -profile:v high -g 60"
        );
        assert_eq!(
            enc(Encoder::Amf),
            "-c:v h264_amf -quality balanced -rc vbr_peak -b:v 4M -maxrate 6M -bufsize 8M -g 60"
        );
        assert_eq!(
            enc(Encoder::Qsv),
            "-c:v h264_qsv -preset medium -b:v 4M -maxrate 6M -bufsize 8M -g 60"
        );
        assert_eq!(enc(Encoder::Mf), "-c:v h264_mf -b:v 4M -g 60");
        assert_eq!(
            enc(Encoder::OpenH264),
            "-c:v libopenh264 -b:v 4M -maxrate 6M -g 60"
        );
        assert_eq!(
            enc(Encoder::X264),
            "-c:v libx264 -preset veryfast -b:v 4M -maxrate 6M -bufsize 8M -g 60"
        );
    }

    #[test]
    fn the_chain_is_hardware_first() {
        let names: Vec<_> = CHAIN.iter().map(|e| e.name()).collect();
        assert_eq!(
            names,
            [
                "h264_nvenc",
                "h264_amf",
                "h264_qsv",
                "h264_mf",
                "libopenh264",
                "libx264"
            ]
        );
    }

    /// Plays a script: one outcome per call, each with the lines it printed first.
    struct Fake {
        script: Mutex<Vec<(Vec<&'static str>, bool)>>,
        calls: Mutex<Vec<String>>,
    }

    impl Fake {
        fn new(script: Vec<(Vec<&'static str>, bool)>) -> Self {
            Self {
                script: Mutex::new(script),
                calls: Mutex::default(),
            }
        }
    }

    impl Runner for Fake {
        async fn run(
            &self,
            args: Vec<String>,
            on_line: &mut (dyn FnMut(&str) + Send),
        ) -> Result<(), RunFailed> {
            self.calls.lock().unwrap().push(args.join(" "));
            let (lines, ok) = self.script.lock().unwrap().remove(0);
            for l in lines {
                on_line(l);
            }
            if ok {
                Ok(())
            } else {
                Err(RunFailed {
                    stderr_tail: "boom".into(),
                })
            }
        }
    }

    // The fake keeps its state in Mutexes so its futures are Send like the real runner's (checked below).
    #[tokio::test]
    async fn gpu_init_failure_falls_back_to_cpu() {
        let fake = Fake::new(vec![
            (vec![], false),
            (vec!["out_time_us=0"], false),
            (vec!["out_time_us=N/A"], false),
            (vec![], false),
            (vec!["out_time_us=2000000", "progress=end"], true),
        ]);
        let mut seen = Vec::new();
        let got = run_with_fallback(
            &fake,
            &CHAIN,
            4_000,
            |e| vec![e.name().to_owned()],
            |f| seen.push(f),
        )
        .await;
        assert_eq!(got, Ok(Encoder::OpenH264));
        assert_eq!(
            *fake.calls.lock().unwrap(),
            [
                "h264_nvenc",
                "h264_amf",
                "h264_qsv",
                "h264_mf",
                "libopenh264"
            ]
        );
        assert_eq!(seen, [0.5]);
    }

    #[tokio::test]
    async fn a_mid_encode_failure_does_not_fall_back() {
        let fake = Fake::new(vec![(vec!["out_time_us=1000000"], false)]);
        let got =
            run_with_fallback(&fake, &CHAIN, 4_000, |e| vec![e.name().to_owned()], |_| {}).await;
        assert!(matches!(got, Err(EncodeError::Failed { .. })));
        assert_eq!(fake.calls.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn every_encoder_failing_to_start_is_an_error() {
        let fake = Fake::new(vec![(vec![], false); 6]);
        let got =
            run_with_fallback(&fake, &CHAIN, 4_000, |e| vec![e.name().to_owned()], |_| {}).await;
        assert_eq!(
            got,
            Err(EncodeError::Failed {
                stderr_tail: "boom".into()
            })
        );
        assert_eq!(fake.calls.lock().unwrap().len(), 6);
    }

    #[tokio::test]
    async fn progress_is_a_clamped_fraction_of_the_kept_time() {
        let fake = Fake::new(vec![(
            vec!["out_time_us=1000000", "out_time_us=9000000", "progress=end"],
            true,
        )]);
        let mut seen = Vec::new();
        run_with_fallback(
            &fake,
            &CHAIN,
            4_000,
            |e| vec![e.name().to_owned()],
            |f| seen.push(f),
        )
        .await
        .unwrap();
        assert_eq!(seen, [0.25, 1.0]);
    }

    #[test]
    fn the_real_runners_future_is_send() {
        fn needs_send<T: Send>(_: T) {}
        let r = FfmpegRunner(FfmpegPaths {
            ffmpeg: "ffmpeg".into(),
            ffprobe: "ffprobe".into(),
        });
        let mut on_line = |_: &str| {};
        needs_send(r.run(Vec::new(), &mut on_line));
    }

    #[test]
    fn a_source_without_a_frame_rate_gets_a_60_fps_gop() {
        for fps in [0.0, f64::NAN] {
            let cmd = run(
                &probe(1920, 1080, fps, 1),
                &two(),
                Preset::Light,
                Encoder::X264,
            );
            assert!(cmd.contains("-g 120 "), "{fps}: {cmd}");
        }
    }

    #[tokio::test]
    async fn the_working_encoder_is_remembered_and_never_moves_back_up() {
        let store = WorkingEncoder::new();
        let build = |e: Encoder| vec![e.name().to_owned()];
        // first run: Nvenc and Amf cannot start, Qsv works
        let fake = Fake::new(vec![(vec![], false), (vec![], false), (vec![], true)]);
        assert_eq!(
            store.run(&fake, 1000, build, |_| {}).await,
            Ok(Encoder::Qsv)
        );
        // second run starts at Qsv
        let fake = Fake::new(vec![(vec![], true)]);
        assert_eq!(
            store.run(&fake, 1000, build, |_| {}).await,
            Ok(Encoder::Qsv)
        );
        assert_eq!(*fake.calls.lock().unwrap(), ["h264_qsv"]);
        // Qsv now fails to start: Mf is tried, and is the new start
        let fake = Fake::new(vec![(vec![], false), (vec![], true)]);
        assert_eq!(store.run(&fake, 1000, build, |_| {}).await, Ok(Encoder::Mf));
        assert_eq!(*fake.calls.lock().unwrap(), ["h264_qsv", "h264_mf"]);
        assert_eq!(store.chain()[0], Encoder::Mf);
        // a stray later report of an earlier encoder does not move it back up
        store.remember(Encoder::Nvenc);
        assert_eq!(store.chain()[0], Encoder::Mf);
        // a mid-encode failure remembers nothing new
        let fake = Fake::new(vec![(vec!["out_time_us=1"], false)]);
        assert!(store.run(&fake, 1000, build, |_| {}).await.is_err());
        assert_eq!(store.chain()[0], Encoder::Mf);
    }

    #[test]
    fn tail_keeps_the_end() {
        assert_eq!(tail("short", 100), "short");
        assert_eq!(tail("0123456789", 4), "6789");
        assert_eq!(tail("ééé", 3), "é");
    }
}
