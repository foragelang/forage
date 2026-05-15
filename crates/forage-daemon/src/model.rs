//! Wire types for the daemon. Studio holds these in store entries and
//! passes them through Tauri commands; the daemon writes them to SQLite.
//!
//! All types derive `Serialize`, `Deserialize`, and `TS` so the same
//! definition is the source of truth for Rust persistence, Rust↔Rust
//! API, and the TypeScript Tauri bindings. No `#[default]` on enums —
//! every state must be explicitly chosen at construction time.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A scheduled recipe: the daemon's unit of recurring work.
///
/// A `Run` is created on first explicit Run-live (auto-created via
/// `Daemon::ensure_run`) and configured (cadence, output path, enabled
/// flag) by the user through Studio. The daemon scheduler ticks against
/// `Run.cadence`; each tick produces a `ScheduledRun` record (success
/// *or* failure).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Run {
    /// ULID — sortable, opaque, generated once at creation.
    pub id: String,
    /// The recipe's header name (`recipe "<name>"`). The daemon's
    /// canonical key for the recipe — output stores, deployment dirs,
    /// and scheduled-run rows all anchor on it.
    pub recipe_name: String,
    /// Workspace the recipe lives in. The daemon is per-workspace, so
    /// in practice this matches the daemon's `workspace_root`; carried
    /// on the row so consumers can render context without a back-ref.
    #[ts(type = "string")]
    pub workspace_root: PathBuf,
    pub enabled: bool,
    pub cadence: Cadence,
    /// Output store path — defaults to `<workspace_root>/.forage/data/<recipe_name>.sqlite`.
    #[ts(type = "string")]
    pub output: PathBuf,
    /// Derived from the latest `ScheduledRun` plus the prior 7 ok runs.
    /// See `health::derive_health`.
    pub health: Health,
    /// Next time the scheduler will fire this run, ms since epoch.
    /// `None` for `Cadence::Manual`.
    #[ts(type = "number | null")]
    pub next_run: Option<i64>,
    /// Pointer to the deployed-recipe version the scheduler should
    /// execute. `None` until the slug has been deployed at least once;
    /// scheduled fires against a `None` pointer record a clean failure
    /// rather than crashing, so the user can configure cadence before
    /// the first deploy.
    #[ts(type = "number | null")]
    pub deployed_version: Option<u32>,
}

/// How often the daemon should fire a `Run`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Cadence {
    /// User-triggered only. Scheduler ignores.
    Manual,
    /// Every N units (minutes/hours/days) since the last successful fire.
    Interval { every_n: u32, unit: TimeUnit },
    /// Standard 6-field cron expression: sec min hour dom month dow.
    /// `cron::Schedule::from_str` is the parser; same dialect as the
    /// `cron` crate everywhere.
    Cron { expr: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "lowercase")]
pub enum TimeUnit {
    /// Minutes.
    M,
    /// Hours.
    H,
    /// Days.
    D,
}

/// Per-Run derived health label.
///
/// `Ok`: latest run succeeded and emit counts are not drifting.
/// `Drift`: latest run succeeded but a record type's count fell ≥30%
///   below the median of the prior 7 ok runs.
/// `Fail`: latest run failed.
/// `Paused`: Run.enabled is false. The daemon doesn't compute this from
///   history — Studio sets it when the user toggles the run off.
/// `Unknown`: a freshly created run with no history yet, or the daemon
///   hasn't observed it in this lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "lowercase")]
pub enum Health {
    Ok,
    Drift,
    Fail,
    Paused,
    Unknown,
}

/// One execution of a `Run`. Every fired run produces one row, success
/// or failure. Consumers chart `counts` and `outcome` over time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ScheduledRun {
    /// ULID.
    pub id: String,
    pub run_id: String,
    /// When the run kicked off, ms since epoch.
    #[ts(type = "number")]
    pub at: i64,
    pub trigger: Trigger,
    pub outcome: Outcome,
    pub duration_s: f64,
    /// Emit counts grouped by record type name. BTreeMap so consumers
    /// see consistent ordering across rows; serialized as a JSON object
    /// in storage.
    pub counts: BTreeMap<String, u32>,
    /// Number of validation issues (errors + warnings) the recipe
    /// surfaced. Pure-error runs short-circuit before execution, so a
    /// row with a non-zero `diagnostics` count and `outcome: Fail` is
    /// the validation-blocked path; otherwise the count reflects
    /// validation warnings the engine ran through.
    pub diagnostics: u32,
    /// Free-form failure reason. `None` on success. The diagnostic
    /// report carries structured stall info; this is the
    /// human-readable summary.
    pub stall: Option<String>,
    /// Which deployed version the engine actually executed for this
    /// row. `None` only for the no-deployment short-circuit failure
    /// (where `stall == Some("recipe not deployed")`) and for the
    /// synthetic cron-fail recorded before any version could be
    /// resolved. Every other row — including engine-side failures —
    /// carries the version the engine ran, so emit counts remain
    /// interpretable across deploys.
    #[ts(type = "number | null")]
    pub recipe_version: Option<u32>,
}

/// Why the daemon fired this run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "lowercase")]
pub enum Trigger {
    /// Scheduler tick (interval or cron).
    Schedule,
    /// Explicit user action (Studio "Run live" button or `trigger_run` API).
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    Ok,
    Fail,
}

/// Configuration update payload for `Daemon::configure_run`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RunConfig {
    pub cadence: Cadence,
    #[ts(type = "string")]
    pub output: PathBuf,
    pub enabled: bool,
}

/// Metadata for one frozen deployed version of a recipe. The source
/// and catalog live on disk under `<daemon_dir>/deployments/<recipe_name>/v<n>/`;
/// this is the row recorded in the daemon DB and the wire shape Studio
/// renders in its recipe-status surface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DeployedVersion {
    pub recipe_name: String,
    pub version: u32,
    #[ts(type = "number")]
    pub deployed_at: i64,
}

/// Daemon process status, surfaced to the Studio footer.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DaemonStatus {
    /// True iff the scheduler task is alive. When false, the daemon is
    /// still usable as an API surface (configure, trigger, list) but
    /// scheduled cadences won't fire.
    pub running: bool,
    pub version: String,
    /// When the current scheduler task started, ms since epoch. 0 when
    /// `running` is false.
    #[ts(type = "number")]
    pub started_at: i64,
    /// Number of enabled, non-manual runs the scheduler is currently
    /// tracking.
    pub active_count: u32,
}
