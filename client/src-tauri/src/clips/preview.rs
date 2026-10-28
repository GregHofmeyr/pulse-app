//! How the editor plays a local file, and the ffmpeg arguments for the preview, filmstrip and thumbnail.

use serde::Serialize;

use super::probe::Probe;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PreviewPlan {
    Direct,
    Remux,
    Encode,
}

pub const FILMSTRIP_TILES: u32 = 24;

fn first_audio(p: &Probe) -> Option<&str> {
    p.audio.first().map(|a| a.codec.as_str())
}

/// Audio the `<video>` element plays inside an MP4: AAC or MP3 (or none at all).
fn browser_audio(p: &Probe) -> bool {
    matches!(first_audio(p), None | Some("aac" | "mp3"))
}

/// How to play `p` (a file with this extension) in the webview.
pub fn plan(p: &Probe, extension: &str) -> PreviewPlan {
    let ext = extension.to_ascii_lowercase();
    let h264 = p.video_codec == "h264";
    match ext.as_str() {
        "mp4" | "mov" | "m4v" if h264 && browser_audio(p) => PreviewPlan::Direct,
        "webm" if matches!(p.video_codec.as_str(), "vp8" | "vp9") => PreviewPlan::Direct,
        _ if h264 => PreviewPlan::Remux,
        _ => PreviewPlan::Encode,
    }
}

/// A remux can keep the first audio track as it is.
pub fn can_copy_audio(p: &Probe) -> bool {
    browser_audio(p)
}

/// The encoder's fastest preset (software MF and openh264 have none).
fn fast_preset(encoder: &str) -> &'static [&'static str] {
    match encoder {
        "h264_nvenc" => &["-preset", "p1"],
        "h264_amf" => &["-quality", "speed"],
        "h264_qsv" => &["-preset", "veryfast"],
        "libx264" => &["-preset", "ultrafast"],
        _ => &[],
    }
}

/// Arguments that write the preview `out` (an MP4) from `src`. `Direct` needs none (an empty list).
/// `encoder` and `copy_audio` matter to Encode and Remux respectively.
pub fn args(
    plan: PreviewPlan,
    src: &str,
    out: &str,
    encoder: &str,
    copy_audio: bool,
) -> Vec<String> {
    let mut a = vec!["-i", src, "-map", "0:v:0", "-map", "0:a:0?"];
    match plan {
        PreviewPlan::Direct => return Vec::new(),
        PreviewPlan::Remux => {
            a.extend(["-c:v", "copy"]);
            if copy_audio {
                a.extend(["-c:a", "copy"]);
            } else {
                a.extend(["-c:a", "aac", "-b:a", "160k"]);
            }
        }
        PreviewPlan::Encode => {
            a.extend([
                "-vf",
                "scale=-2:540",
                "-pix_fmt",
                "yuv420p",
                "-c:v",
                encoder,
            ]);
            a.extend(fast_preset(encoder));
            a.extend(["-b:v", "2500k", "-c:a", "aac", "-b:a", "128k"]);
        }
    }
    a.extend(["-movflags", "+faststart", "-y", out]);
    a.into_iter().map(str::to_owned).collect()
}

/// One JPEG of 24 keyframe tiles, 72 px tall, spread over the source.
pub fn filmstrip_args(src: &str, duration_ms: u64, out_jpg: &str) -> Vec<String> {
    let step = duration_ms as f64 / 1000.0 / f64::from(FILMSTRIP_TILES);
    let filter = format!(
        "select='isnan(prev_selected_t)+gte(t-prev_selected_t,{step:.3})',scale=-2:72,tile={FILMSTRIP_TILES}x1"
    );
    [
        "-skip_frame",
        "nokey",
        "-i",
        src,
        "-vf",
        &filter,
        "-frames:v",
        "1",
        "-q:v",
        "5",
        "-y",
        out_jpg,
    ]
    .map(str::to_owned)
    .to_vec()
}

/// One JPEG frame at `at_source_ms`, at most 640 px on its long side.
pub fn thumb_args(src: &str, at_source_ms: u64, out_jpg: &str) -> Vec<String> {
    let at = format!("{:.3}", at_source_ms as f64 / 1000.0);
    [
        "-ss",
        &at,
        "-i",
        src,
        "-frames:v",
        "1",
        "-vf",
        "scale='if(gt(iw,ih),min(640,iw),-2)':'if(gt(iw,ih),-2,min(640,ih))'",
        "-q:v",
        "3",
        "-y",
        out_jpg,
    ]
    .map(str::to_owned)
    .to_vec()
}

#[cfg(test)]
mod tests {
    use super::super::probe::AudioStream;
    use super::*;

    fn probe(video: &str, audio: &[&str]) -> Probe {
        Probe {
            duration_ms: 41_000,
            width: 1920,
            height: 1080,
            fps: 60.0,
            video_codec: video.into(),
            audio: audio
                .iter()
                .enumerate()
                .map(|(i, c)| AudioStream {
                    index: i as u32 + 1,
                    codec: (*c).into(),
                })
                .collect(),
            format_name: String::new(),
            title: None,
            size: 1,
        }
    }

