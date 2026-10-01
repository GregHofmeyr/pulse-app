pub mod api;
pub mod backoff;
pub mod commands;
pub mod gateway;
pub mod ipc;
pub mod logging;
pub mod outbox;
pub mod session;
pub mod sounds;
pub mod voice;

/// Keeps the log writer alive (dropping it flushes and stops logging).
struct LogGuard(
    #[allow(dead_code)] std::sync::Mutex<Option<tracing_appender::non_blocking::WorkerGuard>>,
);

/// The main window is built here rather than in tauri.conf.json so each profile gets its own
/// webview storage (settings, per-user volumes) and so we can recover if the renderer dies.
fn create_main_window(app: &tauri::App, profile_dir: &std::path::Path) -> tauri::Result<()> {
    let win = tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::default())
        .title("Pulse")
        .inner_size(1280.0, 800.0)
        .min_inner_size(900.0, 560.0)
        .decorations(false)
        .background_color(tauri::window::Color(0x14, 0x15, 0x19, 0xff))
        .data_directory(profile_dir.join("webview"))
        .build()?;
    // WebKitGTK: if the page renderer crashes, reload instead of leaving a dead window.
    #[cfg(target_os = "linux")]
    win.with_webview(|wv| {
        use webkit2gtk::WebViewExt;
        wv.inner().connect_web_process_terminated(|view, reason| {
            tracing::error!(?reason, "web renderer died; reloading");
            view.reload();
        });
    })?;
    #[cfg(not(target_os = "linux"))]
    let _ = win;
    Ok(())
}

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            use tauri::Manager;
            let profile = std::env::var("PULSE_PROFILE").ok();
            let dir = session::profile_dir(&app.path().app_data_dir()?, profile.as_deref());
            // Log file per profile; the guard must live as long as the app.
            let log_dir = session::profile_dir(&app.path().app_log_dir()?, profile.as_deref());
            match logging::init(&log_dir) {
                Ok(guard) => {
                    app.manage(LogGuard(std::sync::Mutex::new(Some(guard))));
                }
                Err(e) => eprintln!("logging unavailable ({}): {e}", log_dir.display()),
            }
            tracing::info!(profile = ?profile, data = %dir.display(), "profile");
            create_main_window(app, &dir)?;
            let emitter = app.handle().clone();
            let outbox = outbox::Outbox::new(std::sync::Arc::new(
                move |nonce: &str,
                      status: outbox::Status,
                      message: Option<&pulse_protocol::rest::Message>| {
                    use tauri::Emitter;
                    let _ = emitter.emit(
                        "pulse://outbox",
                        serde_json::json!({ "nonce": nonce, "status": status, "message": message }),
                    );
                },
            ));
            app.manage(commands::Core::new(session::Store::new(dir), outbox));
            app.manage(voice::mictest::MicTest::default());
            #[cfg(unix)]
            {
                let h = app.handle().clone();
                let path = ipc::socket_path();
                // Needs a runtime: run the listener on Tauri's.
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = ipc::serve(&path, move |c| commands::hotkey(&h, c)) {
                        tracing::warn!(path = %path.display(), error = %e, "hotkey socket unavailable");
                    }
                });
            }
            #[cfg(windows)]
            {
                use tauri_plugin_global_shortcut::{
                    Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState,
                };
                let mute = Shortcut::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::KeyM);
                let deafen = Shortcut::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::KeyD);
                app.handle().plugin(
                    tauri_plugin_global_shortcut::Builder::new()
                        .with_handler(move |app, sc, ev| {
                            if ev.state() == ShortcutState::Pressed {
                                if sc == &mute {
                                    commands::hotkey(app, ipc::Command::ToggleMute);
                                } else if sc == &deafen {
                                    commands::hotkey(app, ipc::Command::ToggleDeafen);
                                }
                            }
                        })
                        .build(),
                )?;
                // Another app may already own these: log and carry on rather than refusing to start.
                for sc in [mute, deafen] {
                    if let Err(e) = app.global_shortcut().register(sc) {
                        tracing::warn!(shortcut = ?sc, error = %e, "couldn't register hotkey");
                    }
                }
            }
            let handle = app.handle().clone();
            app.manage(voice::VoiceManager::new(std::sync::Arc::new(
                move |e: voice::VoiceEvent| {
                    use tauri::Emitter;
                    let _ = handle.emit("voice://event", &e);
                },
            )));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::login,
            commands::register,
            commands::restore_session,
            commands::logout,
            commands::list_servers,
            commands::gateway_reconnect_now,
            commands::send_typing,
            commands::create_server,
            commands::create_channel,
            commands::join_server,
            commands::list_channels,
            commands::list_members,
            commands::list_messages,
            commands::send_message,
            commands::edit_message,
            commands::delete_message,
            commands::join_voice,
            commands::leave_voice,
            commands::toggle_mute,
            commands::toggle_deafen,
            commands::set_peer_volume,
            commands::set_audio_config,
            commands::list_audio_devices,
            commands::start_mic_test,
            commands::stop_mic_test,
            commands::play_sound,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Pulse");
}
