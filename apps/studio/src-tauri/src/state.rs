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
//!   would otherwise need `RwLock`: the muda `Menu` / `MenuItem` handles
//!   (only accessed from the Tauri main thread, but the types aren't
//!   easy to wrap in `ArcSwap`).
//!
//! Studio owns the user-facing workspace: scans for recipes, tracks
//! declarations, resolves catalogs against on-disk drafts. The daemon
//! owns deployed versions and run history; the two domains touch
//! through the `deploy_recipe` Tauri command, which validates a draft
//! against the Studio-side catalog and hands the frozen pair to the
//! daemon.
//!
//! The active workspace is a paired (daemon, workspace) value
//! installed under one `ArcSwapOption` so readers see them swap as a
//! unit. Studio boots with that slot empty; the user picks Open or
//! New to install a session. The `workspace_switch` mutex serializes
//! open/close transitions so two concurrent commands can't interleave
//! and leak a daemon scheduler.

use std::collections::HashSet;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use arc_swap::{ArcSwap, ArcSwapOption};
use forage_core::workspace::Workspace;
use forage_daemon::Daemon;
use forage_http::ResumeAction;
use tauri::Wry;
use tauri::menu::MenuItem;
use tokio::sync::{Notify, oneshot};

/// The paired daemon + workspace that together represent an open
/// workspace. Stored as one `ArcSwapOption<WorkspaceSession>` so
/// readers see them install or clear together — never one without
/// the other.
pub struct WorkspaceSession {
    /// In-process daemon for scheduling, run history, and output stores.
    pub daemon: Arc<Daemon>,
    /// Cached on-disk workspace view: recipes, declarations files,
    /// manifest. Loaded via `forage_core::workspace::load` at open
    /// time and replaced by `refresh_workspace` on filesystem
    /// changes (which swaps in a new session with the same daemon).
    pub workspace: Arc<Workspace>,
}

pub struct StudioState {
    /// The active workspace, daemon-and-all. Empty until the user opens
    /// a workspace; replaced atomically on open / close / switch /
    /// refresh so readers via `require_session` see either the prior
    /// pair or the next pair, never a half-installed mix. Reads are
    /// lock-free `ArcSwap` loads; writes serialize through
    /// `workspace_switch`.
    pub session: ArcSwapOption<WorkspaceSession>,
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
    /// The native File menu's `Close Workspace` item handle. Stashed so
    /// `open_workspace` / `close_workspace` can toggle its enabled
    /// state — disabled when no workspace is open, enabled otherwise.
    /// Populated once by `lib.rs::run` during setup.
    pub menu_close_workspace: Mutex<Option<MenuItem<Wry>>>,
    /// Serializes open/close transitions. The open path on a live
    /// workspace is `close → open`; without this mutex two concurrent
    /// `open_workspace` calls (e.g. ⌘O firing while the dialog from a
    /// prior ⌘O is still resolving) could interleave their close+open
    /// sequences and leak a daemon scheduler.
    pub workspace_switch: tokio::sync::Mutex<()>,
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
    /// Construct an empty state — no workspace, no daemon. The boot
    /// path no longer requires a daemon up front; `open_workspace`
    /// installs one when the user picks a folder.
    pub fn new_empty() -> Self {
        Self {
            session: ArcSwapOption::empty(),
            run_cancel: ArcSwapOption::empty(),
            last_context_menu: Mutex::new(None),
            menu_close_workspace: Mutex::new(None),
            workspace_switch: tokio::sync::Mutex::new(()),
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
