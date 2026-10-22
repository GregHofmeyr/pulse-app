pub mod api;
pub mod backoff;
pub mod bindmatch;
pub mod commands;
pub mod gateway;
pub mod hookwatch;
pub mod hotkeys;
pub mod ipc;
pub mod keybinds;
pub mod logging;
pub mod outbox;
pub mod reports;
pub mod session;
pub mod sounds;
#[cfg(feature = "spike")]
pub mod spike;
pub mod startup;
pub mod throttle;
pub mod voice;
pub mod webview_memory;

/// Keeps the log writer alive (dropping it flushes and stops logging).
struct LogGuard(
    #[allow(dead_code)] std::sync::Mutex<Option<tracing_appender::non_blocking::WorkerGuard>>,
);

/// The main window is built here rather than in tauri.conf.json so each profile gets its own
/// webview storage (settings, per-user volumes) and so we can recover if the renderer dies.
fn create_main_window(app: &tauri::App, profile_dir: &std::path::Path) -> tauri::Result<()> {
    let b = tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::default())
        .title("Pulse")
        .inner_size(1280.0, 800.0)
        .min_inner_size(900.0, 560.0)
        .decorations(false)
        .background_color(tauri::window::Color(0x14, 0x15, 0x19, 0xff))
        .data_directory(profile_dir.join("webview"));
    // THROWAWAY (spike): extra WebView2 browser args from PULSE_WEBVIEW2_ARGS (S1 HW off, S6, S7).
    #[cfg(all(windows, feature = "spike"))]
    let b = b.additional_browser_args(&spike::webview_args(
        // wry 0.57's default (webview2/mod.rs): keep it, or these features come back on
        "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection",
        std::env::var("PULSE_WEBVIEW2_ARGS").ok().as_deref(),
    ));
    let win = b.build()?;
    // WebKitGTK: if the page renderer crashes, reload instead of leaving a dead window.
    #[cfg(target_os = "linux")]
    win.with_webview(|wv| {
        use webkit2gtk::WebViewExt;
        wv.inner().connect_web_process_terminated(|view, reason| {
            tracing::error!(?reason, "web renderer died; reloading");
            view.reload();
        });
    })?;
    // WebView2: the Low memory target while minimised or in the background.
    #[cfg(windows)]
    webview_memory::install(&win);
    // THROWAWAY (spike S12): log WebView2 renderer/browser process failures.
    #[cfg(all(windows, feature = "spike"))]
    win.with_webview(|wv| {
        use webview2_com::Microsoft::Web::WebView2::Win32::COREWEBVIEW2_PROCESS_FAILED_KIND;
        use webview2_com::ProcessFailedEventHandler;
        let Ok(core) = (unsafe { wv.controller().CoreWebView2() }) else {
            tracing::warn!(target: "spike", "spike: no CoreWebView2 for the ProcessFailed hook");
            return;
        };
        let handler = ProcessFailedEventHandler::create(Box::new(|_, args| {
            let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND::default();
            let kind = match args {
                Some(a) if unsafe { a.ProcessFailedKind(&mut kind) }.is_ok() => kind.0,
                _ => -1,
            };
            tracing::error!(
                target: "spike",
                "spike: {{\"test\":\"S12\",\"event\":\"renderer_failed\",\"kind\":{kind}}}"
            );
            Ok(())
        }));
        let mut token = 0i64;
        let _ = unsafe { core.add_ProcessFailed(&handler, &mut token) };
    })?;
    #[cfg(not(any(target_os = "linux", windows)))]
    let _ = win;
    Ok(())
}

/// Every IPC command, plus any extra ones (the spike build's) appended at the end.
macro_rules! handlers {
    ($($extra:tt)*) => {
        tauri::generate_handler![
            commands::login,
            commands::register,
            commands::restore_session,
            commands::logout,
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
            commands::play_sound,
            commands::create_dm,
            commands::add_members,
            commands::remove_member,
            commands::rename_channel,
            commands::mark_read,
            commands::set_mute,
            commands::clear_mute,
            commands::close_conversation,
            startup::ui_painted,
            keybinds::get_keybinds,
            keybinds::set_keybinds,
            keybinds::pause_keybinds,
            reports::submit_report,
            reports::save_report,
            $($extra)*
        ]
    };
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
    let builder = builder
        .setup(|app| {
            use tauri::Manager;
            let profile = std::env::var("PULSE_PROFILE").ok();
            let dir = session::profile_dir(&app.path().app_data_dir()?, profile.as_deref());
            // Log file per profile; the guard must live as long as the app.
            let log_dir = session::profile_dir(&app.path().app_log_dir()?, profile.as_deref());
            // Last session's crash / unexpected exit, captured before this session writes to the log.
            reports::crash::detect(&log_dir, reports::unix_now());
            reports::crash::mark_running(&log_dir);
            app.manage(reports::LogDir(log_dir.clone()));
            match logging::init(&log_dir) {
                Ok(guard) => {
                    app.manage(LogGuard(std::sync::Mutex::new(Some(guard))));
                }
                Err(e) => eprintln!("logging unavailable ({}): {e}", log_dir.display()),
            }
            tracing::info!(profile = ?profile, data = %dir.display(), "profile");
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
            app.manage(commands::Core::new(session::Store::new(dir.clone(), profile.as_deref()), outbox));
            app.manage(voice::mictest::MicTest::default());
            #[cfg(feature = "spike")]
            app.manage(spike::loopback::Loopback::default());
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
            let binds = keybinds::Store::load(&dir);
            binds.set_problems(hotkeys::install(app, &binds.get())?);
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
        });
    // `generate_handler!` can't take `cfg` on single entries: spike builds append theirs here.
    #[cfg(feature = "spike")]
    let builder = builder.invoke_handler(handlers![
        spike::spike_log,
        spike::spike_ticket,
        spike::spike_os_stats,
        spike::spike_window_pid,
        spike::loopback::spike_loopback_start,
        spike::loopback::spike_loopback_stop,
        spike::loopback::spike_beep_watch,
    ]);
    #[cfg(not(feature = "spike"))]
    let builder = builder.invoke_handler(handlers![]);
    builder
        .build(tauri::generate_context!())
        .expect("error while building Pulse")
        .run(|app, event| {
            // Quitting leaves voice properly, so others don't see a ghost until LiveKit times it
            // out. (Minimising/hiding isn't an exit: you stay in voice.)
            if let tauri::RunEvent::Exit = event {
                use tauri::Manager;
                let voice = app.state::<voice::VoiceManager>();
                tauri::async_runtime::block_on(async {
                    let leave = voice.leave();
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
