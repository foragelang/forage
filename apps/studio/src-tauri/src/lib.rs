//! Forage Studio — Tauri backend.

mod browser_driver;
mod commands;
mod menu;
mod state;
mod workspace;

use state::StudioState;
use tauri::Manager;
use tracing_subscriber::EnvFilter;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Route `tracing::` events from forage-{core,http,browser,…} to stderr
    // so `cargo tauri dev` shows them. tracing-subscriber's tracing-log
    // bridge also captures `log::` records from reqwest / cookie_store /
    // tao, so this single subscriber covers everything — no
    // tauri-plugin-log needed (and indeed adding both panics with
    // "logger already set").
    //
    // RUST_LOG *adds to* our defaults rather than replacing them.
    // `EnvFilter::try_from_default_env()` would honor RUST_LOG verbatim,
    // and `RUST_LOG=forage_http=trace` is exclusive — it disables every
    // other target including our own logs. Prepending the defaults and
    // letting RUST_LOG come last means env-supplied directives win for
    // their target while everything else stays visible.
    let defaults =
        "info,forage_http=debug,forage_core=debug,forage_studio=debug,forage_browser=debug";
    let directives = match std::env::var("RUST_LOG") {
        Ok(env) if !env.is_empty() => format!("{defaults},{env}"),
        _ => defaults.to_string(),
    };
    let filter = EnvFilter::new(directives);
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .with_writer(std::io::stderr)
        .try_init();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // Note: no tauri-plugin-log — tracing-subscriber above already
            // captures both `tracing::` events and (via its tracing-log
            // bridge) `log::` records from reqwest / cookie_store / tao.
            // Adding tauri-plugin-log on top panics with "logger already set."
            //
            // Drop an empty `forage.toml` at the workspace root on first
            // launch so an existing user library quietly becomes a
            // workspace. The check is idempotent — no overwrite if the
            // manifest is already there. Failure surfaces as a real
            // setup error: Studio can't operate without a writable
            // workspace, so silent fallback isn't useful.
            let ws_root = workspace::workspace_root();
            workspace::ensure_workspace_manifest(&ws_root).map_err(|e| {
                format!(
                    "failed to initialize workspace manifest at {}: {e}",
                    ws_root.display()
                )
            })?;
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
            commands::validate_recipe,
            commands::create_recipe,
            commands::delete_recipe,
            commands::show_recipe_context_menu,
            commands::run_recipe,
            commands::cancel_run,
            commands::debug_resume,
            commands::set_breakpoints,
            commands::set_recipe_breakpoints,
            commands::load_recipe_breakpoints,
            commands::set_pause_iterations,
            commands::recipe_outline,
            commands::recipe_hover,
            commands::language_dictionary,
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
