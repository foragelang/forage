//! Forage Studio — Tauri backend.

mod browser_driver;
mod commands;
mod library;
mod menu;
mod state;

use state::StudioState;
use tauri::Manager;
use tracing_subscriber::EnvFilter;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Route `tracing::` events from forage-{core,http,browser,…} to stderr
    // so `cargo tauri dev` shows them. tauri-plugin-log handles `log::`
    // records (reqwest, cookie_store, tao); without this subscriber our
    // engine's `debug!`/`trace!` calls would never surface.
    //
    // RUST_LOG overrides; default shows DEBUG for our crates and the engine
    // module path, INFO for everything else.
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new(
            "info,forage_http=debug,forage_core=debug,forage_studio=debug,forage_browser=debug",
        )
    });
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .with_writer(std::io::stderr)
        .try_init();

    tauri::Builder::default()
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle()
                    .plugin(tauri_plugin_log::Builder::default().build())?;
            }
            app.manage(StudioState::default());
            let m = menu::build_menu(app.handle())?;
            app.set_menu(m)?;
            Ok(())
        })
        .on_menu_event(|app, event| {
            menu::on_menu_event(app, event);
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_recipes,
            commands::load_recipe,
            commands::save_recipe,
            commands::create_recipe,
            commands::run_recipe,
            commands::cancel_run,
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
