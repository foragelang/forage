//! Forage Studio — Tauri backend.

mod browser_driver;
mod commands;
mod library;
mod state;

use state::StudioState;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle()
                    .plugin(tauri_plugin_log::Builder::default().build())?;
            }
            app.manage(StudioState::default());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_recipes,
            commands::load_recipe,
            commands::save_recipe,
            commands::create_recipe,
            commands::run_recipe,
            commands::publish_recipe,
            commands::auth_whoami,
            commands::auth_start_device_flow,
            commands::auth_poll_device,
            commands::auth_logout,
            commands::studio_version,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Forage Studio");
}
