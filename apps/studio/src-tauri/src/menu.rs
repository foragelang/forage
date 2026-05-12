//! Native menus. Tauri 2's menu builder API.

use tauri::menu::{Menu, MenuBuilder, MenuItemBuilder, PredefinedMenuItem, SubmenuBuilder};
use tauri::{AppHandle, Emitter, Wry};

pub fn build_menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
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
    let publish = MenuItemBuilder::with_id("publish", "Publish to Hub…")
        .accelerator("CmdOrCtrl+Shift+P")
        .build(app)?;

    let file = SubmenuBuilder::new(app, "File")
        .item(&new_recipe)
        .separator()
        .item(&save)
        .separator()
        .item(&PredefinedMenuItem::close_window(
            app,
            Some("Close Window"),
        )?)
        .item(&PredefinedMenuItem::quit(app, None)?)
        .build()?;

    let recipe = SubmenuBuilder::new(app, "Recipe")
        .item(&run_live)
        .item(&run_replay)
        .separator()
        .item(&validate)
        .item(&publish)
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

    Ok(menu)
}

/// Wire menu events into Tauri events the frontend listens for.
pub fn on_menu_event(app: &AppHandle, event: tauri::menu::MenuEvent) {
    let id = event.id().as_ref();
    match id {
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
        "publish" => {
            let _ = app.emit("menu:publish", ());
        }
        // Context-menu items use ID prefix `recipe_delete:<slug>` so we
        // can route many recipe slugs through one handler.
        other if other.starts_with("recipe_delete:") => {
            let slug = &other["recipe_delete:".len()..];
            let _ = app.emit("menu:recipe_delete", slug);
        }
        _ => {}
    }
}
