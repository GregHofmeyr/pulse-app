pub mod api;
pub mod backoff;
pub mod bindmatch;
pub mod clips;
pub mod commands;
pub mod gateway;
pub mod hookwatch;
pub mod hotkeys;
pub mod ipc;
pub mod keybinds;
pub mod logging;
pub mod media;
pub mod outbox;
pub mod qr;
pub mod reports;
pub mod session;
pub mod sounds;
pub mod startup;
pub mod stream;
pub mod throttle;
pub mod voice;
pub mod webview_memory;

/// Keeps the log writer alive (dropping it flushes and stops logging).
struct LogGuard(
    #[allow(dead_code)] std::sync::Mutex<Option<tracing_appender::non_blocking::WorkerGuard>>,
);

static FOCUS_LOG_BUDGET: std::sync::Mutex<Option<logging::LineBudget>> =
    std::sync::Mutex::new(None);

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
    // Focus decides message sounds and read marking: log each change the window reports (the page
    // logs what it holds, and why each message did or didn't ding), so a missed sound can be told
    // apart from a focus the window never lost.
    win.on_window_event(|e| {
        if let tauri::WindowEvent::Focused(focused) = e {
            // budgeted: focus can flip often (alt-tabbing, focus moving inside the window)
            logging::log_budgeted(
                &FOCUS_LOG_BUDGET,
                20,
                1.0,
                || tracing::info!(focused, "window focus"),
                |n| tracing::info!("window focus: {n} changes not logged (too many)"),
            );
        }
    });
    // WebView2: the Low memory target while minimised or in the background.
    #[cfg(windows)]
    webview_memory::install(&win);
    Ok(())
}

