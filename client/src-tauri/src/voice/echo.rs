//! Echo cancellation: Auto / On / Off (#76, noise-suppression design §9). Auto, the default,
//! turns it off when the output is headphones or a headset (nothing from the speakers can reach
//! the mic, and the canceller only costs CPU and can chop speech), and on otherwise, including when
//! the device doesn't say what it is. Switching the output device re-evaluates.
//!
//! What the output is comes from the system: on Windows the endpoint's form factor
//! (`PKEY_AudioEndpoint_FormFactor`, read by cpal into the device description); on Linux the sound
//! server's `device.form_factor` property of the sink, when it has one.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EchoMode {
    #[default]
    Auto,
    On,
    Off,
}

/// What the output device is, as far as the system says.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Output {
    /// Headphones or a headset: nothing it plays reaches the mic.
    Headphones,
    /// Speakers, a monitor, line out and the like.
    Other,
    /// The system doesn't say (or no output is open yet).
    #[default]
    Unknown,
}

/// Whether echo cancellation runs.
pub fn echo_on(mode: EchoMode, output: Output) -> bool {
    match mode {
        EchoMode::On => true,
        EchoMode::Off => false,
        // unknown keeps it on: leave things as they were
        EchoMode::Auto => output != Output::Headphones,
    }
}

/// cpal's device type (Windows: the endpoint form factor).
pub fn from_device_type(t: cpal::DeviceType) -> Output {
    match t {
        cpal::DeviceType::Headphones | cpal::DeviceType::Headset => Output::Headphones,
        cpal::DeviceType::Unknown => Output::Unknown,
        _ => Output::Other,
    }
}

/// A sound server's `device.form_factor` (PulseAudio's names; PipeWire's pulse server uses them too).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn from_form_factor(v: &str) -> Output {
    match v.trim() {
        "headphone" | "headset" => Output::Headphones,
        "" => Output::Unknown,
        _ => Output::Other,
    }
}

/// What the opened output `dev` is. Blocking (asks the system); call it off the audio callbacks.
pub fn output_of(dev: &cpal::Device) -> Output {
    use cpal::traits::DeviceTrait;
    let described = dev
        .description()
        .map(|d| from_device_type(d.device_type()))
        .unwrap_or_default();
    if described != Output::Unknown {
        return described;
    }
    #[cfg(target_os = "linux")]
    if let Ok(id) = dev.id()
        && id.host() == cpal::HostId::PulseAudio
    {
        return linux::sink_form_factor(id.id());
    }
    described
}

#[cfg(target_os = "linux")]
mod linux {
    use super::{Output, from_form_factor};
    use std::ffi::CString;
    use std::time::Duration;

    /// The sound server may be slow or hung: never hold the device opening up for long.
    const ASK_FOR: Duration = Duration::from_millis(500);

    /// The `device.form_factor` of sink `name` (PipeWire also offers `device.form-factor`).
    pub fn sink_form_factor(name: &str) -> Output {
        let Ok(name) = CString::new(name) else {
            return Output::Unknown;
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let _ = std::thread::Builder::new()
            .name("pulse-form-factor".into())
            .spawn(move || {
                let ask = || -> Option<String> {
                    let client = pulseaudio::Client::from_env(c"pulse").ok()?;
                    let info = futures::executor::block_on(client.sink_info_by_name(name)).ok()?;
                    [c"device.form_factor", c"device.form-factor"]
                        .iter()
                        .find_map(|k| info.props.get_bytes(*k))
                        .map(|v| {
                            String::from_utf8_lossy(v)
                                .trim_end_matches('\0')
                                .to_string()
                        })
                };
                let _ = tx.send(ask());
            });
        match rx.recv_timeout(ASK_FOR) {
            Ok(Some(v)) => from_form_factor(&v),
            _ => Output::Unknown,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_is_off_on_headphones_and_on_otherwise() {
        assert!(!echo_on(EchoMode::Auto, Output::Headphones));
        assert!(echo_on(EchoMode::Auto, Output::Other));
        assert!(
            echo_on(EchoMode::Auto, Output::Unknown),
            "unknown leaves it as it was"
        );
    }

    #[test]
    fn on_and_off_ignore_the_device() {
        for o in [Output::Headphones, Output::Other, Output::Unknown] {
            assert!(echo_on(EchoMode::On, o));
            assert!(!echo_on(EchoMode::Off, o));
        }
    }

    #[test]
    fn windows_headphones_and_headsets_count_as_headphones() {
        assert_eq!(
            from_device_type(cpal::DeviceType::Headphones),
            Output::Headphones
        );
        assert_eq!(
            from_device_type(cpal::DeviceType::Headset),
            Output::Headphones
        );
        assert_eq!(from_device_type(cpal::DeviceType::Speaker), Output::Other);
        assert_eq!(from_device_type(cpal::DeviceType::Unknown), Output::Unknown);
    }

    #[test]
    fn sound_server_form_factors() {
        assert_eq!(from_form_factor("headphone"), Output::Headphones);
        assert_eq!(from_form_factor("headset"), Output::Headphones);
        assert_eq!(from_form_factor("speaker"), Output::Other);
        assert_eq!(from_form_factor("internal"), Output::Other);
        assert_eq!(from_form_factor(""), Output::Unknown);
    }

    /// By hand, on a machine with a sound server: what each output says it is.
    #[test]
    #[ignore = "needs real audio devices"]
    fn real_outputs() {
        use cpal::traits::HostTrait;
        for d in cpal::default_host().output_devices().unwrap() {
            println!(
                "{:?}: {:?}",
                super::super::devices::device_name(&d),
                output_of(&d)
            );
        }
    }

    #[test]
    fn the_mode_reads_and_writes_lowercase() {
        assert_eq!(
            serde_json::from_str::<EchoMode>(r#""auto""#).unwrap(),
            EchoMode::Auto
        );
        assert_eq!(serde_json::to_string(&EchoMode::Off).unwrap(), r#""off""#);
        assert_eq!(EchoMode::default(), EchoMode::Auto);
    }
}
