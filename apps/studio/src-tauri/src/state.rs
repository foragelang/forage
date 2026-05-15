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
use std::sync::atomic::AtomicU8;
use std::sync::{Arc, Mutex};

use arc_swap::{ArcSwap, ArcSwapOption};
use forage_core::Scope;
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
    /// 0-based source lines with breakpoints set. Persists across runs
    /// and is read on every engine pause; the frontend pushes a fresh
    /// set via `set_breakpoints` whenever the user toggles a gutter
    /// marker. Keyed on the start line of the statement (step / emit /
    /// for) the engine is about to enter so the gutter click maps
    /// directly to the engine's pause check — no name lookup, no
    /// re-parse on the hot path.
    pub breakpoints: ArcSwap<HashSet<u32>>,
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

/// What kind of step the user clicked, encoded as a small enum so the
/// pause hook can branch on it without round-tripping a string. Kept
/// in `AtomicU8` rather than `Atomic<StepKind>` (which doesn't exist
/// stable) so swap-and-clear is cheap. `None` is the "no pending
/// step-action; pause only on breakpoint" baseline.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepKind {
    None = 0,
    Over = 1,
    In = 2,
}

impl StepKind {
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => StepKind::Over,
            2 => StepKind::In,
            _ => StepKind::None,
        }
    }
}

/// Per-run debug coordination. Holds the pending oneshot the engine task
/// is awaiting on. The pause hooks (`before_step` / `before_emit` /
/// `before_for_loop`) put a fresh sender into `pending` and park on the
/// receiver; `debug_resume` takes the sender out and fires it.
///
/// `step_kind` is the one-shot the user clicks Step Over / Step In from a
/// paused state — the *next* pause-able statement waits regardless of
/// whether it sits on a breakpoint. We swap-clear it inside the pause
/// hook so it's a single shot per click.
///
/// `paused_scope` holds a clone of the engine's live `Scope` at the
/// moment the pause fired. The watch-expression evaluator and the REPL
/// command pull from this — they can't reach back into the engine task
/// (it's parked on the oneshot), but they need a real `Scope` to
/// evaluate against. Cleared on resume so a stale scope can't survive
/// a continue.
///
/// The oneshot sender lives in `Mutex<Option<…>>` rather than another
/// `ArcSwapOption` because we need atomic take-and-fire: removing the
/// sender from the cell *and* sending the value have to happen together
/// so two `debug_resume` callers can't race on a single pause. A short
/// Mutex critical section gives that without extra machinery.
#[derive(Default)]
pub struct DebugSession {
    pub pending: Mutex<Option<oneshot::Sender<ResumeAction>>>,
    pub step_kind: AtomicU8,
    pub paused_scope: Mutex<Option<Scope>>,
    /// Run id minted at `run_recipe` start, used by the studio-side
    /// `RUN_STEP_RESPONSE_EVENT` / `RUN_BEGIN_EVENT` /
    /// `RUN_DEBUG_RESUMED_EVENT` events so the frontend can correlate
    /// pause events with their originating run. Set once at session
    /// creation; cleared with the session at run end.
    pub run_id: String,
}
