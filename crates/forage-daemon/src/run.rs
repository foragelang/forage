//! One execution of a `Run`. Loads the deployed source + catalog,
//! executes against the appropriate engine, writes emitted records
//! to the output store, and persists a `ScheduledRun` row capturing
//! what happened.
//!
//! Always produces a `ScheduledRun` — even when the engine fails or
//! the Run has no deployment yet — so the consumer (Studio) can
//! render a failure row in the history table. Only setup-level errors
//! (DB corruption, missing run row) bubble out as `Err`.

use std::path::Path;
use std::sync::Arc;

use forage_core::ast::EngineKind;
use forage_core::{EvalValue, Recipe, TypeCatalog};
use forage_http::{Engine, LiveTransport};
use indexmap::IndexMap;

use crate::error::{DaemonError, RunError};
use crate::model::{Outcome, ScheduledRun, Trigger};
use crate::output::{OutputStore, derive_schema};
use crate::{Daemon, ProgressForwarder};

impl Daemon {
    /// Drive a single run cycle. Returns the resulting `ScheduledRun`
    /// on success *and* on engine failure (the row is persisted in
    /// both cases). Only setup-level errors (DB corruption, missing
    /// run row) bubble out as `Err`.
    pub async fn run_once(
        self: &Arc<Self>,
        run_id: &str,
        trigger: Trigger,
    ) -> Result<ScheduledRun, RunError> {
        let started_ms = self.now_ms();
        let scheduled_run_id = ulid::Ulid::new().to_string();

        let run = {
            let conn = self.connection.lock().expect("daemon connection poisoned");
            crate::db::get_run_by_id(&conn, run_id)?.ok_or_else(|| {
                RunError::Daemon(DaemonError::UnknownRun {
                    id: run_id.to_string(),
                })
            })?
        };

        let outcome = execute(self, &run, &scheduled_run_id, started_ms).await;
        let finished_ms = self.now_ms();
        let duration_s = ((finished_ms - started_ms).max(0) as f64) / 1000.0;
        let scheduled = match outcome {
            Ok(ok) => ScheduledRun {
                id: scheduled_run_id,
                run_id: run.id.clone(),
                at: started_ms,
                trigger,
                outcome: Outcome::Ok,
                duration_s,
                counts: ok.counts,
                diagnostics: ok.diagnostics,
                stall: None,
            },
            Err(failure) => ScheduledRun {
                id: scheduled_run_id,
                run_id: run.id.clone(),
                at: started_ms,
                trigger,
                outcome: Outcome::Fail,
                duration_s,
                counts: std::collections::BTreeMap::new(),
                diagnostics: failure.diagnostics,
                stall: Some(failure.message),
            },
        };

        // Persist + recompute health + refresh next-run timestamp in
        // a single SQL transaction so a crash between the two writes
        // can't leave `Run.health` mismatched with the latest
        // `ScheduledRun`.
        {
            let mut conn = self.connection.lock().expect("daemon connection poisoned");
            let tx = conn
                .transaction()
                .map_err(|e| RunError::Daemon(DaemonError::Sqlite(e)))?;
            crate::db::insert_scheduled_run(&tx, &scheduled)?;
            let prior_ok = crate::db::list_prior_ok_scheduled_runs(
                &tx,
                &run.id,
                scheduled.at,
                crate::health::PRIOR_WINDOW as u32,
            )?;
            let mut health = crate::health::derive_health(&scheduled, &prior_ok);
            // A paused run is never auto-promoted to Ok by a manual
            // trigger — Studio toggles the enabled flag back on
            // explicitly to clear that label.
            if !run.enabled {
                health = crate::model::Health::Paused;
            }
            let now_ms = self.now_ms();
            let next_run = crate::scheduler::advance_next_run(&run, now_ms);
            let updated = crate::model::Run {
                health,
                next_run,
                ..run.clone()
            };
            crate::db::update_run(&tx, &updated)?;
            tx.commit()
                .map_err(|e| RunError::Daemon(DaemonError::Sqlite(e)))?;
        }

        // Schedule may need to recompute (next_run changed). Wake the
        // scheduler so it picks up the new fire time.
        self.schedule_changed.notify_one();

        if let Some(cb) = self.run_completed_cb.lock().expect("cb poisoned").as_ref() {
            cb(&scheduled);
        }
        Ok(scheduled)
    }
}

struct RunSuccess {
    counts: std::collections::BTreeMap<String, u32>,
    diagnostics: u32,
}

struct RunFailure {
    message: String,
    diagnostics: u32,
}

