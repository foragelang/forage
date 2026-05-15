//! Daemon-database persistence. Holds the schedule (`runs`) and the
//! history (`scheduled_runs`) for a single workspace.
//!
//! The DB lives at `<workspace_root>/.forage/daemon.sqlite`. One
//! connection is owned by the `Daemon` and protected by a Mutex —
//! every operation is short-lived (a few rows), so contention isn't a
//! concern, and the sync rusqlite API stays compatible with both
//! sync API consumers (`list_runs`, `configure_run`) and the async
//! `run_once` flow (which does its DB work in the same thread between
//! engine awaits).
//!
//! Greenfield, pre-1.0: there is one schema. When it changes, we edit
//! the CREATE statements below and the caller is expected to
//! `rm -rf .forage` to pick the new shape up. No version tracking, no
//! in-place migration.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, params};

use crate::error::DaemonError;
use crate::model::{
    Cadence, DeployedVersion, Health, Outcome, OutputFormat, Run, ScheduledRun, Trigger,
};

/// Open the daemon DB and ensure the schema is in place.
pub(crate) fn open_connection(daemon_dir: &Path) -> Result<Connection, DaemonError> {
    std::fs::create_dir_all(daemon_dir)?;
    let db_path = daemon_dir.join("daemon.sqlite");
    let conn = Connection::open(&db_path).map_err(DaemonError::Sqlite)?;
    init_schema(&conn)?;
    Ok(conn)
}

/// Create the daemon schema if the DB is empty. A no-op against an
/// already-initialized DB; an error against a DB initialized at an
/// older shape (callers clear `.forage/` to recover).
fn init_schema(conn: &Connection) -> Result<(), DaemonError> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS runs (
            id               TEXT PRIMARY KEY,
            recipe_name      TEXT NOT NULL,
            workspace_root   TEXT NOT NULL,
            enabled          INTEGER NOT NULL,
            cadence_json     TEXT NOT NULL,
            output_path      TEXT NOT NULL,
            health           TEXT NOT NULL,
            next_run         INTEGER,
            deployed_version INTEGER,
            inputs_json      TEXT NOT NULL,
            output_format    TEXT NOT NULL
        );

        CREATE UNIQUE INDEX IF NOT EXISTS runs_recipe_name ON runs(recipe_name);

        CREATE TABLE IF NOT EXISTS scheduled_runs (
            id              TEXT PRIMARY KEY,
            run_id          TEXT NOT NULL,
            at              INTEGER NOT NULL,
            trigger         TEXT NOT NULL,
            outcome         TEXT NOT NULL,
            duration_s      REAL NOT NULL,
            counts_json     TEXT NOT NULL,
            diagnostics     INTEGER NOT NULL,
            stall           TEXT,
            recipe_version  INTEGER,
            FOREIGN KEY (run_id) REFERENCES runs(id) ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS scheduled_runs_run_id_at
            ON scheduled_runs(run_id, at DESC);

        CREATE TABLE IF NOT EXISTS deployed_versions (
            recipe_name  TEXT NOT NULL,
            version      INTEGER NOT NULL,
            deployed_at  INTEGER NOT NULL,
            PRIMARY KEY (recipe_name, version)
        );
        "#,
    )?;
    Ok(())
}

// --- runs ----------------------------------------------------------------

pub(crate) fn insert_run(conn: &Connection, run: &Run) -> Result<(), DaemonError> {
    let cadence_json = serde_json::to_string(&run.cadence)?;
    let health = health_to_str(run.health);
    let inputs_json = serde_json::to_string(&run.inputs)?;
    let output_format = output_format_to_str(run.output_format);
    conn.execute(
        "INSERT INTO runs(id, recipe_name, workspace_root, enabled, cadence_json, output_path, health, next_run, deployed_version, inputs_json, output_format)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            run.id,
            run.recipe_name,
            run.workspace_root.to_string_lossy(),
            run.enabled as i64,
            cadence_json,
            run.output.to_string_lossy(),
            health,
            run.next_run,
            run.deployed_version,
            inputs_json,
            output_format,
        ],
    )?;
    Ok(())
}

pub(crate) fn update_run(conn: &Connection, run: &Run) -> Result<(), DaemonError> {
    let cadence_json = serde_json::to_string(&run.cadence)?;
    let health = health_to_str(run.health);
    let inputs_json = serde_json::to_string(&run.inputs)?;
    let output_format = output_format_to_str(run.output_format);
    let changed = conn.execute(
        "UPDATE runs SET
            recipe_name      = ?2,
            workspace_root   = ?3,
            enabled          = ?4,
            cadence_json     = ?5,
            output_path      = ?6,
            health           = ?7,
            next_run         = ?8,
            deployed_version = ?9,
            inputs_json      = ?10,
            output_format    = ?11
         WHERE id = ?1",
        params![
            run.id,
            run.recipe_name,
            run.workspace_root.to_string_lossy(),
            run.enabled as i64,
            cadence_json,
            run.output.to_string_lossy(),
            health,
            run.next_run,
            run.deployed_version,
            inputs_json,
            output_format,
        ],
    )?;
    if changed == 0 {
        return Err(DaemonError::UnknownRun { id: run.id.clone() });
    }
    Ok(())
}

