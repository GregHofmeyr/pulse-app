//! Virtual mics that already clean up their own sound (#588): SteelSeries Sonar, NVIDIA Broadcast, Krisp and the
//! like run their own noise suppression (and often a compressor and EQ) before Pulse sees a sample. Strong on top
//! is suppression on suppression, which carves the voice (the audit behind #586). Recognised by device name only,
//! so Voice settings can suggest Standard; nothing is changed automatically.

/// (lowercase fragment of the device name, the name shown to the person). First match wins.
const KNOWN: &[(&str, &str)] = &[
    ("sonar", "SteelSeries Sonar"),
    ("nvidia broadcast", "NVIDIA Broadcast"),
    ("rtx voice", "NVIDIA RTX Voice"),
    ("krisp", "Krisp"),
    ("voicemeeter", "Voicemeeter"),
    ("wave link", "Elgato Wave Link"),
    ("wavelink", "Elgato Wave Link"),
    ("goxlr", "GoXLR"),
    ("nahimic", "Nahimic"),
    ("razer synapse", "Razer Synapse"),
];

/// What already processes the mic named `device`, if it's one we know.
pub fn processed_by(device: &str) -> Option<&'static str> {
    let name = device.to_lowercase();
    KNOWN
        .iter()
        .find(|(fragment, _)| name.contains(fragment))
        .map(|(_, label)| *label)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_virtual_mics_are_recognised_whatever_the_case() {
        for (device, want) in [
            (
                "Microphone (SteelSeries Sonar - Microphone)",
                "SteelSeries Sonar",
            ),
            ("SONAR - MICROPHONE", "SteelSeries Sonar"),
            ("Microphone (NVIDIA Broadcast)", "NVIDIA Broadcast"),
            ("Microphone (NVIDIA RTX Voice)", "NVIDIA RTX Voice"),
            ("Microphone (Krisp Microphone)", "Krisp"),
            (
                "VoiceMeeter Output (VB-Audio VoiceMeeter VAIO)",
                "Voicemeeter",
            ),
            ("Wave Link Stream (Elgato Wave:3)", "Elgato Wave Link"),
            ("Chat Mic (TC-Helicon GoXLR)", "GoXLR"),
        ] {
            assert_eq!(processed_by(device), Some(want), "{device}");
        }
    }

    #[test]
    fn ordinary_mics_are_not() {
        for device in [
            "Microphone (Realtek(R) Audio)",
            "Headset Microphone (Arctis Nova 7)",
            "Microphone (Elgato Wave:3)",
            "Built-in Audio Analog Stereo",
            "",
        ] {
            assert_eq!(processed_by(device), None, "{device}");
        }
    }
}
