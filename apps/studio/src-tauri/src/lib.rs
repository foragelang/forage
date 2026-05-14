//! Forage Studio — Tauri backend.

mod browser_driver;
mod commands;
mod daemon_browser;
mod menu;
mod state;
mod workspace;

use std::sync::Arc;

use daemon_browser::StudioLiveBrowserDriver;
use state::StudioState;
use tauri::{Emitter, Manager};
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
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            // Note: no tauri-plugin-log — tracing-subscriber above already
            // captures both `tracing::` events and (via its tracing-log
            // bridge) `log::` records from reqwest / cookie_store / tao.
            // Adding tauri-plugin-log on top panics with "logger already set."
            //
            // Drop an empty `forage.toml` at the workspace root on first
            // launch so an existing user workspace quietly becomes
            // discoverable. The check is idempotent — no overwrite if
            // the manifest is already there. Failure surfaces as a real
            // setup error: Studio can't operate without a writable
            // workspace, so silent fallback isn't useful.
            let ws_root = workspace::workspace_root();
            workspace::ensure_workspace_manifest(&ws_root).map_err(|e| {
                format!(
                    "failed to initialize workspace manifest at {}: {e}",
                    ws_root.display()
                )
            })?;

            // Open the daemon; it loads the workspace once at
            // construction and serves it through `Daemon::workspace()`
            // for both its own scheduler and Studio's command paths.
            let daemon = forage_daemon::Daemon::open(ws_root.clone()).map_err(|e| {
                format!("failed to open daemon at {}: {e}", ws_root.display())
            })?;

            // Plug Studio's live browser driver into the daemon so
            // scheduled `engine browser` recipes can run. Without
            // this, the daemon fails those runs with
            // `NoBrowserDriver` at `run_once` time.
            let handle = app.handle().clone();
            daemon.set_browser_driver(Arc::new(StudioLiveBrowserDriver::new(handle.clone())));

            // Forward daemon run completions to the frontend as a
            // single Tauri event. The store-keeping (refresh runs
            // sidebar, refetch scheduled-runs) happens entirely in
            // the UI's event listener.
            let cb_handle = handle.clone();
            daemon.on_run_completed(Box::new(move |sr| {
                if let Err(e) = cb_handle.emit("forage:daemon-run-completed", sr) {
                    tracing::warn!(error = %e, "emit daemon-run-completed failed");
                }
            }));

            // Spawn the scheduler last so it doesn't tick before the
            // browser driver / callback are in place. `start_scheduler`
            // calls `tokio::spawn` internally; the Tauri setup closure
            // runs on the main thread outside the runtime, so enter it
            // via `block_on` before the spawn.
            tauri::async_runtime::block_on(async {
                daemon.start_scheduler();
            });

            app.manage(StudioState::new(daemon));
            let m = menu::build_menu(app.handle())?;
            app.set_menu(m)?;
            Ok(())
        })
        .on_menu_event(|app, event| {
            menu::on_menu_event(app, event);
        })
        .invoke_handler(tauri::generate_handler![
            commands::validate_recipe,
            commands::recipe_progress_unit,
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
            // Workspace + files + daemon.
            commands::current_workspace,
            commands::list_workspace_files,
            commands::load_file,
            commands::save_file,
            commands::daemon_status,
            commands::list_runs,
            commands::get_run,
            commands::configure_run,
            commands::remove_run,
            commands::trigger_run,
            commands::list_scheduled_runs,
            commands::load_run_records,
            commands::validate_cron_expr,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Forage Studio");
}
