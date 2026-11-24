//! A bug report's diagnostics beyond the basics (#605): the voice settings in effect, the formats the audio devices
//! opened, the stream state the page knows, and what netwatch knows about the network. Shapes only: gathering is
//! [`super::meta`]. Nothing here carries an address, SSID, hostname, user name or path (device names are fine: the
//! report already had them).

use serde::{Deserialize, Serialize};

use crate::voice::ReportView;
use crate::voice::denoise::NsLevel;
use crate::voice::devices::OpenedFormat;
use crate::voice::echo::{EchoMode, Output};

/// The settings voice runs with, and what its processing actually does.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VoiceSettings {
    pub noise_suppression: NsLevel,
    /// What runs, where the processor has said; differs from the setting when it stepped down.
    pub noise_suppression_active: Option<NsLevel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub noise_suppression_note: Option<String>,
    pub gain_pct: u16,
    pub auto_gain: bool,
    pub gate: Gate,
    pub echo_cancellation: EchoMode,
    pub echo_cancellation_running: Option<bool>,
    /// What the system says the output is (it no longer decides anything: #354).
    pub output_kind: Option<Output>,
    /// "Even out voice volumes".
    pub even_out_voices: bool,
    /// Push-to-talk doesn't exist yet: the mic sends on voice activity.
    pub transmit: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mic_bitrate: Option<MicBitrate>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Gate {
    /// On detected speech above the room's noise floor; else on `threshold`.
    pub auto: bool,
    /// The manual gate's RMS threshold (0 = always open).
    pub threshold: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MicBitrate {
    /// The channel's setting; `None` = Default.
    pub channel_kbps: Option<u32>,
    /// What the mic is capped at from it.
    pub max_bps: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VoiceReport {
    /// `in_call`, `not_in_call` (settings are then a rejoin's, if one is under way) or `busy`.
    pub state: &'static str,
    pub settings: Option<VoiceSettings>,
}

pub fn voice_report(v: &ReportView) -> VoiceReport {
    let settings = v.cfg.as_ref().map(|c| VoiceSettings {
        noise_suppression: c.noise_suppression,
        noise_suppression_active: v.ns.as_ref().map(|n| n.active),
        noise_suppression_note: v.ns.as_ref().and_then(|n| n.note.clone()),
        gain_pct: c.input_gain_pct,
        auto_gain: c.auto_gain,
        gate: Gate {
            auto: c.auto_sensitivity,
            // f32 → f64 prints 0.019999999552965164; four places is all it ever had
            threshold: (f64::from(c.sensitivity) * 10_000.0).round() / 10_000.0,
        },
        echo_cancellation: c.echo_mode,
        echo_cancellation_running: v.echo.map(|e| e.running),
        output_kind: v.echo.map(|e| e.output),
        even_out_voices: c.even_out_voices,
        transmit: "voice_activity",
        mic_bitrate: v.mic_kbps.zip(v.mic_max_bps).map(|(k, max)| MicBitrate {
            channel_kbps: (k != 0).then_some(k),
            max_bps: max,
        }),
    });
    VoiceReport {
        state: v.state,
        settings,
    }
}

/// The devices a call opened (with their formats) and the system's defaults.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AudioDevices {
    pub input: Option<OpenedFormat>,
    pub output: Option<OpenedFormat>,
    pub default_input: Option<String>,
    pub default_output: Option<String>,
}

/// What the page knows and Rust doesn't, sent with a report: typed, so nothing else rides along.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReportUi {
    pub stream: Option<StreamUi>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StreamUi {
    /// Our own share, if we're live (never its window title).
    pub live: Option<LiveUi>,
    /// Streams being watched (`watching_ghost` of them from outside their voice channel).
    pub watching: u32,
    pub watching_ghost: u32,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LiveUi {
    pub height: u16,
    pub fps: u8,
    pub audio: bool,
    pub sound: Option<SoundSource>,
    pub source: Option<SourceKind>,
    pub reconnecting: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SoundSource {
    App,
    Everything,
    #[serde(other)]
    Unknown,
}

/// The capture's `displaySurface`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    Monitor,
    Window,
    Browser,
    #[serde(other)]
    Unknown,
}

/// What netwatch can say without an address: whether there's a route out over each family.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Network {
    pub ipv4_route: bool,
    pub ipv6_route: bool,
    #[serde(flatten)]
    pub events: super::tally::NetSummary,
    /// Voice rejoins this session (LiveKit gave up and we joined again).
    pub voice_rejoins: u32,
}

pub fn network(route: crate::netwatch::Route, voice_rejoins: u32) -> Network {
    Network {
        ipv4_route: route.v4.is_some(),
        ipv6_route: route.v6.is_some(),
        events: super::tally::net(),
        voice_rejoins,
    }
}

/// The server refuses a meta over 16 KB, and a crash report adds up to ~2 KB to this one: keep the base under
/// [`META_BUDGET`] by dropping the least needed extras first.
pub const META_BUDGET: usize = 12 * 1024;
const DROP_ORDER: [&str; 5] = ["warnings", "network", "system", "audio_devices", "streams"];

pub fn fit(meta: &mut serde_json::Value, budget: usize) {
    for key in DROP_ORDER {
        if meta.to_string().len() <= budget {
            return;
        }
        if let Some(m) = meta.as_object_mut()
            && m.remove(key).is_some()
        {
            m.insert(
                key.into(),
                serde_json::json!("dropped: report details too large"),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voice::devices::{AudioConfig, EchoStatus, NsStatus};

    fn in_call() -> ReportView {
        ReportView {
            state: "in_call",
            cfg: Some(AudioConfig {
                input: Some("Headset Mic (alex's)".into()),
                input_gain_pct: 150,
                sensitivity: 0.02,
                auto_sensitivity: false,
                auto_gain: true,
                noise_suppression: NsLevel::StrongLowDelay,
                ..AudioConfig::default()
            }),
            ns: Some(NsStatus {
                active: NsLevel::Standard,
                note: Some("Strong unavailable, using Standard".into()),
            }),
            echo: Some(EchoStatus {
                mode: EchoMode::Auto,
                running: true,
                output: Output::Headphones,
            }),
            mic_kbps: Some(0),
            mic_max_bps: Some(64_000),
            input: Some(OpenedFormat {
                device: "Headset Mic".into(),
                rate: 48_000,
                channels: 1,
                sample_format: "f32".into(),
            }),
            output: None,
        }
    }

    #[test]
    fn voice_settings_shape() {
        let v = serde_json::to_value(voice_report(&in_call())).unwrap();
        assert_eq!(
            v,
            serde_json::json!({
                "state": "in_call",
                "settings": {
                    "noise_suppression": "strong_low_delay",
                    "noise_suppression_active": "standard",
                    "noise_suppression_note": "Strong unavailable, using Standard",
                    "gain_pct": 150,
                    "auto_gain": true,
                    "gate": { "auto": false, "threshold": 0.02 },
                    "echo_cancellation": "auto",
                    "echo_cancellation_running": true,
                    "output_kind": "headphones",
                    "even_out_voices": true,
                    "transmit": "voice_activity",
                    "mic_bitrate": { "channel_kbps": null, "max_bps": 64000 },
                }
            })
        );
        // the device names belong to `audio_devices` / `voice`, not here
        assert!(!v.to_string().contains("Headset"));
    }

    #[test]
    fn a_channel_bitrate_is_reported_as_set() {
        let mut view = in_call();
        view.mic_kbps = Some(96);
        view.mic_max_bps = Some(96_000);
        let r = voice_report(&view);
        assert_eq!(
            r.settings.unwrap().mic_bitrate,
            Some(MicBitrate {
                channel_kbps: Some(96),
                max_bps: 96_000
            })
        );
    }

    #[test]
    fn out_of_a_call_there_are_no_settings() {
        let v = serde_json::to_value(voice_report(&ReportView {
            state: "not_in_call",
            ..Default::default()
        }))
        .unwrap();
        assert_eq!(
            v,
            serde_json::json!({ "state": "not_in_call", "settings": null })
        );
    }

    #[test]
    fn devices_shape() {
        let view = in_call();
        let d = AudioDevices {
            input: view.input.clone(),
            output: None,
            default_input: Some("Headset Mic".into()),
            default_output: None,
        };
        assert_eq!(
            serde_json::to_value(&d).unwrap(),
            serde_json::json!({
                "input": { "device": "Headset Mic", "rate": 48000, "channels": 1, "sample_format": "f32" },
                "output": null,
                "default_input": "Headset Mic",
                "default_output": null,
            })
        );
    }

    #[test]
    fn the_pages_stream_state_is_typed_and_drops_anything_else() {
        let ui: ReportUi = serde_json::from_value(serde_json::json!({
            "stream": {
                "live": { "height": 1080, "fps": 60, "audio": true, "sound": "app", "source": "window",
                          "label": "Secret window title", "channelId": "123" },
                "watching": 1,
                "watching_ghost": 0,
            },
            "extra": "/home/alex/notes.txt",
        }))
        .unwrap();
        let back = serde_json::to_value(&ui).unwrap();
        let s = back.to_string();
        assert!(
            !s.contains("Secret") && !s.contains("alex") && !s.contains("123"),
            "{s}"
        );
        assert_eq!(back["stream"]["live"]["source"], "window");
        assert_eq!(back["stream"]["watching"], 1);
    }

    #[test]
    fn unknown_words_from_the_page_dont_fail_the_report() {
        let ui: ReportUi = serde_json::from_value(serde_json::json!({
            "stream": { "live": { "height": 720, "fps": 30, "source": "application", "sound": "nope" } }
        }))
        .unwrap();
        let live = ui.stream.unwrap().live.unwrap();
        assert_eq!(live.source, Some(SourceKind::Unknown));
        assert_eq!(live.sound, Some(SoundSource::Unknown));
        // and nothing at all is fine too
        assert_eq!(
            serde_json::from_value::<ReportUi>(serde_json::json!({})).unwrap(),
            ReportUi::default()
        );
    }

    #[test]
    fn network_has_no_addresses() {
        let route = crate::netwatch::Route {
            v4: Some(std::net::Ipv4Addr::new(192, 168, 1, 23)),
            v6: None,
        };
        let v = serde_json::to_value(network(route, 2)).unwrap();
        assert_eq!(v["ipv4_route"], true);
        assert_eq!(v["ipv6_route"], false);
        assert_eq!(v["voice_rejoins"], 2);
        assert!(v["gateway_reconnects"].is_number());
        assert!(!v.to_string().contains("192.168"), "{v}");
    }

    #[test]
    fn fit_drops_extras_until_it_fits() {
        let mut meta = serde_json::json!({
            "app_version": "0.1.0",
            "warnings": "w".repeat(5000),
            "network": "n".repeat(5000),
            "system": { "uptime_s": 1 },
        });
        fit(&mut meta, 6000);
        assert_eq!(meta["app_version"], "0.1.0");
        assert!(meta["warnings"].as_str().unwrap().starts_with("dropped"));
        assert_eq!(meta["network"].as_str().unwrap().len(), 5000);
        assert!(meta.to_string().len() <= 6000);

        let mut small = serde_json::json!({ "warnings": { "warn": 1 } });
        fit(&mut small, META_BUDGET);
        assert_eq!(small["warnings"]["warn"], 1);
    }
}
