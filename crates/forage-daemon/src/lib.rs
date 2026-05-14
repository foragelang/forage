//! Forage scheduling + persistence runtime.
//!
//! The daemon is the system that:
//! - Tracks one `Run` per recipe in a per-workspace
//!   `<workspace_root>/.forage/daemon.sqlite` (the "daemon DB").
//! - Runs an in-process scheduler over those Runs (interval / cron /
//!   manual), firing the engine and writing emitted records to
//!   `Run.output` (the "output store").
//! - Derives per-Run health (Ok / Drift / Fail / Paused) from history
//!   via a count-based drift rule.
//! - Surfaces a callback-driven API so a host (Studio today, a
//!   sidecar tomorrow) can listen for `run-completed` events without
//!   coupling to the runtime.
//!
//! The library API is the source of truth. Phase 3 wires this into
//! Studio's Tauri commands; an out-of-process binary is a future
//! drop-in that uses the same `Daemon` type.

mod db;
mod error;
mod health;
mod model;
mod output;
mod run;
mod scheduler;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use async_trait::async_trait;
use chrono::Utc;
use forage_core::workspace::{Workspace, discover, load};
use forage_core::{EvalValue, Recipe, Snapshot};
use forage_http::{ProgressSink, RunEvent};
use indexmap::IndexMap;
use rusqlite::{Connection, OptionalExtension};
use tokio::sync::Notify;
use tokio::task::JoinHandle;

// Wire types — Studio (Phase 3) carries these through Tauri commands
// and ts-rs generates matching TypeScript from them.
pub use error::{DaemonError, RunError};
pub use model::{
    Cadence, DaemonStatus, Health, Outcome, Run, RunConfig, ScheduledRun, TimeUnit, Trigger,
};

// Drift derivation. The constants and `derive_health` are part of the
// stable health-rule contract — exposed so the test suite can pin
// edge cases like the 70%/71% threshold and downstream consumers can
// re-derive health from a synthesized history.
pub use health::{PRIOR_WINDOW, derive_health};

// Output-store API. `OutputStore` and `derive_schema` are how Studio
// (Phase 3) and any other host inspect / write a Run's emitted rows
// without holding the full daemon state.
pub use output::{ColumnDef, ColumnStorage, OutputStore, TableDef, WriteTx, derive_schema, load_records};

// Scheduler helpers. The pure-computation functions (`next_fire_for`,
// `advance_next_run`, `interval_ms`, `validate_cron`) are part of the
// public surface so callers can reason about when a Run will fire
// without spinning up a scheduler task.
pub use scheduler::{advance_next_run, interval_ms, next_fire_for, validate_cron};

/// Boxed live-engine driver error. The trait is `Send + Sync` so the
/// daemon can plug it into `RunError::Browser` without erasing the
/// underlying type beyond what trait objects already require.
pub type BrowserDriverError = Box<dyn std::error::Error + Send + Sync>;

/// Host-side hook for live browser-engine recipes. The daemon is
/// engine-agnostic: it doesn't ship a `wry` driver of its own.
/// Studio implements this against its Tauri-managed `AppHandle` and
/// plugs the driver in via `Daemon::set_browser_driver`. Daemons
/// without a registered driver simply fail browser-engine runs at
/// `run_once` time — they're still useful for HTTP-engine recipes.
#[async_trait]
pub trait LiveBrowserDriver: Send + Sync {
    async fn run_live(
        &self,
        recipe: &Recipe,
        inputs: IndexMap<String, EvalValue>,
        secrets: IndexMap<String, String>,
        progress: Arc<dyn ProgressSink>,
    ) -> Result<Snapshot, BrowserDriverError>;
}

/// Optional host-side `ProgressSink` for live engines. When set, the
/// daemon's `ProgressForwarder` forwards every engine event to this
/// sink so the host (Studio) can render a live event stream in its
/// editor UI. The daemon doesn't read events from this sink — it
/// derives counts from the engine's resulting `Snapshot`; the sink is
/// purely a passthrough.
type HostProgressSlot = Mutex<Option<Arc<dyn ProgressSink>>>;

