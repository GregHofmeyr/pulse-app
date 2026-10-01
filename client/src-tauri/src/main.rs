// No console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // `pulse-app --toggle-mute` / `--toggle-deafen`: poke the running app and exit (Linux hotkeys).
    let args: Vec<String> = std::env::args().collect();
    if let Some(cmd) = pulse_client::ipc::Command::from_args(&args) {
        #[cfg(unix)]
        match pulse_client::ipc::send(&pulse_client::ipc::socket_path(), cmd) {
            Ok(()) => std::process::exit(0),
            Err(e) => {
                eprintln!("Pulse isn't running ({e})");
                std::process::exit(1);
            }
        }
        #[cfg(not(unix))]
        {
            let _ = cmd;
            eprintln!("use the in-app shortcuts (Ctrl+Shift+M / Ctrl+Shift+D) on this platform");
            std::process::exit(1);
        }
    }
    pulse_client::run();
}
