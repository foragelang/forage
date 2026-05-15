//! Forage Studio — Tauri backend.

mod browser_driver;
mod commands;
mod daemon_browser;
pub mod hub_sync;
mod menu;
mod state;
mod workspace;

use state::StudioState;
use tauri::{Emitter, Manager};
use tauri_plugin_deep_link::DeepLinkExt;
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
        .plugin(tauri_plugin_deep_link::init())
        .setup(|app| {
            // Prune stale entries from the recents sidecar at boot —
            // paths that no longer exist (tempdirs from old test runs
            // are a common source). One info log per dropped entry,
            // file rewritten if anything changed. After this, the
            // every-poll read path doesn't have to filter or log.
            workspace::prune_recents();

            // Studio boots without a workspace. The Welcome screen is
            // the entry point; the user picks Open, New, or a recent
            // workspace to install a daemon. The daemon's lifecycle is
            // now tied to a workspace, not to the app — see
            // `commands::open_workspace`.
            let state = StudioState::new_empty();

            let (menu, close_workspace_item) = menu::build_menu(app.handle())?;
            // Stash the Close Workspace MenuItem so the open/close
            // commands can toggle its enabled state. The item is
            // disabled at build time; `open_workspace_inner` flips
            // it on when a workspace lands in state.
            *state
                .menu_close_workspace
                .lock()
                .expect("menu_close_workspace mutex") = Some(close_workspace_item);

            app.manage(state);
            app.set_menu(menu)?;

            // `forage://clone/<author>/<slug>` deeplinks. The hub IDE's
            // "Open in Studio" button fires one of these; we validate
            // the URL against the segment regex and emit a
            // `forage:deeplink-clone` event the frontend listens for.
            // The frontend then calls `sync_from_hub` against the
            // active workspace (it needs to know which workspace the
            // user is in, which only the JS side tracks).
            let app_handle = app.handle().clone();
            app.deep_link().on_open_url(move |event| {
                for url in event.urls() {
                    let url_str = url.to_string();
                    match hub_sync::parse_clone_url(&url_str) {
                        Ok((author, slug, version)) => {
                            tracing::info!(
                                author = %author,
                                slug = %slug,
                                version = ?version,
                                "deeplink clone received"
                            );
                            let payload = serde_json::json!({
                                "author": author,
                                "slug": slug,
                                "version": version,
                            });
                            if let Err(e) =
                                app_handle.emit("forage:deeplink-clone", payload)
                            {
                                tracing::warn!(error = %e, "emit deeplink-clone failed");
                            }
                        }
                        Err(e) => {
                            tracing::warn!(
                                url = %url_str,
                                error = %e,
                                "ignoring invalid forage:// URL"
                            );
                        }
                    }
                }
            });

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
            commands::sync_from_hub,
            commands::fork_from_hub,
            commands::preview_publish,
            commands::auth_whoami,
            commands::auth_start_device_flow,
            commands::auth_poll_device,
            commands::auth_logout,
            commands::studio_version,
            // Workspace lifecycle.
            commands::open_workspace,
            commands::new_workspace,
            commands::close_workspace,
            commands::list_recent_workspaces,
            // Workspace + files + daemon.
            commands::current_workspace,
            commands::list_workspace_files,
            commands::refresh_workspace,
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
            commands::load_run_jsonld,
            commands::validate_cron_expr,
            commands::deploy_recipe,
            commands::list_deployed_versions,
            commands::list_recipe_statuses,
            commands::notebook_run,
            commands::notebook_compose_source,
            commands::notebook_save,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Forage Studio");
}
