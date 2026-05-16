//! One execution of a `Run`. Loads the deployed linked module,
//! executes through the engine-agnostic `forage_core::run_recipe`
//! entry point, writes emitted records to the output store, and
//! persists a `ScheduledRun` row capturing what happened.
//!
//! Always produces a `ScheduledRun` — even when the engine fails or
//! the Run has no deployment yet — so the consumer (Studio) can
//! render a failure row in the history table. Only setup-level errors
//! (DB corruption, missing run row) bubble out as `Err`.
//!
//! The runtime (forage-core) owns the composition walk; the daemon's
//! only run-side responsibilities are loading the deployed closure,
//! wiring the appropriate drivers (HTTP transport, browser replay,
//! browser live), and persisting the resulting snapshot.

use std::sync::Arc;

use async_trait::async_trait;
use forage_core::{
    Drivers, EvalValue, LinkedModule, PriorRecords, RecipeDriver, RunOptions, Snapshot,
    TypeCatalog, run_recipe,
};
use forage_http::{HttpDriver, LiveTransport, ProgressSink, ReplayTransport};
use indexmap::IndexMap;

use crate::error::{DaemonError, RunError};
use crate::model::{Outcome, RunFlags, ScheduledRun, Trigger};
use crate::output::{OutputStore, derive_schema};
use crate::{Daemon, LiveBrowserDriver, ProgressForwarder};

