//! Mutable runtime state held by the Tauri app.

use std::sync::{Arc, Mutex};

use forage_hub::AuthStore;
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
}