/// Host-side callback fired once per completed run. Studio uses this
/// to emit a Tauri event (`forage:daemon-run-completed`) so the UI
/// can refetch.
pub type RunCompletedCb = Box<dyn Fn(&ScheduledRun) + Send + Sync>;

/// Source of "now" timestamps and waits. Production uses
/// `SystemClock` (wall clock + tokio sleep); tests inject a stub that
/// drives the scheduler off `advance()` so ticks aren't tied to
/// wall-clock waits.
#[async_trait]
pub trait Clock: Send + Sync {
    /// Current wall-clock as ms-since-epoch.
    fn now_ms(&self) -> i64;
    /// Block until `now_ms() >= deadline_ms`. Cancellation-safe so the
    /// scheduler can race this against `schedule_changed`/`shutdown`
    /// via `tokio::select!`.
    async fn sleep_until_ms(&self, deadline_ms: i64);
}

pub struct SystemClock;

#[async_trait]
impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        Utc::now().timestamp_millis()
    }
    async fn sleep_until_ms(&self, deadline_ms: i64) {
        let now = self.now_ms();
        let delta = (deadline_ms - now).max(0) as u64;
        tokio::time::sleep(std::time::Duration::from_millis(delta)).await;
    }
}

pub struct Daemon {
    workspace_root: PathBuf,
    daemon_dir: PathBuf,
    /// Daemon DB connection. Sync API protected by a Mutex; every
    /// query is short-lived so contention is irrelevant.
    pub(crate) connection: Mutex<Connection>,
    /// Wake the scheduler when the schedule changes (configure_run /
    /// remove_run / new Run added). The scheduler `select!`s on this.
    pub(crate) schedule_changed: Notify,
    /// Hard stop signal: aborts the scheduler task on the next loop
    /// iteration. `Daemon::close` fires this.
    pub(crate) shutdown: Notify,
    /// Optional host progress sink. Engines emit through both this
    /// and the daemon's internal collector.
    pub(crate) host_progress: HostProgressSlot,
    /// Live browser-engine driver, plugged in by the host.
    pub(crate) browser_driver: Mutex<Option<Arc<dyn LiveBrowserDriver>>>,
    /// Fires after each completed run.
    pub(crate) run_completed_cb: Mutex<Option<RunCompletedCb>>,
    /// Set true by the scheduler task on entry, cleared on exit (via
    /// `close()` → shutdown notify). The flag mirrors task liveness
    /// without needing an `Arc<JoinHandle>`, which isn't a thing.
    pub(crate) scheduler_running: AtomicBool,
    /// When the scheduler started — used by `DaemonStatus`.
    scheduler_started_at: Mutex<Option<i64>>,
    /// Handle for the spawned scheduler task. Held so `close()` can
    /// abort it; `start_scheduler` checks this slot and no-ops if a
    /// live handle is already present.
    scheduler_handle: Mutex<Option<JoinHandle<()>>>,
    /// Time source. Production uses `SystemClock`; tests inject a
    /// stub so they can drive scheduler ticks without wall-clock waits.
    pub(crate) clock: Arc<dyn Clock>,
    /// Workspace loaded at `Daemon::open` and reused across every
    /// `run_once`. Avoids re-walking the directory tree (and re-parsing
    /// every recipe) on every scheduled fire. `refresh_workspace()`
    /// re-reads it for Studio's filesystem-watch path. Held via a sync
    /// `RwLock` and never across `.await`; `run.rs` reads it briefly
    /// to derive the catalog, then drops the guard before the engine
    /// call.
    pub(crate) workspace: RwLock<Workspace>,
}

