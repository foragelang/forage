//! Mutable runtime state held by the Tauri app.

use std::sync::Mutex;

use forage_hub::AuthStore;

#[derive(Default)]
#[allow(dead_code)] // wired in when autosave + cached auth lookups land.
pub struct StudioState {
    /// Open-recipe scratch state — slug → unsaved buffer.
    pub dirty_buffers: Mutex<std::collections::HashMap<String, String>>,
    /// Auth store wrapper.
    pub auth_store: AuthStore,
}
