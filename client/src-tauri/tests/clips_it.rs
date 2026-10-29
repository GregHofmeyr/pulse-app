//! The export against a real ffmpeg. Run: `PULSE_DEV_FFMPEG_DIR=/usr/bin cargo test -p pulse-client --test
//! clips_it -- --ignored` (or with ffmpeg and ffprobe on PATH).

use std::path::{Path, PathBuf};
use std::process::Command;

use std::sync::Mutex;

use pulse_client::clips::encode::{
    self, CHAIN, Encoder, FfmpegRunner, Preset, RunFailed, Runner, Span, run_with_fallback,
};
use pulse_client::clips::ffmpeg::{self, FfmpegPaths};
use pulse_client::clips::probe::Probe;
use pulse_client::clips::{preview, probe};

fn tools() -> FfmpegPaths {
    match std::env::var_os(ffmpeg::DEV_DIR_ENV).filter(|v| !v.is_empty()) {
        Some(dir) => FfmpegPaths {
            ffmpeg: PathBuf::from(&dir).join("ffmpeg"),
            ffprobe: PathBuf::from(&dir).join("ffprobe"),
        },
        None => FfmpegPaths {
            ffmpeg: "ffmpeg".into(),
            ffprobe: "ffprobe".into(),
        },
    }
}