impl Daemon {
    /// Open (or create) the daemon at `<workspace_root>/.forage/daemon.sqlite`.
    /// Runs schema migrations on connect.
    pub fn open(workspace_root: PathBuf) -> Result<Arc<Self>, DaemonError> {
        Self::open_with_clock(workspace_root, Arc::new(SystemClock))
    }

    /// Same as `open`, with a custom clock. Used by tests to drive
    /// deterministic scheduler ticks.
    pub fn open_with_clock(
        workspace_root: PathBuf,
        clock: Arc<dyn Clock>,
    ) -> Result<Arc<Self>, DaemonError> {
        let daemon_dir = workspace_root.join(".forage");
        let conn = db::open_connection(&daemon_dir)?;
        // Load the workspace once. `discover` ancestor-walks for a
        // `forage.toml` marker; lonely-recipe mode (no marker) is not
        // supported by the daemon — the daemon is per-workspace by
        // construction.
        let ws_root = discover(&workspace_root)
            .map(|w| w.root.clone())
            .ok_or_else(|| DaemonError::NoWorkspace {
                root: workspace_root.clone(),
            })?;
        let workspace = load(&ws_root)?;
        Ok(Arc::new(Self {
            workspace_root,
            daemon_dir,
            connection: Mutex::new(conn),
            schedule_changed: Notify::new(),
            shutdown: Notify::new(),
            host_progress: Mutex::new(None),
            browser_driver: Mutex::new(None),
            run_completed_cb: Mutex::new(None),
            scheduler_running: AtomicBool::new(false),
            scheduler_started_at: Mutex::new(None),
            scheduler_handle: Mutex::new(None),
            clock,
            workspace: RwLock::new(workspace),
        }))
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    /// Read-lock guard on the daemon's loaded `Workspace`. Studio's
    /// Tauri commands read this when serving `current_workspace` and
    /// `list_workspace_files` instead of holding their own copy —
    /// dual ownership was the bug. Held briefly and never across
    /// `.await`; `refresh_workspace()` takes the write side.
    pub fn workspace(&self) -> std::sync::RwLockReadGuard<'_, Workspace> {
        self.workspace.read().expect("workspace lock poisoned")
    }

    /// "Now" through the configured clock. Production = wall clock;
    /// tests = stubbed timeline.
    pub fn now_ms(&self) -> i64 {
        self.clock.now_ms()
    }

    /// Spawn the scheduler task. Idempotent: if a live scheduler task
    /// already exists, returns without spawning a second one (so two
    /// tasks can't race on the same `Notify`). The first caller is
    /// authoritative; subsequent calls are no-ops.
    pub fn start_scheduler(self: &Arc<Self>) {
        let mut slot = self.scheduler_handle.lock().expect("handle slot poisoned");
        if let Some(handle) = slot.as_ref() {
            if !handle.is_finished() {
                tracing::debug!("start_scheduler called while scheduler already running — no-op");
                return;
            }
        }
        self.scheduler_running.store(true, Ordering::SeqCst);
        *self.scheduler_started_at.lock().expect("ts slot poisoned") = Some(self.now_ms());
        *slot = Some(scheduler::start(self.clone()));
    }

    /// Signal the scheduler to exit and abort its task. The scheduler
    /// loop reacts to the `shutdown` notify on its next iteration; for
    /// callers who want a hard stop without waiting, `close` also
    /// aborts the held `JoinHandle`.
    pub fn close(self: Arc<Self>) {
        self.shutdown.notify_waiters();
        self.scheduler_running.store(false, Ordering::SeqCst);
        *self.scheduler_started_at.lock().expect("ts slot poisoned") = None;
        if let Some(handle) = self
            .scheduler_handle
            .lock()
            .expect("handle slot poisoned")
            .take()
        {
            handle.abort();
        }
    }

