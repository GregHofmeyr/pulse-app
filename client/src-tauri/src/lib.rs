pub mod api;
pub mod commands;
pub mod session;

pub fn run() {
    tauri::Builder::default()
        .manage(commands::Core::default())
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