fn ffmpeg_ok(t: &FfmpegPaths, args: &[&str]) {
    let out = Command::new(&t.ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-nostdin"])
        .args(args)
        .output()
        .expect("run ffmpeg");
    assert!(
        out.status.success(),
        "ffmpeg {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn ffprobe_json(t: &FfmpegPaths, path: &Path) -> serde_json::Value {
    let out = Command::new(&t.ffprobe)
        .args(probe::args(&path.to_string_lossy()))
        .output()
        .expect("run ffprobe");
    assert!(out.status.success());
    serde_json::from_slice(&out.stdout).expect("ffprobe json")
}

/// The names of the top-level boxes of an MP4, in file order.
fn top_level_boxes(path: &Path) -> Vec<String> {
    let bytes = std::fs::read(path).unwrap();
    let (mut at, mut names) = (0usize, Vec::new());
    while at + 8 <= bytes.len() {
        let size = u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
        names.push(String::from_utf8_lossy(&bytes[at + 4..at + 8]).into_owned());
        if size < 8 {
            break;
        }
        at += size;
    }
    names
}

/// 10 s of 1440p60 with two audio tracks (and a title tag), probed.
fn two_track_source(t: &FfmpegPaths, src: &Path) -> Probe {
    ffmpeg_ok(
        t,
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=2560x1440:rate=60:duration=10",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=10",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=660:duration=10",
            "-map",
            "0:v",
            "-map",
            "1:a",
            "-map",
            "2:a",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-crf",
            "30",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-metadata",
            "title=secret",
            "-y",
            &src.to_string_lossy(),
        ],
    );
    let p = probe::parse(
        &String::from_utf8_lossy(
            &Command::new(&t.ffprobe)
                .args(probe::args(&src.to_string_lossy()))
                .output()
                .unwrap()
                .stdout,
        ),
        std::fs::metadata(src).unwrap().len(),
    )
    .expect("probe the source");
    assert_eq!((p.width, p.height, p.audio.len()), (2560, 1440, 2));
    p
}

/// The two pieces both two-piece exports keep: 5 s.
const KEPT: [Span; 2] = [
    Span {
        start_ms: 1_000,
        end_ms: 3_000,
    },
    Span {
        start_ms: 6_000,
        end_ms: 9_000,
    },
];

#[tokio::test]
#[ignore = "needs ffmpeg and ffprobe (PULSE_DEV_FFMPEG_DIR or PATH)"]
async fn exports_two_pieces_as_light() {
    let t = tools();
    let dir = tempfile::tempdir().unwrap();
    let (src, out) = (dir.path().join("src.mp4"), dir.path().join("out.mp4"));

    let p = two_track_source(&t, &src);

    let kept = KEPT;
    let mut last = 0.0f32;
    let used = run_with_fallback(
        &FfmpegRunner(t.clone()),
        &CHAIN,
        5_000,
        |enc| encode::args(&p, &src, &kept, Preset::Light, enc, &out),
        |f| last = f,
    )
    .await
    .expect("export");
    eprintln!("clips_it: encoder used = {}", used.name());
    assert!(last > 0.5, "progress reached {last}");

    let j = ffprobe_json(&t, &out);
    let streams = j["streams"].as_array().unwrap();
    let video: Vec<_> = streams
        .iter()
        .filter(|s| s["codec_type"] == "video")
        .collect();
    let audio: Vec<_> = streams
        .iter()
        .filter(|s| s["codec_type"] == "audio")
        .collect();
    assert_eq!(video.len(), 1);
    assert_eq!(
        (video[0]["width"].as_u64(), video[0]["height"].as_u64()),
        (Some(1280), Some(720))
    );
    assert_eq!(video[0]["codec_name"], "h264");
    let (n, d) = video[0]["avg_frame_rate"]
        .as_str()
        .unwrap()
        .split_once('/')
        .unwrap();
    assert!(n.parse::<f64>().unwrap() / d.parse::<f64>().unwrap() <= 60.5);
    assert_eq!(audio.len(), 1);
    assert_eq!(audio[0]["codec_name"], "aac");
    assert_eq!(audio[0]["channels"], 2);
    let secs: f64 = j["format"]["duration"].as_str().unwrap().parse().unwrap();
    assert!((secs - 5.0).abs() < 0.1, "duration {secs}");
    assert!(
        j["format"]["tags"].get("title").is_none(),
        "title tag survived"
    );

    let boxes = top_level_boxes(&out);
    let at = |n: &str| {
        boxes
            .iter()
            .position(|b| b == n)
            .unwrap_or_else(|| panic!("{n} in {boxes:?}"))
    };
    assert!(at("moov") < at("mdat"), "faststart: {boxes:?}");

    // thumbnail and filmstrip of the source
    let (thumb, strip) = (dir.path().join("thumb.jpg"), dir.path().join("strip.jpg"));
    ffmpeg_ok(
        &t,
        &preview::thumb_args(&src.to_string_lossy(), 1_500, &thumb.to_string_lossy())
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
    );
    ffmpeg_ok(
        &t,
        &preview::filmstrip_args(&src.to_string_lossy(), 10_000, &strip.to_string_lossy())
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
    );
    assert!(thumb.is_file() && strip.is_file());
}

/// A phone clip: stored 2560x1440 with a 90 degree display rotation, so it shows as portrait 1440x2560.
#[tokio::test]
#[ignore = "needs ffmpeg and ffprobe (PULSE_DEV_FFMPEG_DIR or PATH)"]
async fn exports_a_rotated_source_as_portrait_light() {
    let t = tools();
    let dir = tempfile::tempdir().unwrap();
    let (plain, src, out) = (
        dir.path().join("plain.mp4"),
        dir.path().join("rot.mp4"),
        dir.path().join("out.mp4"),
    );
    ffmpeg_ok(
        &t,
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=2560x1440:rate=30:duration=3",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-crf",
            "30",
            "-pix_fmt",
            "yuv420p",
            "-y",
            &plain.to_string_lossy(),
        ],
    );
    ffmpeg_ok(
        &t,
        &[
            "-display_rotation",
            "90",
            "-i",
            &plain.to_string_lossy(),
            "-c",
            "copy",
            "-y",
            &src.to_string_lossy(),
        ],
    );
    let p = probe::parse(
        &String::from_utf8_lossy(
            &Command::new(&t.ffprobe)
                .args(probe::args(&src.to_string_lossy()))
                .output()
                .unwrap()
                .stdout,
        ),
        std::fs::metadata(&src).unwrap().len(),
    )
    .expect("probe the source");
    assert_eq!(
        (p.width, p.height),
        (1440, 2560),
        "probe applies the rotation"
    );

    let kept = [Span {
        start_ms: 0,
        end_ms: 2_000,
    }];
    run_with_fallback(
        &FfmpegRunner(t.clone()),
        &CHAIN,
        2_000,
        |enc| encode::args(&p, &src, &kept, Preset::Light, enc, &out),
        |_| {},
    )
    .await
    .expect("export");

    let j = ffprobe_json(&t, &out);
    let v = j["streams"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["codec_type"] == "video")
        .unwrap();
    assert_eq!(
        (v["width"].as_u64(), v["height"].as_u64()),
        (Some(720), Some(1280))
    );
    assert!(
        j["streams"]
            .as_array()
            .unwrap()
            .iter()
            .all(|s| s["codec_type"] != "audio")
    );
}

