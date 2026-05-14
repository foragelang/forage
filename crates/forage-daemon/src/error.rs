//! Daemon error types. Two flavors: `DaemonError` for setup, persistence,
//! and configuration; `RunError` for things that go wrong inside one
//! `run_once` cycle.
//!
//! A `RunError` is *always* persisted as a failed `ScheduledRun` —
//! callers see both the typed error (via `Result`) and the recorded
//! row. `DaemonError` is for failures that prevent recording the
//! scheduled-run at all (DB unreachable, the Run doesn't exist).

use std::path::PathBuf;

use forage_core::parse::ParseError;
use forage_core::workspace::WorkspaceError;
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
    #[error("workspace error: {0}")]
    Workspace(#[from] WorkspaceError),
    #[error("no workspace found at {root}")]
    NoWorkspace { root: PathBuf },
    #[error("corrupt daemon state: {detail}")]
    Corrupt { detail: String },
    #[error("corrupt daemon DB: {detail}")]
    CorruptDb { detail: String },
    #[error("invalid cron expression '{expr}': {detail}")]
    BadCron { expr: String, detail: String },
}

/// Errors that surface during a single recipe run. Every variant
/// produces a failed `ScheduledRun` row; the variant determines the
/// `stall` message that ends up persisted.
#[derive(Debug, Error)]
pub enum RunError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("workspace: {0}")]
    Workspace(#[from] WorkspaceError),
    #[error("workspace not found at {root}")]
    NoWorkspace { root: PathBuf },
    #[error("recipe file not found at {path}")]
    RecipeMissing { path: PathBuf },
    #[error("parse error: {0}")]
    Parse(#[from] ParseError),
    #[error("validation failed: {detail}")]
    Validation { detail: String },
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
