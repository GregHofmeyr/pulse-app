pub mod api;
pub mod backoff;
pub mod commands;
pub mod gateway;
pub mod ipc;
pub mod outbox;
pub mod session;
pub mod voice;

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            use tauri::Manager;
            let dir = session::profile_dir(
                &app.path().app_data_dir()?,
                std::env::var("PULSE_PROFILE").ok().as_deref(),
            );
            let emitter = app.handle().clone();
            let outbox = outbox::Outbox::new(std::sync::Arc::new(
                move |nonce: &str, status: outbox::Status| {
                    use tauri::Emitter;
                    let _ = emitter.emit(
                        "pulse://outbox",
                        serde_json::json!({ "nonce": nonce, "status": status }),
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
                        eprintln!("hotkey socket unavailable ({}): {e}", path.display());
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
                        eprintln!("couldn't register hotkey {sc:?}: {e}");
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running Pulse");
}