    /// Reload the workspace from disk. Studio (Phase 3) calls this on
    /// filesystem events so the cached `Workspace` reflects new /
    /// renamed / deleted recipes without restarting the daemon.
    pub fn refresh_workspace(&self) -> Result<(), DaemonError> {
        let root = self
            .workspace
            .read()
            .expect("workspace lock poisoned")
            .root
            .clone();
        let fresh = load(&root)?;
        *self.workspace.write().expect("workspace lock poisoned") = fresh;
        Ok(())
    }

    pub fn status(&self) -> Result<DaemonStatus, DaemonError> {
        let running = self.scheduler_running.load(Ordering::SeqCst);
        let started_at = self
            .scheduler_started_at
            .lock()
            .expect("ts slot poisoned")
            .unwrap_or(0);
        let active_count = self
            .list_runs()?
            .iter()
            .filter(|r| r.enabled && !matches!(r.cadence, Cadence::Manual))
            .count() as u32;
        Ok(DaemonStatus {
            running,
            version: env!("CARGO_PKG_VERSION").to_string(),
            started_at,
            active_count,
        })
    }

    pub fn list_runs(&self) -> Result<Vec<Run>, DaemonError> {
        let conn = self.connection.lock().expect("daemon connection poisoned");
        db::list_runs(&conn)
    }

    pub fn get_run(&self, run_id: &str) -> Result<Option<Run>, DaemonError> {
        let conn = self.connection.lock().expect("daemon connection poisoned");
        db::get_run_by_id(&conn, run_id)
    }

    pub fn get_run_by_slug(&self, slug: &str) -> Result<Option<Run>, DaemonError> {
        let conn = self.connection.lock().expect("daemon connection poisoned");
        db::get_run_by_slug(&conn, slug)
    }

    pub fn list_scheduled_runs(
        &self,
        run_id: &str,
        limit: u32,
        before: Option<i64>,
    ) -> Result<Vec<ScheduledRun>, DaemonError> {
        let conn = self.connection.lock().expect("daemon connection poisoned");
        db::list_scheduled_runs(&conn, run_id, limit, before)
    }