pub(crate) fn delete_run(conn: &Connection, run_id: &str) -> Result<(), DaemonError> {
    let changed = conn.execute("DELETE FROM runs WHERE id = ?1", params![run_id])?;
    if changed == 0 {
        return Err(DaemonError::UnknownRun {
            id: run_id.to_string(),
        });
    }
    Ok(())
}

pub(crate) fn get_run_by_id(conn: &Connection, run_id: &str) -> Result<Option<Run>, DaemonError> {
    conn.query_row(
        "SELECT id, recipe_name, workspace_root, enabled, cadence_json, output_path, health, next_run, deployed_version, inputs_json, output_format
         FROM runs WHERE id = ?1",
        params![run_id],
        row_to_run,
    )
    .optional()
    .map_err(DaemonError::Sqlite)?
    .transpose()
}

pub(crate) fn get_run_by_name(conn: &Connection, name: &str) -> Result<Option<Run>, DaemonError> {
    conn.query_row(
        "SELECT id, recipe_name, workspace_root, enabled, cadence_json, output_path, health, next_run, deployed_version, inputs_json, output_format
         FROM runs WHERE recipe_name = ?1",
        params![name],
        row_to_run,
    )
    .optional()
    .map_err(DaemonError::Sqlite)?
    .transpose()
}

pub(crate) fn list_runs(conn: &Connection) -> Result<Vec<Run>, DaemonError> {
    let mut stmt = conn.prepare(
        "SELECT id, recipe_name, workspace_root, enabled, cadence_json, output_path, health, next_run, deployed_version, inputs_json, output_format
         FROM runs ORDER BY recipe_name ASC",
    )?;
    let mut out = Vec::new();
    let rows = stmt.query_map([], row_to_run)?;
    for row in rows {
        out.push(row??);
    }
    Ok(out)
}

fn row_to_run(r: &rusqlite::Row<'_>) -> rusqlite::Result<Result<Run, DaemonError>> {
    let id: String = r.get(0)?;
    let recipe_name: String = r.get(1)?;
    let workspace_root: String = r.get(2)?;
    let enabled: i64 = r.get(3)?;
    let cadence_json: String = r.get(4)?;
    let output_path: String = r.get(5)?;
    let health: String = r.get(6)?;
    let next_run: Option<i64> = r.get(7)?;
    let deployed_version: Option<i64> = r.get(8)?;
    let inputs_json: String = r.get(9)?;
    let output_format: String = r.get(10)?;

    let cadence: Cadence = match serde_json::from_str(&cadence_json) {
        Ok(c) => c,
        Err(e) => return Ok(Err(DaemonError::Serde(e))),
    };
    let health = match health_from_str(&health) {
        Some(h) => h,
        None => {
            return Ok(Err(DaemonError::Corrupt {
                detail: format!("unknown health '{health}' for run {id}"),
            }));
        }
    };
    let deployed_version = match deployed_version {
        Some(v) if v >= 0 && v <= u32::MAX as i64 => Some(v as u32),
        Some(v) => {
            return Ok(Err(DaemonError::Corrupt {
                detail: format!("deployed_version {v} out of u32 range for run {id}"),
            }));
        }
        None => None,
    };
    let inputs: indexmap::IndexMap<String, serde_json::Value> = match serde_json::from_str(
        &inputs_json,
    ) {
        Ok(serde_json::Value::Object(obj)) => obj.into_iter().collect(),
        Ok(other) => {
            return Ok(Err(DaemonError::Corrupt {
                detail: format!(
                    "inputs_json must be a JSON object for run {id}, got {other:?}"
                ),
            }));
        }
        Err(e) => return Ok(Err(DaemonError::Serde(e))),
    };
    let output_format = match output_format_from_str(&output_format) {
        Some(f) => f,
        None => {
            return Ok(Err(DaemonError::Corrupt {
                detail: format!("unknown output_format '{output_format}' for run {id}"),
            }));
        }
    };

    Ok(Ok(Run {
        id,
        recipe_name,
        workspace_root: PathBuf::from(workspace_root),
        enabled: enabled != 0,
        cadence,
        output: PathBuf::from(output_path),
        health,
        next_run,
        deployed_version,
        inputs,
        output_format,
    }))
}

