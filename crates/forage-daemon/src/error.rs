//! Daemon error types. Three flavors: `DaemonError` for setup,
//! persistence, and configuration; `DeployError` for the deploy
//! pipeline (parse + validate + write); `RunError` for things that go
//! wrong inside one `run_once` cycle.
//!
//! A `RunError` is *always* persisted as a failed `ScheduledRun` —
//! callers see both the typed error (via `Result`) and the recorded
//! row. `DaemonError` is for failures that prevent recording the
//! scheduled-run at all (DB unreachable, the Run doesn't exist).

use forage_core::parse::ParseError;
use thiserror::Error;

use crate::BrowserDriverError;

#[derive(Debug, Error)]
pub enum DaemonError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("serde error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("unknown run id: {id}")]
    UnknownRun { id: String },
    #[error("unknown deployment: {slug} v{version}")]
    UnknownDeployment { slug: String, version: u32 },
    #[error("corrupt daemon state: {detail}")]
    Corrupt { detail: String },
    #[error("corrupt daemon DB: {detail}")]
    CorruptDb { detail: String },
    #[error("invalid cron expression '{expr}': {detail}")]
    BadCron { expr: String, detail: String },
}

/// Errors that surface from `Daemon::deploy`. Parse and validation
/// failures abort the deploy before any row is inserted or file is
/// written, so the daemon's fortress invariant — only validated,
/// frozen versions enter the store — holds even when callers feed
/// junk.
#[derive(Debug, Error)]
pub enum DeployError {
    #[error("parse: {0}")]
    Parse(String),
    #[error("validate: {0}")]
    Validate(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("daemon: {0}")]
    Daemon(#[from] DaemonError),
}

/// Errors that surface during a single recipe run. Every variant
/// produces a failed `ScheduledRun` row; the variant determines the
/// `stall` message that ends up persisted.
#[derive(Debug, Error)]
pub enum RunError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("parse error: {0}")]
    Parse(#[from] ParseError),
    #[error("engine: {0}")]
    Engine(String),
    #[error("browser: {0}")]
    Browser(#[source] BrowserDriverError),
    #[error("output store: {0}")]
    Output(String),
    #[error("browser-engine recipes require a LiveBrowserDriver — none registered")]
    NoBrowserDriver,
    #[error("daemon: {0}")]
    Daemon(#[from] DaemonError),
}
