pub mod api;
pub mod attachments;
pub mod backoff;
pub mod bindmatch;
pub mod clips;
pub mod commands;
pub mod embed_window;
pub mod gateway;
pub mod gifs;
pub mod hookwatch;
pub mod hotkeys;
pub mod ipc;
pub mod keybinds;
pub mod links;
pub mod logging;
pub mod media;
pub mod models;
pub mod notify;
pub mod outbox;
pub mod qr;
pub mod reports;
pub mod session;
pub mod soundboard;
pub mod sounds;
pub mod startup;
pub mod stream;
pub mod throttle;
pub mod updater;
pub mod video;
pub mod voice;
pub mod webview_crash;
pub mod webview_memory;
pub mod window_bg;

/// Keeps the log writer alive (dropping it flushes and stops logging).
struct LogGuard(
    #[allow(dead_code)] std::sync::Mutex<Option<tracing_appender::non_blocking::WorkerGuard>>,
);

static FOCUS_LOG_BUDGET: std::sync::Mutex<Option<logging::LineBudget>> =
    std::sync::Mutex::new(None);

/// The main window is built here rather than in tauri.conf.json so each profile gets its own
/// webview storage (settings, per-user volumes) and so we can recover if the renderer dies.
fn create_main_window(
    app: &tauri::App,
    data_dir: &std::path::Path,
    background: tauri::window::Color,
) -> tauri::Result<()> {
    let win = tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::default())
        .title("Pulse")
        .inner_size(1280.0, 800.0)
        .min_inner_size(900.0, 560.0)
        .decorations(false)
        // The theme's window colour from last time (window_bg), so a Light start doesn't flash dark.
        .background_color(background)
        .data_directory(data_dir.to_path_buf())
        // Links (target=_blank) and a page navigating to a website open in the browser instead.
        .on_new_window(|u, _| links::new_window(u))
        .on_navigation(links::main_navigation)
        .build()?;
    // If the page's process dies, reload (or restart) instead of leaving a dead window.
    #[cfg(any(target_os = "linux", windows))]
    webview_crash::install(&win)?;
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

/// Unminimise, show and focus the main window: a second launch, or a clicked notification.
fn bring_to_front(app: &tauri::AppHandle) {
    use tauri::Manager;
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.unminimize();
        let _ = win.show();
        let _ = win.set_focus();
    }
}

/// Leave voice and stream audio and stop clip work (no ffmpeg keeps running, or holding its files, after Pulse): on
/// quit, and before an update's installer takes over (#82: its exit skips `RunEvent::Exit`, and the installer has
/// to replace the bundled ffmpeg).
pub(crate) async fn wind_down(app: &tauri::AppHandle) {
    use tauri::Manager;
    let voice = app.state::<voice::VoiceManager>();
    let stream_audio = app.state::<stream::audio::StreamAudio>();
    let core = app.state::<commands::Core>();
    // side by side: one slow close must not use up the other's budget
    let leave = async {
        tokio::join!(stream_audio.leave(), voice.leave());
    };
    let budget = std::time::Duration::from_secs(2);
    let (left, stopped) = tokio::join!(
        tokio::time::timeout(budget, leave),
        tokio::time::timeout(budget, core.stop_clips()),
    );
    if left.is_err() {
        tracing::warn!("voice: leave on exit timed out");
    }
    if stopped.is_err() {
        tracing::warn!("clips: stopping on exit timed out");
    }
}