// --- scheduled_runs ------------------------------------------------------

pub(crate) fn insert_scheduled_run(
    conn: &Connection,
    sr: &ScheduledRun,
) -> Result<(), DaemonError> {
    let counts_json = serde_json::to_string(&sr.counts)?;
    conn.execute(
        "INSERT INTO scheduled_runs(id, run_id, at, trigger, outcome, duration_s, counts_json, diagnostics, stall, recipe_version)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            sr.id,
            sr.run_id,
            sr.at,
            trigger_to_str(sr.trigger),
            outcome_to_str(sr.outcome),
            sr.duration_s,
            counts_json,
            sr.diagnostics,
            sr.stall,
            sr.recipe_version,
        ],
    )?;
    Ok(())
}

/// Most recent first; optional `before` cursor for paging through deep history.
pub(crate) fn list_scheduled_runs(
    conn: &Connection,
    run_id: &str,
    limit: u32,
    before: Option<i64>,
) -> Result<Vec<ScheduledRun>, DaemonError> {
    let mut out = Vec::new();
    match before {
        Some(b) => {
            let mut stmt = conn.prepare(
                "SELECT id, run_id, at, trigger, outcome, duration_s, counts_json, diagnostics, stall, recipe_version
                 FROM scheduled_runs
                 WHERE run_id = ?1 AND at < ?2
                 ORDER BY at DESC LIMIT ?3",
            )?;
            let rows = stmt.query_map(params![run_id, b, limit], row_to_scheduled_run)?;
            for row in rows {
                out.push(row??);
            }
        }
        None => {
            let mut stmt = conn.prepare(
                "SELECT id, run_id, at, trigger, outcome, duration_s, counts_json, diagnostics, stall, recipe_version
                 FROM scheduled_runs
                 WHERE run_id = ?1
                 ORDER BY at DESC LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![run_id, limit], row_to_scheduled_run)?;
            for row in rows {
                out.push(row??);
            }
        }
    }
    Ok(out)
}

/// The most recent N ok-outcome scheduled-runs prior to a given timestamp,
/// in time-descending order. Used by drift derivation.
pub(crate) fn list_prior_ok_scheduled_runs(
    conn: &Connection,
    run_id: &str,
    before_at: i64,
    limit: u32,
) -> Result<Vec<ScheduledRun>, DaemonError> {
    let mut stmt = conn.prepare(
        "SELECT id, run_id, at, trigger, outcome, duration_s, counts_json, diagnostics, stall, recipe_version
         FROM scheduled_runs
         WHERE run_id = ?1 AND at < ?2 AND outcome = 'ok'
         ORDER BY at DESC LIMIT ?3",
    )?;
    let mut out = Vec::new();
    let rows = stmt.query_map(params![run_id, before_at, limit], row_to_scheduled_run)?;
    for row in rows {
        out.push(row??);
    }
    Ok(out)
}

fn row_to_scheduled_run(
    r: &rusqlite::Row<'_>,
) -> rusqlite::Result<Result<ScheduledRun, DaemonError>> {
    let id: String = r.get(0)?;
    let run_id: String = r.get(1)?;
    let at: i64 = r.get(2)?;
    let trigger: String = r.get(3)?;
    let outcome: String = r.get(4)?;
    let duration_s: f64 = r.get(5)?;
    let counts_json: String = r.get(6)?;
    let diagnostics: u32 = r.get(7)?;
    let stall: Option<String> = r.get(8)?;
    let recipe_version_raw: Option<i64> = r.get(9)?;

    let trigger = match trigger_from_str(&trigger) {
        Some(t) => t,
        None => {
            return Ok(Err(DaemonError::Corrupt {
                detail: format!("unknown trigger '{trigger}' for scheduled_run {id}"),
            }));
        }
    };
    let outcome = match outcome_from_str(&outcome) {
        Some(o) => o,
        None => {
            return Ok(Err(DaemonError::Corrupt {
                detail: format!("unknown outcome '{outcome}' for scheduled_run {id}"),
            }));
        }
    };
    let counts: BTreeMap<String, u32> = match serde_json::from_str(&counts_json) {
        Ok(c) => c,
        Err(e) => return Ok(Err(DaemonError::Serde(e))),
    };
    let recipe_version = match recipe_version_raw {
        Some(v) if (0..=u32::MAX as i64).contains(&v) => Some(v as u32),
        Some(v) => {
            return Ok(Err(DaemonError::Corrupt {
                detail: format!("recipe_version {v} out of u32 range for scheduled_run {id}"),
            }));
        }
        None => None,
    };
    Ok(Ok(ScheduledRun {
        id,
        run_id,
        at,
        trigger,
        outcome,
        duration_s,
        counts,
        diagnostics,
        stall,
        recipe_version,
    }))
}