pub fn run() {
    startup::begin();
    let mut builder = tauri::Builder::default();
    // Must be the first plugin: a second launch exits before anything else starts.
    if session::single_instance(std::env::var("PULSE_PROFILE").ok().as_deref()) {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            use tauri::Manager;
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.unminimize();
                let _ = win.show();
                let _ = win.set_focus();
            }
        }));
    }
    builder
        .setup(|app| {
            use tauri::Manager;
            let profile = std::env::var("PULSE_PROFILE").ok();
            let dir = session::profile_dir(&app.path().app_data_dir()?, profile.as_deref());
            // Log file per profile; the guard must live as long as the app.
            let log_dir = session::profile_dir(&app.path().app_log_dir()?, profile.as_deref());
            // Last session's crash / unexpected exit, captured before this session writes to the log.
            reports::crash::detect(&log_dir, reports::unix_now());
            reports::crash::mark_running(&log_dir);
            // "Send crash reports automatically" off: what was just captured waits for the user.
            let report_settings = reports::settings::Store::load(&dir);
            if !report_settings.auto_send_crashes() {
                reports::crash::hold_pending(&log_dir);
            }
            app.manage(report_settings);
            app.manage(reports::LogDir(log_dir.clone()));
            match logging::init(&log_dir) {
                Ok(guard) => {
                    app.manage(LogGuard(std::sync::Mutex::new(Some(guard))));
                }
                Err(e) => eprintln!("logging unavailable ({}): {e}", log_dir.display()),
            }
            tracing::info!(profile = ?profile, data = %logging::redact_home(&dir), "profile");
            // Loaded (and acted on) above, before logging was up: report how that went now.
            app.state::<reports::settings::Store>().log_loaded();
            startup::mark("logging_ready"); // (since process start: covers crash checks + log setup)
            let webview_dir = session::profile_dir(&app.path().app_local_data_dir()?, profile.as_deref());
            create_main_window(app, &webview_dir)?;
            startup::mark("window_built");
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
            let media_root =
                session::profile_dir(&app.path().app_cache_dir()?, profile.as_deref()).join("media");
            let h = app.handle().clone();
            app.manage(commands::Core::new(
                session::Store::new(dir.clone(), profile.as_deref()),
                outbox,
                media_root,
                Box::new(move || media::clear_webview_cache(&h)),
            ));
            app.manage(voice::mictest::MicTest::default());
            app.manage(stream::PickerMemory::default());
            let h = app.handle().clone();
            app.manage(stream::audio::StreamAudio::new(std::sync::Arc::new(
                move |channel_id| {
                    use tauri::Emitter;
                    let _ = h.emit(
                        "stream://audio_closed",
                        serde_json::json!({ "channel_id": channel_id }),
                    );
                },
            )));
            #[cfg(unix)]
            {
                let h = app.handle().clone();
                match ipc::socket_path() {
                    // Needs a runtime: run the listener on Tauri's.
                    Ok(path) => {
                        tauri::async_runtime::spawn(async move {
                            if let Err(e) = ipc::serve(&path, move |c| commands::hotkey(&h, c)) {
                                tracing::warn!(path = %logging::redact_home(&path), error = %e, "hotkey socket unavailable");
                            }
                        });
                    }
                    Err(e) => tracing::warn!(error = %e, "hotkey socket unavailable"),
                }
            }
            let binds = keybinds::Store::load(&dir);
            let saved = binds.get();
            binds.set_problems(hotkeys::install(app, &saved.binds, saved.enabled)?);
            app.manage(binds);
            let handle = app.handle().clone();
            app.manage(voice::VoiceManager::new(std::sync::Arc::new(
                move |e: voice::VoiceEvent| {
                    use tauri::Emitter;
                    let _ = handle.emit("voice://event", &e);
                },
            )));
            // voice comes back by itself after LiveKit gives up reconnecting
            let h = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                h.state::<voice::VoiceManager>().run_rejoins().await
            });
            // A stored login connects now, while the webview boots: Ready no longer waits for
            // the UI and a separate /me round trip. (Keychain reads block: not on the UI thread.)
            let h = app.handle().clone();
            tauri::async_runtime::spawn_blocking(move || {
                h.state::<commands::Core>().start_stored(&h, false);
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            // Back at the app (or woken from sleep): a dead connection should show now, not at the
            // next heartbeat.
            if let tauri::WindowEvent::Focused(true) = event {
                use tauri::Manager;
                if let Some(core) = window.try_state::<commands::Core>() {
                    core.probe_gateway();
                }
            }
        })
        .register_asynchronous_uri_scheme_protocol(media::SCHEME, media::handle)
        .invoke_handler(tauri::generate_handler![
            commands::login,
            commands::login_mfa,
            commands::cancel_login,
            commands::register,
            commands::restore_session,
            commands::logout,
            commands::logout_others,
            commands::change_password,
            commands::two_factor_status,
            commands::two_factor_setup,
            commands::two_factor_enable,
            commands::two_factor_disable,
            commands::two_factor_regenerate,
            commands::list_servers,
            commands::gateway_probe,
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
            commands::prefetch_voice,
            commands::toggle_mute,
            commands::toggle_deafen,
            commands::set_peer_volume,
            commands::set_audio_config,
            commands::list_audio_devices,
            commands::start_mic_test,
            commands::stop_mic_test,
            commands::watch_voice_meters,
            commands::watch_voice_link,
            commands::play_sound,
            commands::create_dm,
            commands::add_members,
            commands::remove_member,
            commands::rename_channel,
            commands::mark_read,
            commands::set_mute,
            commands::clear_mute,
            commands::close_conversation,
            commands::upload_emoji_media,
            commands::create_emoji,
            commands::rename_emoji,
            commands::delete_emoji,
            commands::add_reaction,
            commands::remove_reaction,
            commands::list_reactors,
            // quotes
            commands::list_messages_around,
            commands::list_messages_after,
            commands::upload_quote_media,
            commands::post_quote,
            commands::edit_quote,
            commands::list_replies,
            commands::send_reply,
            commands::mark_replies_seen,
            commands::give_award,
            commands::take_award,
            commands::link_speaker,
            commands::unlink_speaker,
            commands::leaderboard,
            commands::search_quotes,
            commands::start_qoty,
            commands::vote_qoty,
            commands::unvote_qoty,
            commands::add_qoty_option,
            commands::end_qoty,
            commands::reopen_qoty,
            // clips
            commands::add_game,
            commands::edit_clip,
            commands::clip_url,
            commands::list_clips,
            commands::clip_facets,
            commands::set_clip_view,
            commands::clip_storage,
            startup::ui_painted,
            keybinds::get_keybinds,
            keybinds::set_keybinds,
            keybinds::set_keybinds_enabled,
            keybinds::pause_keybinds,
            reports::submit_report,
            reports::save_report,
            reports::crash::send_held_report,
            reports::crash::discard_held_report,
            reports::settings::get_report_settings,
            reports::settings::set_report_settings,
            stream::stream_ticket,
            stream::stream_go_live,
            stream::stream_reannounce,
            stream::stream_stop,
            stream::stream_watch,
            stream::stream_unwatch,
            stream::stream_preview_put,
            stream::stream_preview_get,
            stream::stream_log,
            logging::ui_log,
            stream::stream_picker_window,
            stream::stream_audio_start,
            stream::stream_audio_stop,
            stream::stream_audio_leave,
            stream::stream_audio_mute,
            stream::stream_audio_watch,
            stream::set_stream_volume,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Pulse")
        .run(|app, event| {
            // Quitting leaves voice properly, so others don't see a ghost until LiveKit times it
            // out. (Minimising/hiding isn't an exit: you stay in voice.)
            if let tauri::RunEvent::Exit = event {
                use tauri::Manager;
                let voice = app.state::<voice::VoiceManager>();
                let stream_audio = app.state::<stream::audio::StreamAudio>();
                tauri::async_runtime::block_on(async {
                    // side by side: one slow close must not use up the other's budget
                    let leave = async {
                        tokio::join!(stream_audio.leave(), voice.leave());
                    };
                    if tokio::time::timeout(std::time::Duration::from_secs(2), leave)
                        .await
                        .is_err()
                    {
                        tracing::warn!("voice: leave on exit timed out");
                    }
                });
                reports::crash::clear_running(&app.state::<reports::LogDir>().0);
            }
        });
}