pub fn run() {
    startup::begin();
    let mut builder = tauri::Builder::default();
    // Must be the first plugin: a second launch exits before anything else starts.
    if session::single_instance(std::env::var("PULSE_PROFILE").ok().as_deref()) {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            bring_to_front(app)
        }));
    }
    // App updates (updater.rs, #82), Windows only: driven from Rust, so no webview permission is granted.
    #[cfg(windows)]
    {
        builder = builder.plugin(tauri_plugin_updater::Builder::new().build());
    }
    builder
        .plugin(tauri_plugin_dialog::init())
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
            // Downloaded models (#77): local (not roaming) data, one copy for every profile.
            models::install_low_delay(app.path().app_local_data_dir()?.join("models"));
            let webview_dir = session::profile_dir(&app.path().app_local_data_dir()?, profile.as_deref());
            // The link-preview pop-out shares the main webview's data directory.
            let webview_data = webview_dir.join("webview");
            create_main_window(app, &webview_data, window_bg::load(&dir))?;
            app.manage(window_bg::Dir(dir.clone()));
            app.manage(embed_window::WebviewData(webview_data));
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
            let fh = app.handle().clone();
            let vh = app.handle().clone();
            app.manage(
                commands::Core::new(
                    session::Store::new(dir.clone(), profile.as_deref()),
                    outbox,
                    media_root,
                    Box::new(move || media::clear_webview_cache(&h)),
                )
                .with_clips(
                    webview_dir.join("clips"),
                    Box::new(move |p| {
                        let _ = fh.asset_protocol_scope().forbid_file(p);
                    }),
                )
                .with_videos(video::Videos::new(Box::new(move |p| {
                    vh.asset_protocol_scope().is_allowed(p)
                })))
                .with_attachments(
                    session::profile_dir(&app.path().app_cache_dir()?, profile.as_deref())
                        .join("attachments"),
                ),
            );
            // A clicked notification brings the window up, then the page opens the conversation.
            let h = app.handle().clone();
            app.manage(notify::Notifier::new(
                notify::app_id(&app.config().identifier, cfg!(debug_assertions)),
                std::sync::Arc::new(move |click: notify::Click| {
                    use tauri::Emitter;
                    bring_to_front(&h);
                    let _ = h.emit(notify::CLICK_EVENT, click);
                }),
            ));
            app.manage(gifs::Gifs::default());
            app.manage(voice::mictest::MicTest::default());
            app.manage(soundboard::Cache::default());
            app.manage(soundboard::Prepared::default());
            app.manage(soundboard::Clicks::default());
            app.manage(soundboard::Trimming::default());
            app.manage(soundboard::Players::default());
            app.manage(soundboard::Echoes::default());
            app.manage(stream::PickerMemory::default());
            app.manage(stream::ghost_out::GhostOutput::default());
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
            // App updates (#82): the launch check starts now, so the page usually has the answer when it asks. On
            // Windows the installer ends the process: first clear the crash marker (an update isn't a crash) and
            // flush the log. If the installer doesn't start after all, Pulse carries on: both come back (#233).
            let (h, h2) = (app.handle().clone(), app.handle().clone());
            app.manage(updater::Updates::new(std::sync::Arc::new(updater::Handover::new(
                move || {
                    tracing::info!("update: handing over to the installer");
                    reports::crash::clear_running(&h.state::<reports::LogDir>().0);
                    if let Some(guard) = h.try_state::<LogGuard>() {
                        drop(guard.0.lock().unwrap().take());
                    }
                },
                move || {
                    let log_dir = &h2.state::<reports::LogDir>().0;
                    reports::crash::mark_running(log_dir);
                    if let Some(guard) = h2.try_state::<LogGuard>() {
                        match logging::reopen(log_dir) {
                            Ok(g) => *guard.0.lock().unwrap() = Some(g),
                            Err(e) => eprintln!("logging unavailable after a failed update: {e}"),
                        }
                    }
                    tracing::warn!("update: the installer didn't start; carrying on with this version");
                },
            ))));
            updater::start_launch_check(app.handle(), session::Store::new(dir.clone(), profile.as_deref()));
            updater::spawn_periodic(app.handle());
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
            commands::create_category,
            commands::rename_category,
            commands::delete_category,
            commands::save_layout,
            commands::join_server,
            commands::list_channels,
            commands::list_members,
            commands::list_messages,
            commands::send_message,
            gifs::gif_feed,
            gifs::gif_favourites,
            gifs::set_gif_favourite,
            commands::edit_message,
            commands::delete_message,
            commands::hide_embed,
            commands::pin_message,
            commands::unpin_message,
            commands::list_pins,
            commands::share_to_stage,
            commands::stop_stage_share,
            commands::embed_origin,
            window_bg::set_window_background,
            embed_window::open_embed_window,
            commands::join_voice,
            commands::set_voice_bitrate,
            commands::set_channel_bitrate,
            commands::leave_voice,
            commands::prefetch_voice,
            commands::toggle_mute,
            commands::toggle_deafen,
            commands::set_server_voice,
            commands::server_voice,
            commands::disconnect_voice,
            commands::set_peer_volume,
            commands::set_call_mix,
            commands::set_audio_config,
            models::model_status,
            models::model_download,
            commands::list_audio_devices,
            commands::start_mic_test,
            commands::stop_mic_test,
            commands::watch_voice_meters,
            commands::watch_voice_link,
            commands::play_sound,
            commands::start_ringtone,
            commands::stop_ringtone,
            commands::start_call,
            commands::decline_call,
            commands::create_dm,
            commands::add_members,
            commands::remove_member,
            commands::rename_channel,
            commands::set_group_icon,
            commands::delete_channel,
            commands::list_deleted_channels,
            commands::restore_channel,
            commands::purge_channel,
            commands::delete_server,
            commands::list_deleted_servers,
            commands::restore_server,
            commands::mark_read,
            commands::mark_channel_read,
            commands::set_mute,
            commands::clear_mute,
            commands::set_notify_level,
            commands::list_server_wheels,
            commands::list_my_wheels,
            commands::create_wheel,
            commands::update_wheel,
            commands::delete_wheel,
            commands::spin_wheel,
            commands::spin_again,
            commands::spin_wheel_quietly,
            commands::create_poll,
            commands::get_poll,
            commands::vote_poll,
            commands::close_poll,
            commands::close_conversation,
            commands::upload_emoji_media,
            commands::get_profile,
            commands::update_profile,
            commands::get_badge_catalogue,
            commands::set_badge_showcase,
            commands::mark_badges_seen,
            commands::report_muted_talk,
            commands::report_egg,
            commands::set_nickname,
            commands::set_private_nickname,
            commands::set_status,
            commands::set_appear_offline,
            commands::set_live_notify,
            commands::name_archive,
            commands::upload_avatar_media,
            commands::upload_banner_media,
            commands::create_emoji,
            commands::rename_emoji,
            commands::delete_emoji,
            commands::soundboard_support,
            commands::soundboard_pick_file,
            commands::soundboard_open,
            commands::soundboard_preview,
            commands::soundboard_upload,
            commands::soundboard_play,
            commands::soundboard_warm,
            commands::soundboard_trim_play,
            commands::soundboard_trim_stop,
            commands::create_sound,
            commands::update_sound,
            commands::delete_sound,
            commands::list_sounds,
            commands::play_sound_to_call,
            commands::set_sound_favourite,
            commands::set_join_sound,
            commands::add_reaction,
            commands::remove_reaction,
            commands::list_reactors,
            // quotes
            commands::list_messages_around,
            commands::list_messages_after,
            commands::upload_quote_media,
            commands::read_quote_image,
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
            commands::search_messages,
            commands::start_qoty,
            commands::vote_qoty,
            commands::unvote_qoty,
            commands::add_qoty_option,
            commands::end_qoty,
            commands::reopen_qoty,
            // attachments (#57)
            attachments::commands::stage_attachment_paths,
            attachments::commands::stage_attachment_bytes,
            attachments::commands::unstage_attachment,
            attachments::commands::zip_attachment_folder,
            attachments::commands::pick_attachment_files,
            attachments::commands::send_with_attachments,
            attachments::commands::cancel_attachment_send,
            attachments::commands::attachment_urls,
            attachments::commands::save_attachment,
            // clips
            commands::add_game,
            commands::edit_clip,
            commands::clip_url,
            commands::save_clip,
            commands::save_image,
            commands::set_fullscreen,
            commands::list_clips,
            commands::clip_facets,
            commands::set_clip_view,
            commands::clip_storage,
            commands::clip_support,
            commands::open_clip,
            commands::pick_clip_file,
            commands::prepare_preview,
            commands::clip_filmstrip,
            commands::close_clip_source,
            commands::video_url,
            commands::send_clip,
            commands::retry_clip,
            commands::cancel_clip,
            commands::clip_jobs,
            startup::ui_painted,
            keybinds::get_keybinds,
            keybinds::set_keybinds,
            keybinds::set_keybinds_enabled,
            keybinds::pause_keybinds,
            reports::submit_report,
            reports::submit_idea,
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
            notify::notify_message,
            notify::withdraw_notification,
            stream::stream_picker_window,
            stream::stream_audio_start,
            stream::stream_audio_probe,
            stream::stream_audio_sound,
            stream::stream_audio_status,
            stream::stream_audio_stop,
            stream::stream_audio_leave,
            stream::stream_audio_mute,
            stream::stream_audio_watch,
            stream::set_stream_volume,
            updater::update_launch,
            updater::update_check,
            updater::update_install,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Pulse")
        .run(|app, event| {
            // Quitting leaves voice properly, so others don't see a ghost until LiveKit times it
            // out. (Minimising/hiding isn't an exit: you stay in voice.)
            // It also stops clip work, so no ffmpeg keeps running (and holding its files) after Pulse.
            if let tauri::RunEvent::Exit = event {
                use tauri::Manager;
                tauri::async_runtime::block_on(wind_down(app));
                reports::crash::clear_running(&app.state::<reports::LogDir>().0);
            }
        });
}
