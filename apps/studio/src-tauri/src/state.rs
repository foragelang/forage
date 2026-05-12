//! Mutable runtime state held by the Tauri app.

use std::sync::{Arc, Mutex};

use forage_hub::AuthStore;
use tauri::Wry;
use tokio::sync::Notify;

#[derive(Default)]
#[allow(dead_code)] // wired in when autosave + cached auth lookups land.
pub struct StudioState {
    /// Open-recipe scratch state — slug → unsaved buffer.
    pub dirty_buffers: Mutex<std::collections::HashMap<String, String>>,
    /// Auth store wrapper.
    pub auth_store: AuthStore,
    /// Cancellation signal for the in-flight `run_recipe` call. The frontend
    /// `cancel_run` command notifies through this; `run_recipe` selects
    /// against it so the engine future drops mid-fetch.
    pub run_cancel: Mutex<Option<Arc<Notify>>>,
    /// The most recently shown native context menu. Held here so the muda
    /// NSMenu/HMENU stays alive while the user is interacting with it —
    /// without this, popup_menu_at returns and the Menu Rust handle is
    /// dropped before the click event fires, losing the event.
    pub last_context_menu: Mutex<Option<tauri::menu::Menu<Wry>>>,
}
