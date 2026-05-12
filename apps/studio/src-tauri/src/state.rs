//! Mutable runtime state held by the Tauri app.

use std::collections::HashSet;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use forage_http::ResumeAction;
use forage_hub::AuthStore;
use tauri::Wry;
use tokio::sync::{Notify, oneshot};

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
    /// Step names with breakpoints set. Persists across runs — the frontend
    /// pushes the current set via `set_breakpoints` whenever the user
    /// toggles a gutter marker.
    pub breakpoints: Mutex<HashSet<String>>,
    /// The in-flight debug session, if any. Every run installs one; the
    /// `debug_resume` command pulls the pending oneshot out of here to
    /// wake the paused engine task.
    pub debug_session: Mutex<Option<Arc<DebugSession>>>,
}

/// Per-run debug coordination. Holds the pending oneshot the engine task is
/// awaiting on. `before_step` puts a fresh sender into `pending` and parks
/// on the receiver; `debug_resume` takes the sender out and fires it.
///
/// `step_over_pending` is set when the user clicks Step Over from a paused
/// state — the *next* step pause must wait regardless of whether it's on a
/// breakpoint. We swap-clear it inside `before_step` so it's a one-shot.
#[derive(Default)]
pub struct DebugSession {
    pub pending: Mutex<Option<oneshot::Sender<ResumeAction>>>,
    pub step_over_pending: AtomicBool,
}
