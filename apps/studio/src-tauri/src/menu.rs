//! Native menus. Tauri 2's menu builder API.

use tauri::menu::{
    Menu, MenuBuilder, MenuItem, MenuItemBuilder, PredefinedMenuItem, SubmenuBuilder,
};
use tauri::{AppHandle, Emitter, Wry};

/// Build the application menu. Returns both the menu (for installation
/// onto the app) and the `Close Workspace` item so `lib.rs` can stash a
/// handle and toggle its enabled state as the user opens/closes
/// workspaces — disabled while no workspace is open.
pub fn build_menu(app: &AppHandle) -> tauri::Result<(Menu<Wry>, MenuItem<Wry>)> {
    let open_workspace = MenuItemBuilder::with_id("open_workspace", "Open Workspace\u{2026}")
        .accelerator("CmdOrCtrl+O")
        .build(app)?;
    let close_workspace = MenuItemBuilder::with_id("close_workspace", "Close Workspace")
        .accelerator("CmdOrCtrl+W")
        .enabled(false)
        .build(app)?;
    let new_recipe = MenuItemBuilder::with_id("new_recipe", "New Recipe")
        .accelerator("CmdOrCtrl+N")
        .build(app)?;
    let save = MenuItemBuilder::with_id("save", "Save")
        .accelerator("CmdOrCtrl+S")
        .build(app)?;
    let run_live = MenuItemBuilder::with_id("run_live", "Run Live")
        .accelerator("CmdOrCtrl+R")
        .build(app)?;
    let run_replay = MenuItemBuilder::with_id("run_replay", "Run Replay")
        .accelerator("CmdOrCtrl+Shift+R")
        .build(app)?;
    let validate = MenuItemBuilder::with_id("validate", "Validate")
        .accelerator("CmdOrCtrl+Shift+V")
        .build(app)?;

    // Workspace lifecycle anchors the File menu. ⌘W is reassigned from
    // PredefinedMenuItem::close_window to Close Workspace per the
    // design — window close stays available via the OS traffic light.
    let file = SubmenuBuilder::new(app, "File")
        .item(&open_workspace)
        .item(&close_workspace)
        .separator()
        .item(&new_recipe)
        .separator()
        .item(&save)
        .separator()
        .item(&PredefinedMenuItem::quit(app, None)?)
        .build()?;

    let recipe = SubmenuBuilder::new(app, "Recipe")
        .item(&run_live)
        .item(&run_replay)
        .separator()
        .item(&validate)
        .build()?;

    let edit = SubmenuBuilder::new(app, "Edit")
        .item(&PredefinedMenuItem::undo(app, None)?)
        .item(&PredefinedMenuItem::redo(app, None)?)
        .separator()
        .item(&PredefinedMenuItem::cut(app, None)?)
        .item(&PredefinedMenuItem::copy(app, None)?)
        .item(&PredefinedMenuItem::paste(app, None)?)
        .item(&PredefinedMenuItem::select_all(app, None)?)
        .build()?;

    let view = SubmenuBuilder::new(app, "View")
        .item(&PredefinedMenuItem::fullscreen(app, None)?)
        .build()?;

    let menu = MenuBuilder::new(app)
        .item(&file)
        .item(&edit)
        .item(&recipe)
        .item(&view)
        .build()?;

    Ok((menu, close_workspace))
}

/// Wire menu events into Tauri events the frontend listens for.
pub fn on_menu_event(app: &AppHandle, event: tauri::menu::MenuEvent) {
    let id = event.id().as_ref();
    tracing::info!(id = %id, "on_menu_event");
    match id {
        "open_workspace" => {
            let _ = app.emit("menu:open_workspace", ());
        }
        "close_workspace" => {
            let _ = app.emit("menu:close_workspace", ());
        }
        "new_recipe" => {
            let _ = app.emit("menu:new_recipe", ());
        }
        "save" => {
            let _ = app.emit("menu:save", ());
        }
        "run_live" => {
            let _ = app.emit("menu:run_live", ());
        }
        "run_replay" => {
            let _ = app.emit("menu:run_replay", ());
        }
        "validate" => {
            let _ = app.emit("menu:validate", ());
        }
        // Context-menu items use ID prefix `recipe_delete:<name>` so
        // we can route many recipes through one handler.
        other if other.starts_with("recipe_delete:") => {
            let name = &other["recipe_delete:".len()..];
            tracing::info!(name, "menu:recipe_delete dispatching");
            match app.emit("menu:recipe_delete", name) {
                Ok(()) => tracing::info!(name, "menu:recipe_delete emitted"),
                Err(e) => tracing::warn!(name, error = %e, "menu:recipe_delete emit failed"),
            }
        }
        _ => {}
    }
}
