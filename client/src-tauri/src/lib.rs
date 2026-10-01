pub mod api;
pub mod commands;
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running Pulse");
}
