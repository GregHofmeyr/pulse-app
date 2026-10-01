pub mod api;
pub mod backoff;
pub mod commands;
pub mod gateway;
pub mod session;

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            use tauri::Manager;
            let dir = app.path().app_data_dir()?;
            app.manage(commands::Core::new(session::Store::new(dir)));
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running Pulse");
}