    pub fn load_records(
        &self,
        scheduled_run_id: &str,
        type_name: &str,
        limit: u32,
    ) -> Result<Vec<serde_json::Value>, RunError> {
        // We need to know which output-store this scheduled-run wrote
        // to. The row carries `run_id`; the run row carries `output`.
        let run_id_opt = {
            let conn = self.connection.lock().expect("daemon connection poisoned");
            conn.query_row(
                "SELECT run_id FROM scheduled_runs WHERE id = ?1",
                rusqlite::params![scheduled_run_id],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .map_err(|e| RunError::Daemon(DaemonError::Sqlite(e)))?
        };
        let Some(run_id) = run_id_opt else {
            return Ok(Vec::new());
        };
        let output = {
            let conn = self.connection.lock().expect("daemon connection poisoned");
            match db::get_run_by_id(&conn, &run_id).map_err(RunError::Daemon)? {
                Some(run) => run.output,
                None => return Ok(Vec::new()),
            }
        };
        output::load_records(&output, scheduled_run_id, type_name, limit)
    }

    /// Create-or-update a Run for the given slug. Matches the
    /// "auto-create on first Run live" pattern Studio will adopt in
    /// Phase 3 — `slug` is the canonical key here, not a generated id.
    pub fn configure_run(&self, slug: &str, cfg: RunConfig) -> Result<Run, DaemonError> {
        // Reject bad cron expressions up front so we don't store
        // unparseable state. Interval / Manual don't need validation.
        if let Cadence::Cron { expr } = &cfg.cadence {
            scheduler::validate_cron(expr)?;
        }
        let now_ms = self.now_ms();
        let result = {
            let conn = self.connection.lock().expect("daemon connection poisoned");
            let existing = db::get_run_by_slug(&conn, slug)?;
            let is_update = existing.is_some();
            let run = match existing {
                Some(prev) => Run {
                    enabled: cfg.enabled,
                    cadence: cfg.cadence,
                    output: cfg.output,
                    // Re-enabling a previously-paused run clears the
                    // Paused label but we don't have its real health
                    // yet — first scheduler fire will refresh it.
                    // Unknown captures "no signal yet" honestly.
                    health: match (cfg.enabled, prev.enabled) {
                        (true, false) => Health::Unknown,
                        (true, true) => prev.health,
                        (false, _) => Health::Paused,
                    },
                    next_run: None, // recomputed below
                    ..prev
                },
                None => Run {
                    id: ulid::Ulid::new().to_string(),
                    recipe_slug: slug.to_string(),
                    workspace_root: self.workspace_root.clone(),
                    enabled: cfg.enabled,
                    cadence: cfg.cadence,
                    output: cfg.output,
                    health: if cfg.enabled {
                        Health::Unknown
                    } else {
                        Health::Paused
                    },
                    next_run: None,
                },
            };
            let next_run = scheduler::next_fire_for(&run, now_ms);
            let run = Run { next_run, ..run };

            if is_update {
                db::update_run(&conn, &run)?;
            } else {
                db::insert_run(&conn, &run)?;
            }
            run
        };
        self.schedule_changed.notify_one();
        Ok(result)
    }

    pub fn remove_run(&self, run_id: &str) -> Result<(), DaemonError> {
        {
            let conn = self.connection.lock().expect("daemon connection poisoned");
            db::delete_run(&conn, run_id)?;
        }
        self.schedule_changed.notify_one();
        Ok(())
    }

    /// Fire a run manually. Equivalent to `run_once(run_id, Trigger::Manual)`.
    pub async fn trigger_run(
        self: &Arc<Self>,
        run_id: &str,
    ) -> Result<ScheduledRun, RunError> {
        self.run_once(run_id, Trigger::Manual).await
    }

    /// Create a default Run for `slug` if none exists yet. Idempotent:
    /// returns the existing row when present. Used by Studio's "Run
    /// live" path on a recipe without a Run yet.
    pub fn ensure_run(&self, slug: &str) -> Result<Run, DaemonError> {
        if let Some(existing) = self.get_run_by_slug(slug)? {
            return Ok(existing);
        }
        let default_output = self.default_output_path(slug);
        let cfg = RunConfig {
            cadence: Cadence::Manual,
            output: default_output,
            enabled: true,
        };
        self.configure_run(slug, cfg)
    }

    /// Where the output store sits for `slug` when the user hasn't
    /// configured a custom path: `<workspace>/.forage/data/<slug>.sqlite`.
    pub fn default_output_path(&self, slug: &str) -> PathBuf {
        self.daemon_dir.join("data").join(format!("{slug}.sqlite"))
    }

    // --- host hooks --------------------------------------------------

    pub fn set_browser_driver(&self, driver: Arc<dyn LiveBrowserDriver>) {
        *self.browser_driver.lock().expect("driver slot poisoned") = Some(driver);
    }

    pub fn set_host_progress(&self, sink: Arc<dyn ProgressSink>) {
        *self.host_progress.lock().expect("host progress poisoned") = Some(sink);
    }

    pub fn on_run_completed(&self, cb: RunCompletedCb) {
        *self.run_completed_cb.lock().expect("cb poisoned") = Some(cb);
    }

}

/// Forwards engine progress to the host sink (if set). Counts are
/// derived from the resulting `Snapshot` rather than tallied here —
/// the snapshot is the source of truth for what got emitted, so a
/// parallel tally would just be a slightly-out-of-sync duplicate.
///
/// The host sink is a snapshot taken at `run_once` start: changing
/// the host sink mid-run isn't a supported flow.
pub(crate) struct ProgressForwarder {
    pub(crate) host: Option<Arc<dyn ProgressSink>>,
}

impl ProgressSink for ProgressForwarder {
    fn emit(&self, event: RunEvent) {
        if let Some(host) = &self.host {
            host.emit(event);
        }
    }
}
