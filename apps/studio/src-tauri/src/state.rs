//! Mutable runtime state held by the Tauri app.
//!
//! Concurrency rules of thumb:
//!
//! - **Read-mostly state** uses `ArcSwap` / `ArcSwapOption`: the engine's
//!   step-pause hook reads `breakpoints` and `debug_session` once per
//!   step, while writes (user toggles, run start/end) are rare. Lock-free
//!   reads keep the pause path tight.
//! - **Per-pause coordination** (the resume oneshot, the step-over flag)
//!   lives inside `DebugSession`, which is itself swapped in via
//!   `ArcSwapOption`. The session is recreated on every run, so any
//!   stale state from a previous run is dropped when the new session
//!   replaces it.
//! - **Mutexes** remain only where genuine mutation through `&self`
//!   would otherwise need `RwLock`: the dirty-buffer cache, and the
//!   muda `Menu` handle (only accessed from the Tauri main thread, but
//!   the type isn't easy to wrap in `ArcSwap`).

use std::collections::HashSet;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;

use arc_swap::{ArcSwap, ArcSwapOption};
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
    /// Cancellation signal for the in-flight `run_recipe` call. The
    /// frontend `cancel_run` command notifies through this; `run_recipe`
    /// selects against it so the engine future drops mid-fetch. Lifecycle:
    /// installed by `run_recipe` before kicking the engine, cleared (set
    /// to `None`) when the engine future resolves or is cancelled.
    pub run_cancel: ArcSwapOption<Notify>,
    /// The most recently shown native context menu. Held here so the muda
    /// NSMenu/HMENU stays alive while the user is interacting with it —
    /// without this, `popup_menu_at` returns and the Menu Rust handle is
    /// dropped before the click event fires, losing the event. Only
    /// touched on the Tauri main thread.
    pub last_context_menu: Mutex<Option<tauri::menu::Menu<Wry>>>,
    /// Step names with breakpoints set. Persists across runs and is read
    /// on every engine-step pause; the frontend pushes a fresh set via
    /// `set_breakpoints` whenever the user toggles a gutter marker.
    pub breakpoints: ArcSwap<HashSet<String>>,
    /// The in-flight debug session, if any. Every run installs one; the
    /// `debug_resume` command and the engine-side `StudioDebugger` pull
    /// it out of here. `None` means no run is in flight.
    pub debug_session: ArcSwapOption<DebugSession>,
}

/// Per-run debug coordination. Holds the pending oneshot the engine task
/// is awaiting on. `before_step` puts a fresh sender into `pending` and
/// parks on the receiver; `debug_resume` takes the sender out and fires
/// it.
///
/// `step_over_pending` is set when the user clicks Step Over from a
/// paused state — the *next* step pause must wait regardless of whether
/// it's on a breakpoint. We swap-clear it inside `before_step` so it's a
/// one-shot.
///
/// The oneshot sender lives in `Mutex<Option<…>>` rather than another
/// `ArcSwapOption` because we need atomic take-and-fire: removing the
/// sender from the cell *and* sending the value have to happen together
/// so two `debug_resume` callers can't race on a single pause. A short
/// Mutex critical section gives that without extra machinery.
#[derive(Default)]
pub struct DebugSession {
    pub pending: Mutex<Option<oneshot::Sender<ResumeAction>>>,
    pub step_over_pending: AtomicBool,
}