async fn execute(
    daemon: &Arc<Daemon>,
    run: &crate::model::Run,
    scheduled_run_id: &str,
    scheduled_at_ms: i64,
) -> Result<RunSuccess, RunFailure> {
    let Some(version) = run.deployed_version else {
        return Err(RunFailure {
            message: "recipe not deployed".to_string(),
            diagnostics: 0,
        });
    };
    let deployed = match daemon.load_deployed(&run.recipe_slug, version) {
        Ok(d) => d,
        Err(e) => {
            return Err(RunFailure {
                message: format!("load deployment v{version}: {e}"),
                diagnostics: 0,
            });
        }
    };
    let recipe = match forage_core::parse(&deployed.source) {
        Ok(r) => r,
        Err(e) => {
            return Err(RunFailure {
                message: format!("parse deployed source: {e}"),
                diagnostics: 0,
            });
        }
    };
    // The catalog was validated against the source at deploy time;
    // we trust it here without re-validating. A parser version drift
    // since deploy would surface above in `forage_core::parse`.
    let catalog: TypeCatalog = deployed.catalog.into();

    // Inputs live on disk next to the user's edit-folder recipe.
    // Drafts and deployed versions share inputs intentionally — the
    // user wants to iterate on a recipe's fixture without redeploying
    // each time.
    let recipe_path = run
        .workspace_root
        .join(&run.recipe_slug)
        .join("recipe.forage");
    let inputs = load_inputs(&recipe_path);
    let secrets = load_secrets(&recipe);

    let tables = derive_schema(&recipe, &catalog);
    let mut store = match OutputStore::open(&run.output, tables) {
        Ok(s) => s,
        Err(e) => {
            return Err(RunFailure {
                message: format!("open output store: {e}"),
                diagnostics: 0,
            });
        }
    };

    let host_progress = daemon
        .host_progress
        .lock()
        .expect("host progress poisoned")
        .clone();
    let sink: Arc<dyn forage_http::ProgressSink> = Arc::new(ProgressForwarder {
        host: host_progress,
    });

    let snapshot_result = match recipe.engine_kind {
        EngineKind::Http => {
            let transport = match LiveTransport::new() {
                Ok(t) => t,
                Err(e) => {
                    return Err(RunFailure {
                        message: format!("http transport: {e}"),
                        diagnostics: 0,
                    });
                }
            };
            let engine = Engine::new(&transport).with_progress(sink.clone());
            engine
                .run(&recipe, inputs, secrets)
                .await
                .map_err(|e| RunFailure {
                    message: format!("engine: {e}"),
                    diagnostics: 0,
                })
        }
        EngineKind::Browser => {
            let Some(driver) = daemon
                .browser_driver
                .lock()
                .expect("driver poisoned")
                .clone()
            else {
                return Err(RunFailure {
                    message: "browser engine requires a LiveBrowserDriver — none registered".into(),
                    diagnostics: 0,
                });
            };
            driver
                .run_live(&recipe, inputs, secrets, sink.clone())
                .await
                .map_err(|e| RunFailure {
                    message: format!("browser: {e}"),
                    diagnostics: 0,
                })
        }
    };

    let snapshot = match snapshot_result {
        Ok(s) => s,
        Err(f) => return Err(f),
    };

    // Persist every emitted record under one transaction so a failed
    // write rolls back the whole batch.
    if let Err(e) = write_records(&mut store, scheduled_run_id, scheduled_at_ms, &snapshot) {
        return Err(RunFailure {
            message: format!("write records: {e}"),
            diagnostics: 0,
        });
    }

    let mut counts: std::collections::BTreeMap<String, u32> = std::collections::BTreeMap::new();
    for rec in &snapshot.records {
        *counts.entry(rec.type_name.clone()).or_insert(0) += 1;
    }
    Ok(RunSuccess {
        counts,
        diagnostics: 0,
    })
}

fn write_records(
    store: &mut OutputStore,
    scheduled_run_id: &str,
    at_ms: i64,
    snapshot: &forage_core::Snapshot,
) -> Result<(), RunError> {
    let mut tx = store.begin_tx()?;
    for rec in &snapshot.records {
        tx.write_record(
            scheduled_run_id,
            at_ms,
            &rec.id,
            &rec.type_name,
            &rec.fields,
        )?;
    }
    tx.commit()
}

/// Inputs convention matches the CLI / Studio: per-recipe
/// `fixtures/inputs.json` sitting next to the `recipe.forage`. Absent
/// file → empty input map.
fn load_inputs(recipe_path: &Path) -> IndexMap<String, EvalValue> {
    let Some(dir) = recipe_path.parent() else {
        return IndexMap::new();
    };
    let path = dir.join("fixtures").join("inputs.json");
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return IndexMap::new();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return IndexMap::new();
    };
    let mut out = IndexMap::new();
    if let serde_json::Value::Object(o) = value {
        for (k, v) in o {
            out.insert(k, EvalValue::from(&v));
        }
    }
    out
}

/// Secrets convention matches the CLI / Studio: each declared secret
/// resolves via `FORAGE_SECRET_<NAME>` env var. Unset env vars → not
/// in the map (the engine treats missing-secret as a recipe error).
fn load_secrets(recipe: &Recipe) -> IndexMap<String, String> {
    let mut out = IndexMap::new();
    for s in &recipe.secrets {
        let key = format!("FORAGE_SECRET_{}", s.to_uppercase());
        if let Ok(v) = std::env::var(&key) {
            out.insert(s.clone(), v);
        }
    }
    out
}
