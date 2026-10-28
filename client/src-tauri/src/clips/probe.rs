//! `ffprobe` arguments and the parse of its JSON.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioStream {
    pub index: u32,
    pub codec: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Probe {
    pub duration_ms: u64,
    /// As displayed: a rotation of 90 or 270 degrees is already applied.
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub video_codec: String,
    pub audio: Vec<AudioStream>,
    pub format_name: String,
    pub title: Option<String>,
    pub size: u64,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ProbeError {
    #[error("Pulse couldn't read that video.")]
    NoVideo,
    #[error("Pulse couldn't read that video.")]
    Unreadable,
}

pub fn args(path: &str) -> Vec<String> {
    [
        "-v",
        "error",
        "-print_format",
        "json",
        "-show_format",
        "-show_streams",
        path,
    ]
    .map(str::to_owned)
    .to_vec()
}

#[derive(Deserialize)]
struct Raw {
    #[serde(default)]
    streams: Vec<RawStream>,
    format: Option<RawFormat>,
}

#[derive(Deserialize)]
struct RawStream {
    index: Option<u32>,
    codec_name: Option<String>,
    codec_type: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    avg_frame_rate: Option<String>,
    r_frame_rate: Option<String>,
    duration: Option<String>,
    disposition: Option<RawDisposition>,
    #[serde(default)]
    side_data_list: Vec<serde_json::Value>,
    #[serde(default)]
    tags: std::collections::HashMap<String, serde_json::Value>,
}

#[derive(Deserialize)]
struct RawDisposition {
    attached_pic: Option<u8>,
}

#[derive(Deserialize)]
struct RawFormat {
    format_name: Option<String>,
    duration: Option<String>,
    #[serde(default)]
    tags: std::collections::HashMap<String, serde_json::Value>,
}

/// "60/1" or "30000/1001" or "25"; `0/0` and nonsense are `None`.
fn rate(s: Option<&str>) -> Option<f64> {
    let s = s?;
    let (n, d) = s.split_once('/').unwrap_or((s, "1"));
    let (n, d): (f64, f64) = (n.trim().parse().ok()?, d.trim().parse().ok()?);
    (n > 0.0 && d > 0.0).then(|| n / d)
}

fn tag(tags: &std::collections::HashMap<String, serde_json::Value>, name: &str) -> Option<String> {
    tags.iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .and_then(|(_, v)| v.as_str())
        .map(str::to_owned)
}

/// Degrees the display matrix (or the old `rotate` tag) turns the picture by.
fn rotation(v: &RawStream) -> i64 {
    v.side_data_list
        .iter()
        .find_map(|d| d.get("rotation").and_then(|r| r.as_f64()))
        .or_else(|| tag(&v.tags, "rotate").and_then(|r| r.parse().ok()))
        .map_or(0, |r| r as i64)
}

pub fn parse(json: &str, file_size: u64) -> Result<Probe, ProbeError> {
    let raw: Raw = serde_json::from_str(json).map_err(|_| ProbeError::Unreadable)?;
    let video = raw
        .streams
        .iter()
        // cover art (an mp3 with a picture) is a "video" stream that is not a video
        .find(|s| {
            s.codec_type.as_deref() == Some("video")
                && s.disposition.as_ref().and_then(|d| d.attached_pic) != Some(1)
        })
        .ok_or(ProbeError::NoVideo)?;
    let (w, h) = (video.width.unwrap_or(0), video.height.unwrap_or(0));
    if w == 0 || h == 0 {
        return Err(ProbeError::Unreadable);
    }
    let (width, height) = if rotation(video).rem_euclid(180) == 90 {
        (h, w)
    } else {
        (w, h)
    };
    let secs: f64 = raw
        .format
        .as_ref()
        .and_then(|f| f.duration.as_deref())
        .or(video.duration.as_deref())
        .and_then(|d| d.parse().ok())
        .filter(|d: &f64| d.is_finite() && *d > 0.0)
        .ok_or(ProbeError::Unreadable)?;
    let audio = raw
        .streams
        .iter()
        .filter(|s| s.codec_type.as_deref() == Some("audio"))
        .map(|s| AudioStream {
            index: s.index.unwrap_or(0),
            codec: s.codec_name.clone().unwrap_or_default(),
        })
        .collect();
    let title = raw
        .format
        .as_ref()
        .and_then(|f| tag(&f.tags, "title"))
        .map(|t| t.trim().to_owned())
        .filter(|t| !t.is_empty());
    Ok(Probe {
        duration_ms: (secs * 1000.0).round() as u64,
        width,
        height,
        fps: rate(video.avg_frame_rate.as_deref())
            .or_else(|| rate(video.r_frame_rate.as_deref()))
            .unwrap_or(0.0),
        video_codec: video.codec_name.clone().unwrap_or_default(),
        audio,
        format_name: raw.format.and_then(|f| f.format_name).unwrap_or_default(),
        title,
        size: file_size,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const NVIDIA: &str = include_str!("../../tests/fixtures/clips/nvidia_replay.json");
    const ROTATED: &str = include_str!("../../tests/fixtures/clips/rotated_phone.json");
    const TWO_AUDIO: &str = include_str!("../../tests/fixtures/clips/two_audio.json");
    const NO_VIDEO: &str = include_str!("../../tests/fixtures/clips/no_video.json");

    #[test]
    fn args_ask_for_json_format_and_streams() {
        assert_eq!(
            args("/v/a.mp4"),
            [
                "-v",
                "error",
                "-print_format",
                "json",
                "-show_format",
                "-show_streams",
                "/v/a.mp4"
            ]
        );
    }

    #[test]
    fn parses_an_nvidia_replay() {
        let p = parse(NVIDIA, 944_000_000).unwrap();
        assert_eq!((p.width, p.height), (2560, 1440));
        assert_eq!(p.fps, 60.0);
        assert_eq!(p.video_codec, "h264");
        assert_eq!(
            p.audio,
            [AudioStream {
                index: 1,
                codec: "aac".into()
            }]
        );
        assert_eq!(p.format_name, "mov,mp4,m4a,3gp,3g2,mj2");
        assert_eq!(p.title.as_deref(), Some("RUST"));
        assert_eq!(p.duration_ms, 87_360);
        assert_eq!(p.size, 944_000_000);
    }

    #[test]
    fn rotated_phone_video_is_portrait() {
        let p = parse(ROTATED, 1).unwrap();
        assert_eq!((p.width, p.height), (1080, 1920));
        // avg_frame_rate 30000/1001
        assert!((p.fps - 29.97).abs() < 0.01);
        assert_eq!(p.title, None);
    }

    #[test]
    fn two_audio_streams_are_listed() {
        let p = parse(TWO_AUDIO, 1).unwrap();
        let got: Vec<_> = p
            .audio
            .iter()
            .map(|a| (a.index, a.codec.as_str()))
            .collect();
        assert_eq!(got, [(1, "aac"), (2, "opus")]);
        // avg_frame_rate 0/0 falls back to r_frame_rate; upper-case TITLE tag (Matroska) is found
        assert_eq!(p.fps, 60.0);
        assert_eq!(p.title.as_deref(), Some("Valorant"));
    }

    #[test]
    fn no_video_is_refused() {
        assert_eq!(parse(NO_VIDEO, 1), Err(ProbeError::NoVideo));
        assert_eq!(
            ProbeError::NoVideo.to_string(),
            "Pulse couldn't read that video."
        );
    }

    #[test]
    fn cover_art_is_not_a_video() {
        let json = include_str!("../../tests/fixtures/clips/cover_art.json");
        assert_eq!(parse(json, 1), Err(ProbeError::NoVideo));
    }

    #[test]
    fn garbage_is_unreadable() {
        assert_eq!(parse("not json", 1), Err(ProbeError::Unreadable));
        assert_eq!(parse("{}", 1), Err(ProbeError::NoVideo));
    }
}