// --- deployed_versions ---------------------------------------------------

pub(crate) fn insert_deployed_version(
    conn: &Connection,
    dv: &DeployedVersion,
) -> Result<(), DaemonError> {
    conn.execute(
        "INSERT INTO deployed_versions(recipe_name, version, deployed_at)
         VALUES (?1, ?2, ?3)",
        params![dv.recipe_name, dv.version, dv.deployed_at],
    )?;
    Ok(())
}

pub(crate) fn list_deployed_versions(
    conn: &Connection,
    name: &str,
) -> Result<Vec<DeployedVersion>, DaemonError> {
    let mut stmt = conn.prepare(
        "SELECT recipe_name, version, deployed_at FROM deployed_versions
         WHERE recipe_name = ?1 ORDER BY version DESC",
    )?;
    let rows = stmt.query_map(params![name], row_to_deployed_version)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row??);
    }
    Ok(out)
}

pub(crate) fn latest_deployed_version(
    conn: &Connection,
    name: &str,
) -> Result<Option<DeployedVersion>, DaemonError> {
    conn.query_row(
        "SELECT recipe_name, version, deployed_at FROM deployed_versions
         WHERE recipe_name = ?1 ORDER BY version DESC LIMIT 1",
        params![name],
        row_to_deployed_version,
    )
    .optional()
    .map_err(DaemonError::Sqlite)?
    .transpose()
}

/// One row per recipe: the latest deployed version. Used by Studio's
/// recipe-status surface so it doesn't have to fan out per-recipe.
pub(crate) fn list_latest_per_recipe(
    conn: &Connection,
) -> Result<Vec<DeployedVersion>, DaemonError> {
    let mut stmt = conn.prepare(
        "SELECT dv.recipe_name, dv.version, dv.deployed_at
         FROM deployed_versions dv
         JOIN (
             SELECT recipe_name, MAX(version) AS max_version
             FROM deployed_versions
             GROUP BY recipe_name
         ) latest ON dv.recipe_name = latest.recipe_name AND dv.version = latest.max_version
         ORDER BY dv.recipe_name ASC",
    )?;
    let rows = stmt.query_map([], row_to_deployed_version)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row??);
    }
    Ok(out)
}

fn row_to_deployed_version(
    r: &rusqlite::Row<'_>,
) -> rusqlite::Result<Result<DeployedVersion, DaemonError>> {
    let recipe_name: String = r.get(0)?;
    let version_raw: i64 = r.get(1)?;
    let deployed_at: i64 = r.get(2)?;
    let version = if (0..=u32::MAX as i64).contains(&version_raw) {
        version_raw as u32
    } else {
        return Ok(Err(DaemonError::Corrupt {
            detail: format!("version {version_raw} out of u32 range for recipe {recipe_name}"),
        }));
    };
    Ok(Ok(DeployedVersion {
        recipe_name,
        version,
        deployed_at,
    }))
}

// --- enum <-> string ----------------------------------------------------

fn health_to_str(h: Health) -> &'static str {
    match h {
        Health::Ok => "ok",
        Health::Drift => "drift",
        Health::Fail => "fail",
        Health::Paused => "paused",
        Health::Unknown => "unknown",
    }
}

fn health_from_str(s: &str) -> Option<Health> {
    Some(match s {
        "ok" => Health::Ok,
        "drift" => Health::Drift,
        "fail" => Health::Fail,
        "paused" => Health::Paused,
        "unknown" => Health::Unknown,
        _ => return None,
    })
}

fn trigger_to_str(t: Trigger) -> &'static str {
    match t {
        Trigger::Schedule => "schedule",
        Trigger::Manual => "manual",
    }
}

fn trigger_from_str(s: &str) -> Option<Trigger> {
    Some(match s {
        "schedule" => Trigger::Schedule,
        "manual" => Trigger::Manual,
        _ => return None,
    })
}

fn outcome_to_str(o: Outcome) -> &'static str {
    match o {
        Outcome::Ok => "ok",
        Outcome::Fail => "fail",
    }
}

fn outcome_from_str(s: &str) -> Option<Outcome> {
    Some(match s {
        "ok" => Outcome::Ok,
        "fail" => Outcome::Fail,
        _ => return None,
    })
}

fn output_format_to_str(f: OutputFormat) -> &'static str {
    match f {
        OutputFormat::Json => "json",
        OutputFormat::Jsonld => "jsonld",
    }
}

fn output_format_from_str(s: &str) -> Option<OutputFormat> {
    Some(match s {
        "json" => OutputFormat::Json,
        "jsonld" => OutputFormat::Jsonld,
        _ => return None,
    })
}