    #[test]
    fn plan_picks_direct_remux_or_encode() {
        use PreviewPlan::*;
        assert_eq!(plan(&probe("h264", &["aac"]), "mp4"), Direct);
        assert_eq!(plan(&probe("h264", &["opus"]), "mkv"), Remux);
        assert_eq!(plan(&probe("hevc", &["aac"]), "mp4"), Encode);
        assert_eq!(plan(&probe("vp9", &["opus"]), "webm"), Direct);
        assert_eq!(plan(&probe("vp9", &["opus"]), "mkv"), Encode);
        assert_eq!(plan(&probe("h264", &[]), "mov"), Direct);
        // extension case, and audio the browser can't play in an mp4
        assert_eq!(plan(&probe("h264", &["mp3"]), "MP4"), Direct);
        assert_eq!(plan(&probe("h264", &["pcm_s16le"]), "mp4"), Remux);
        assert_eq!(plan(&probe("av1", &[]), "webm"), Encode);
    }

    #[test]
    fn audio_is_copied_only_when_the_browser_plays_it() {
        assert!(can_copy_audio(&probe("h264", &[])));
        assert!(can_copy_audio(&probe("h264", &["aac", "opus"])));
        assert!(can_copy_audio(&probe("h264", &["mp3"])));
        assert!(!can_copy_audio(&probe("h264", &["opus"])));
    }

    fn v(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn remux_copies_video_and_maybe_audio() {
        assert_eq!(
            args(PreviewPlan::Remux, "in.mkv", "out.mp4", "", true),
            v(&[
                "-i",
                "in.mkv",
                "-map",
                "0:v:0",
                "-map",
                "0:a:0?",
                "-c:v",
                "copy",
                "-c:a",
                "copy",
                "-movflags",
                "+faststart",
                "-y",
                "out.mp4"
            ])
        );
        assert_eq!(
            args(PreviewPlan::Remux, "in.mkv", "out.mp4", "", false),
            v(&[
                "-i",
                "in.mkv",
                "-map",
                "0:v:0",
                "-map",
                "0:a:0?",
                "-c:v",
                "copy",
                "-c:a",
                "aac",
                "-b:a",
                "160k",
                "-movflags",
                "+faststart",
                "-y",
                "out.mp4"
            ])
        );
    }

    #[test]
    fn encode_scales_to_540_with_the_fastest_preset() {
        assert_eq!(
            args(
                PreviewPlan::Encode,
                "in.mp4",
                "out.mp4",
                "h264_nvenc",
                false
            ),
            v(&[
                "-i",
                "in.mp4",
                "-map",
                "0:v:0",
                "-map",
                "0:a:0?",
                "-vf",
                "scale=-2:540",
                "-pix_fmt",
                "yuv420p",
                "-c:v",
                "h264_nvenc",
                "-preset",
                "p1",
                "-b:v",
                "2500k",
                "-c:a",
                "aac",
                "-b:a",
                "128k",
                "-movflags",
                "+faststart",
                "-y",
                "out.mp4"
            ])
        );
        let x264 = args(PreviewPlan::Encode, "i", "o", "libx264", false);
        assert!(x264.windows(2).any(|w| w == ["-preset", "ultrafast"]));
        let amf = args(PreviewPlan::Encode, "i", "o", "h264_amf", false);
        assert!(amf.windows(2).any(|w| w == ["-quality", "speed"]));
        let qsv = args(PreviewPlan::Encode, "i", "o", "h264_qsv", false);
        assert!(qsv.windows(2).any(|w| w == ["-preset", "veryfast"]));
        // software MF / openh264 have no preset
        let mf = args(PreviewPlan::Encode, "i", "o", "h264_mf", false);
        assert!(!mf.iter().any(|a| a == "-preset" || a == "-quality"));
    }

    #[test]
    fn filmstrip_args_for_a_41_second_source() {
        assert_eq!(
            filmstrip_args("in.mp4", 41_000, "film.jpg"),
            v(&[
                "-skip_frame",
                "nokey",
                "-i",
                "in.mp4",
                "-vf",
                "select='isnan(prev_selected_t)+gte(t-prev_selected_t,1.708)',scale=-2:72,tile=24x1",
                "-frames:v",
                "1",
                "-q:v",
                "5",
                "-y",
                "film.jpg"
            ])
        );
    }

    #[test]
    fn thumb_args_seek_before_the_input() {
        assert_eq!(
            thumb_args("in.mp4", 20_500, "t.jpg"),
            v(&[
                "-ss",
                "20.500",
                "-i",
                "in.mp4",
                "-frames:v",
                "1",
                "-vf",
                "scale='if(gt(iw,ih),min(640,iw),-2)':'if(gt(iw,ih),-2,min(640,ih))'",
                "-q:v",
                "3",
                "-y",
                "t.jpg"
            ])
        );
    }
}