/// One run: its progress lines and outcome.
type Attempt = (Vec<String>, Result<(), RunFailed>);

/// An [`FfmpegRunner`] that keeps each attempt's progress lines and outcome.
struct Recording {
    inner: FfmpegRunner,
    attempts: Mutex<Vec<Attempt>>,
}

impl Runner for Recording {
    async fn run(
        &self,
        args: Vec<String>,
        on_line: &mut (dyn FnMut(&str) + Send),
    ) -> Result<(), RunFailed> {
        let mut lines = Vec::new();
        let got = self
            .inner
            .run(args, &mut |l: &str| {
                lines.push(l.to_owned());
                on_line(l);
            })
            .await;
        self.attempts.lock().unwrap().push((lines, got.clone()));
        got
    }
}

/// A GPU encoder that cannot open (no NVIDIA driver, an AMD or Intel GPU) fails lazily, on the first
/// filtered frame, after the audio has been encoded and a progress block with audio time printed. That must
/// still fall back. The GPU's failure is stood in for by libx264 with a preset that does not exist, which
/// fails to open the same way on every machine (with or without a GPU).
#[tokio::test]
#[ignore = "needs ffmpeg and ffprobe (PULSE_DEV_FFMPEG_DIR or PATH)"]
async fn an_encoder_that_cannot_open_falls_back_to_the_next() {
    let t = tools();
    let dir = tempfile::tempdir().unwrap();
    let (src, out) = (dir.path().join("src.mp4"), dir.path().join("out.mp4"));
    let p = two_track_source(&t, &src);
    let runner = Recording {
        inner: FfmpegRunner(t.clone()),
        attempts: Mutex::default(),
    };
    let mut last = 0.0f32;
    let used = run_with_fallback(
        &runner,
        &[Encoder::Nvenc, Encoder::X264],
        5_000,
        |enc| {
            let mut a = encode::args(&p, &src, &KEPT, Preset::Light, Encoder::X264, &out);
            if enc == Encoder::Nvenc {
                let at = a.iter().position(|x| x == "veryfast").expect("x264 preset");
                a[at] = "no-such-preset".to_owned();
            }
            a
        },
        |f| last = f,
    )
    .await
    .expect("export");
    assert_eq!(used, Encoder::X264);
    assert!(last > 0.5, "progress reached {last}");

    let attempts = runner.attempts.lock().unwrap();
    assert_eq!(attempts.len(), 2);
    let (lines, got) = &attempts[0];
    let stderr = &got
        .as_ref()
        .expect_err("the first encoder fails")
        .stderr_tail;
    assert!(stderr.contains("Error while opening encoder"), "{stderr}");
    // the trap: audio time was reported before the encoder failed to open
    assert!(
        lines.iter().any(|l| l
            .strip_prefix("out_time_us=")
            .and_then(|v| v.parse::<u64>().ok())
            .is_some_and(|us| us > 0)),
        "expected audio-only progress before the open failure: {lines:?}"
    );
    assert!(attempts[1].1.is_ok());

    let j = ffprobe_json(&t, &out);
    let v = j["streams"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["codec_type"] == "video")
        .unwrap();
    assert_eq!(
        (v["width"].as_u64(), v["height"].as_u64()),
        (Some(1280), Some(720))
    );
    let secs: f64 = j["format"]["duration"].as_str().unwrap().parse().unwrap();
    assert!((secs - 5.0).abs() < 0.1, "duration {secs}");
}
