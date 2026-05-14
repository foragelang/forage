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
//!   would otherwise need `RwLock`: the muda `Menu` handle (only
//!   accessed from the Tauri main thread, but the type isn't easy to
//!   wrap in `ArcSwap`).
//!
//! Studio owns the user-facing workspace: scans for recipes, tracks
//! declarations, resolves catalogs against on-disk drafts. The daemon
//! owns deployed versions and run history; the two domains touch
//! through the `deploy_recipe` Tauri command, which validates a draft
//! against the Studio-side catalog and hands the frozen pair to the
//! daemon.

use std::collections::HashSet;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, RwLock};

use arc_swap::{ArcSwap, ArcSwapOption};
use forage_core::workspace::Workspace;
use forage_daemon::Daemon;
use forage_http::ResumeAction;
use tauri::Wry;
use tokio::sync::{Notify, oneshot};

pub struct StudioState {
    /// In-process daemon for scheduling, run history, and output stores.
    /// Owned for the life of the Studio process; closed during teardown.
    pub daemon: Arc<Daemon>,
    /// Cached on-disk workspace view: recipes, declarations files,
    /// manifest. Loaded at boot via `forage_core::workspace::load` and
    /// refreshed by the `refresh_workspace` command on filesystem
    /// changes. Held briefly under `read()` for catalog resolution and
    /// file-tree listing; `write()` only fires during a refresh.
    pub workspace: RwLock<Workspace>,
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

impl StudioState {
    /// Build a `StudioState` around an already-opened daemon and a
    /// pre-loaded workspace. `lib.rs::run` does the construction at
    /// app boot so the daemon's scheduler is alive before any command
    /// fires; tests construct one through here too.
    pub fn new(daemon: Arc<Daemon>, workspace: Workspace) -> Self {
        Self {
            daemon,
            workspace: RwLock::new(workspace),
            run_cancel: ArcSwapOption::empty(),
            last_context_menu: Mutex::new(None),
            breakpoints: ArcSwap::new(Arc::new(HashSet::new())),
            debug_session: ArcSwapOption::empty(),
        }
    }
}

/// Per-run debug coordination. Holds the pending oneshot the engine task
/// is awaiting on. `before_step` (or `before_iteration`) puts a fresh
/// sender into `pending` and parks on the receiver; `debug_resume`
/// takes the sender out and fires it.
///
/// `step_over_pending` is set when the user clicks Step Over from a
/// paused state — the *next* pause must wait regardless of whether it's
/// on a breakpoint. We swap-clear it inside the pause hook so it's a
/// one-shot.
///
/// `pause_iterations` is the user's "pause inside for-loops" toggle.
/// When true, `before_iteration` waits for the user; when false, it
/// short-circuits to Continue. Carried inside the session (not on
/// StudioState) so it resets to the default when a fresh run starts.
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
    pub pause_iterations: AtomicBool,
}