impl Daemon {
    /// Drive a single run cycle. Returns the resulting `ScheduledRun`
    /// on success *and* on engine failure (the row is persisted in
    /// both cases). Only setup-level errors (DB corruption, missing
    /// run row) bubble out as `Err`.
    pub async fn run_once(
        self: &Arc<Self>,
        run_id: &str,
        trigger: Trigger,
        flags: RunFlags,
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

        let outcome = execute(self, &run, &scheduled_run_id, started_ms, &flags).await;
        let finished_ms = self.now_ms();
        let duration_s = ((finished_ms - started_ms).max(0) as f64) / 1000.0;
        // `recipe_version` is `None` only when the engine never got the
        // chance to run a deployed source (no `deployed_version` on the
        // Run row). Every other arm — engine success, engine failure
        // after a successful `load_deployed` — carries the version that
        // was resolved, so per-version emit counts stay reconstructible
        // after later deploys.
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
                recipe_version: Some(ok.version),
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
                recipe_version: failure.version,
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

    /// Run an ephemeral composition over a list of stage names.
    /// Mirrors the notebook surface: each stage resolves to its
    /// currently-deployed version, the closure of every stage's
    /// deployed module folds into one synthetic module rooted at a
    /// composition body, and the runtime walks the chain. Inputs
    /// flow into stage 1; downstream stages consume the upstream
    /// record stream.
    ///
    /// Always runs in ephemeral mode — the notebook is a playground;
    /// the snapshot is returned in-memory and never written to a
    /// daemon output store. `flags.replay` and `flags.sample_limit`
    /// carry through to every stage.
    pub async fn run_composition(
        self: &Arc<Self>,
        name: &str,
        stage_names: Vec<String>,
        inputs: IndexMap<String, EvalValue>,
        flags: RunFlags,
    ) -> Result<Snapshot, RunError> {
        if stage_names.is_empty() {
            return Err(RunError::Engine("composition has zero stages".into()));
        }
        let module = self
            .synthesize_composition_module(name, &stage_names)
            .map_err(RunError::Engine)?;
        let sink: Arc<dyn ProgressSink> = Arc::new(ProgressForwarder {
            host: self
                .host_progress
                .lock()
                .expect("host progress poisoned")
                .clone(),
        });
        let engine_options = RunOptions {
            sample_limit: flags.sample_limit,
        };
        let replay_captures = match flags.replay.as_deref() {
            Some(path) => Some(
                forage_replay::read_jsonl(path)
                    .map_err(|e| RunError::Engine(format!("replay {}: {e}", path.display())))?,
            ),
            None => None,
        };
        let browser_driver = self.browser_driver.lock().expect("driver poisoned").clone();
        let snapshot = run_with_drivers(
            &module,
            inputs,
            IndexMap::new(),
            &engine_options,
            sink,
            browser_driver,
            replay_captures.as_deref(),
        )
        .await
        .map_err(|f| RunError::Engine(f.message))?;
        Ok(snapshot)
    }

    /// Build a linked module for an ephemeral composition. Each
    /// stage's currently-deployed module supplies its root recipe
    /// (which lands in the synthetic module's stage map) and its own
    /// catalog (merged into the synthetic root's catalog). The
    /// synthetic root has a composition body referencing the supplied
    /// stage names in order.
    fn synthesize_composition_module(
        &self,
        name: &str,
        stage_names: &[String],
    ) -> Result<LinkedModule, String> {
        use forage_core::LinkedRecipe;
        use forage_core::ast::{
            Composition, EngineKind, ForageFile, RecipeBody, RecipeHeader, RecipeRef, Span,
        };
        use forage_core::workspace::SerializableCatalog;

        let mut stages: std::collections::BTreeMap<String, LinkedRecipe> =
            std::collections::BTreeMap::new();
        let mut catalog = TypeCatalog::default();
        let mut stage_refs: Vec<RecipeRef> = Vec::with_capacity(stage_names.len());
        for stage_name in stage_names {
            let current = self
                .current_deployed(stage_name)
                .map_err(|e| format!("compose stage '{stage_name}' deployed-version lookup: {e}"))?
                .ok_or_else(|| format!("compose stage '{stage_name}' has no deployed version"))?;
            let deployed = self
                .load_deployed(stage_name, current.version)
                .map_err(|e| {
                    format!(
                        "compose stage '{stage_name}' load v{}: {e}",
                        current.version
                    )
                })?;
            let stage_catalog: TypeCatalog = deployed.module.catalog.clone().into();
            for (k, v) in stage_catalog.types {
                catalog.types.entry(k).or_insert(v);
            }
            for (k, v) in stage_catalog.enums {
                catalog.enums.entry(k).or_insert(v);
            }
            // Adopt every linked stage from the deployed module too —
            // a stage that's itself a composition pulls its own
            // closure in. Names not already present win; the first
            // deployed module to introduce a name keeps it (which is
            // fine because every deployment is a coherent closure
            // unto itself).
            for (k, v) in deployed.module.stages {
                stages.entry(k).or_insert(v);
            }
            // The root of the deployed module is the stage itself.
            // Use `insert` (not `or_insert`) so the explicitly-named
            // stage always reflects the current deployment — an
            // earlier stage's frozen closure may carry a transitive
            // entry under the same name pinned at an older version;
            // the user's explicit listing overrides it.
            stages.insert(stage_name.clone(), deployed.module.root);
            stage_refs.push(RecipeRef {
                author: None,
                name: stage_name.clone(),
                span: Span::default(),
            });
        }
        // Synthesize a root composition recipe carrying just enough
        // shape to drive `run_recipe`. The validator already ran at
        // each stage's deploy time; the synthetic root never reaches
        // a validator pass.
        let synthetic = ForageFile {
            recipe_headers: vec![RecipeHeader {
                name: name.to_string(),
                engine_kind: EngineKind::Http,
                span: Span::default(),
            }],
            types: Vec::new(),
            enums: Vec::new(),
            inputs: Vec::new(),
            emits: None,
            secrets: Vec::new(),
            functions: Vec::new(),
            auth: None,
            browser: None,
            body: RecipeBody::Composition(Composition {
                stages: stage_refs,
                span: Span::default(),
            }),
            expectations: Vec::new(),
            source: std::sync::Arc::from(""),
        };
        Ok(LinkedModule {
            root: LinkedRecipe::from_file(synthetic),
            stages,
            catalog: SerializableCatalog::from(catalog),
        })
    }
}

struct RunSuccess {
    counts: std::collections::BTreeMap<String, u32>,
    diagnostics: u32,
    /// Deployed version the engine just executed. Always `Some` because
    /// success implies `load_deployed` returned a record.
    version: u32,
}

struct RunFailure {
    message: String,
    diagnostics: u32,
    /// The version that was resolved before the failure happened, or
    /// `None` if the failure happened before a version could be
    /// resolved (the `run.deployed_version == None` short-circuit).
    version: Option<u32>,
}

async fn execute(
    daemon: &Arc<Daemon>,
    run: &crate::model::Run,
    scheduled_run_id: &str,
    scheduled_at_ms: i64,
    flags: &RunFlags,
) -> Result<RunSuccess, RunFailure> {
    let Some(version) = run.deployed_version else {
        return Err(RunFailure {
            message: "recipe not deployed".to_string(),
            diagnostics: 0,
            version: None,
        });
    };
    let deployed = match daemon.load_deployed(&run.recipe_name, version) {
        Ok(d) => d,
        Err(e) => {
            return Err(RunFailure {
                message: format!("load deployment v{version}: {e}"),
                diagnostics: 0,
                version: Some(version),
            });
        }
    };
    let module = deployed.module;

    // Inputs come from the explicit `Run.inputs` field set via
    // `configure_run`. Recipes that declare `input` bindings must have
    // them configured on the row — there's no implicit filesystem
    // fallback.
    let inputs: IndexMap<String, EvalValue> = run
        .inputs
        .iter()
        .map(|(k, v)| (k.clone(), EvalValue::from(v)))
        .collect();
    let secrets = load_secrets(&module);

    let tables = derive_schema(&module);
    // Ephemeral runs land in an in-memory store; persistent runs write
    // to the configured `Run.output` path. The flag is invocation-level
    // (scheduled fires never set it), so the persistent table stays
    // representative of what the recipe actually produces in prod.
    let store_result = if flags.ephemeral {
        OutputStore::ephemeral(tables)
    } else {
        OutputStore::open(&run.output, tables)
    };
    let mut store = match store_result {
        Ok(s) => s,
        Err(e) => {
            return Err(RunFailure {
                message: format!("open output store: {e}"),
                diagnostics: 0,
                version: Some(version),
            });
        }
    };

    let host_progress = daemon
        .host_progress
        .lock()
        .expect("host progress poisoned")
        .clone();
    let sink: Arc<dyn ProgressSink> = Arc::new(ProgressForwarder {
        host: host_progress,
    });

    let engine_options = RunOptions {
        sample_limit: flags.sample_limit,
    };
    // `--replay` loads captures off the supplied JSONL file and feeds
    // them into a ReplayTransport at every stage. A missing file is a
    // setup error — the caller asked for replay against a path that
    // doesn't have captures, so failing the run loudly is more useful
    // than silently running live.
    let replay_captures = match flags.replay.as_deref() {
        Some(path) => match forage_replay::read_jsonl(path) {
            Ok(c) => Some(c),
            Err(e) => {
                return Err(RunFailure {
                    message: format!("replay {}: {e}", path.display()),
                    diagnostics: 0,
                    version: Some(version),
                });
            }
        },
        None => None,
    };

    let browser_driver = daemon
        .browser_driver
        .lock()
        .expect("driver poisoned")
        .clone();
    let snapshot_result = run_with_drivers(
        &module,
        inputs,
        secrets,
        &engine_options,
        sink,
        browser_driver,
        replay_captures.as_deref(),
    )
    .await;
    let snapshot = match snapshot_result {
        Ok(s) => s,
        Err(f) => {
            return Err(RunFailure {
                version: Some(version),
                ..f
            });
        }
    };

    // Persist every emitted record under one transaction so a failed
    // write rolls back the whole batch.
    if let Err(e) = write_records(&mut store, scheduled_run_id, scheduled_at_ms, &snapshot) {
        return Err(RunFailure {
            message: format!("write records: {e}"),
            diagnostics: 0,
            version: Some(version),
        });
    }

    let mut counts: std::collections::BTreeMap<String, u32> = std::collections::BTreeMap::new();
    for rec in &snapshot.records {
        *counts.entry(rec.type_name.clone()).or_insert(0) += 1;
    }
    Ok(RunSuccess {
        counts,
        diagnostics: 0,
        version,
    })
}

/// Build the HTTP + browser driver pair the runtime dispatches
/// through, then hand the module to `forage_core::run_recipe`. The
/// HTTP driver swaps between live and replay transports based on
/// `replay_captures`; the browser driver swaps between replay and the
/// host-supplied live driver based on the same flag.
///
/// Driver construction borrows the live transport / captures /
/// progress sink with explicit lifetimes — that's why this helper
/// owns the construction inline rather than living in `Daemon`.
#[allow(clippy::too_many_arguments)]
async fn run_with_drivers(
    module: &LinkedModule,
    inputs: IndexMap<String, EvalValue>,
    secrets: IndexMap<String, String>,
    options: &RunOptions,
    sink: Arc<dyn ProgressSink>,
    live_browser: Option<Arc<dyn LiveBrowserDriver>>,
    replay_captures: Option<&[forage_replay::Capture]>,
) -> Result<Snapshot, RunFailure> {
    // HTTP transport branches on replay availability. Both transport
    // values have to outlive the runtime call, so we bind one or the
    // other to a local and pass a `&dyn Transport` into the driver.
    let live_transport = if replay_captures.is_none() {
        Some(LiveTransport::new().map_err(|e| RunFailure {
            message: format!("http transport: {e}"),
            diagnostics: 0,
            version: None,
        })?)
    } else {
        None
    };
    let replay_transport = replay_captures.map(|c| ReplayTransport::new(c.to_vec()));
    let http_transport: &dyn forage_http::Transport = match (&live_transport, &replay_transport) {
        (Some(t), _) => t,
        (None, Some(t)) => t,
        (None, None) => unreachable!("exactly one transport is constructed above"),
    };
    let http_driver = HttpDriver::new(http_transport).with_progress(sink.clone());

    let browser_driver: Box<dyn RecipeDriver + '_> = match (replay_captures, live_browser) {
        (Some(captures), _) => Box::new(forage_browser::BrowserReplayDriver::new(captures)),
        (None, Some(driver)) => Box::new(LiveBrowserAdapter {
            driver,
            sink: sink.clone(),
        }),
        (None, None) => Box::new(forage_http::UnsupportedDriver::new(
            "browser engine requires a LiveBrowserDriver — none registered".to_string(),
        )),
    };

    let drivers = Drivers {
        http: &http_driver,
        browser: browser_driver.as_ref(),
    };

    run_recipe(module, inputs, secrets, options, &drivers)
        .await
        .map_err(|e| RunFailure {
            message: format!("engine: {e}"),
            diagnostics: 0,
            version: None,
        })
}

/// Bridge from the daemon's `LiveBrowserDriver` trait (host-supplied
/// — Studio provides the webview) onto the runtime's `RecipeDriver`
/// shape. The progress sink threads through unchanged so the host
/// keeps seeing live events.
struct LiveBrowserAdapter {
    driver: Arc<dyn LiveBrowserDriver>,
    sink: Arc<dyn ProgressSink>,
}

#[async_trait]
impl RecipeDriver for LiveBrowserAdapter {
    async fn run_scraping(
        &self,
        recipe: &forage_core::ForageFile,
        catalog: &TypeCatalog,
        inputs: IndexMap<String, EvalValue>,
        secrets: IndexMap<String, String>,
        options: &RunOptions,
        _prior: PriorRecords,
    ) -> Result<Snapshot, forage_core::RunError> {
        self.driver
            .run_live(recipe, catalog, inputs, secrets, self.sink.clone(), options)
            .await
            .map_err(|e| forage_core::RunError::Driver(format!("browser: {e}")))
    }
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

/// Secrets convention matches the CLI / Studio: each declared secret
/// resolves via `FORAGE_SECRET_<NAME>` env var. Unset env vars → not
/// in the map (the engine treats missing-secret as a recipe error).
fn load_secrets(module: &LinkedModule) -> IndexMap<String, String> {
    // Pull every secret name declared anywhere in the closure so a
    // composition stage that declares its own secrets still sees the
    // env-resolved value at run time.
    let mut names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for s in &module.root.file.secrets {
        names.insert(s.clone());
    }
    for stage in module.stages.values() {
        for s in &stage.file.secrets {
            names.insert(s.clone());
        }
    }
    let mut out = IndexMap::new();
    for name in names {
        let key = format!("FORAGE_SECRET_{}", name.to_uppercase());
        if let Ok(v) = std::env::var(&key) {
            out.insert(name, v);
        }
    }
    out
}
