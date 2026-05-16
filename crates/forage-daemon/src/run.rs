//! One execution of a `Run`. Loads the deployed source + catalog,
//! executes against the appropriate engine, writes emitted records
//! to the output store, and persists a `ScheduledRun` row capturing
//! what happened.
//!
//! Always produces a `ScheduledRun` — even when the engine fails or
//! the Run has no deployment yet — so the consumer (Studio) can
//! render a failure row in the history table. Only setup-level errors
//! (DB corruption, missing run row) bubble out as `Err`.
//!
//! Composed recipes walk the chain in `run_stage` — each stage's
//! emitted records feed the next stage's input slot via
//! `Engine::run_with_prior`. Inner stage runs are not surfaced as
//! their own `ScheduledRun` rows; the composition's row carries the
//! aggregate counts and the chain's final snapshot drives output
//! persistence.

use std::sync::Arc;

use forage_core::ast::{Composition, EngineKind, RecipeBody, RecipeHeader, RecipeRef, Span};
use forage_core::{EvalValue, ForageFile, Record, RunOptions, Snapshot, TypeCatalog};
use forage_http::{Engine, LiveTransport, PriorRecords, ReplayTransport};
use indexmap::IndexMap;

use crate::error::{DaemonError, RunError};
use crate::model::{Outcome, RunFlags, ScheduledRun, Trigger};
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

    /// Run a notebook composition: walk `stages` in order, feeding the
    /// emissions of stage N into stage N+1's input. Each stage name
    /// resolves to its current deployed version through the daemon's
    /// composition runtime — same path a hub-published composition
    /// recipe would take, but without a persistent `Run` row.
    ///
    /// `name` labels the synthetic composition recipe for diagnostics
    /// (`compose stage '<name>' …`). `inputs` flow into stage 1 only;
    /// downstream stages consume the upstream record stream.
    ///
    /// Always runs in ephemeral mode — the notebook is a playground;
    /// the snapshot is returned in-memory and never written to a daemon
    /// output store. `flags.ephemeral` is therefore irrelevant; the
    /// `replay` and `sample_limit` flags carry through to every stage.
    /// Persistence happens through "Publish notebook," which writes the
    /// composition as a `.forage` recipe and goes through the normal
    /// deploy + run path.
    pub async fn run_composition(
        self: &Arc<Self>,
        name: &str,
        stages: Vec<String>,
        inputs: IndexMap<String, EvalValue>,
        flags: RunFlags,
    ) -> Result<Snapshot, RunError> {
        if stages.is_empty() {
            return Err(RunError::Engine("composition has zero stages".into()));
        }
        let synthetic = synthesize_composition_file(name, &stages);

        let sink: Arc<dyn forage_http::ProgressSink> = Arc::new(ProgressForwarder {
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
        let catalog = TypeCatalog::default();
        // `version` is the outer-recipe version that inner-stage
        // failures cite for context. The synthetic composition has no
        // deployed version, so we pass 0 — every inner failure overrides
        // this with the inner stage's actual resolved version anyway.
        let snapshot = run_stage(
            self,
            &synthetic,
            &catalog,
            inputs,
            IndexMap::new(),
            PriorRecords::default(),
            sink,
            0,
            &engine_options,
            replay_captures.as_deref(),
        )
        .await
        .map_err(|f| RunError::Engine(f.message))?;
        Ok(snapshot)
    }
}

/// Build an in-memory `ForageFile` whose body is a composition over
/// `stages`. Carries an HTTP engine kind in the header — composition
/// recipes ignore the header's engine field at run time (each inner
/// stage carries its own), but the AST requires one.
fn synthesize_composition_file(name: &str, stages: &[String]) -> ForageFile {
    let stage_refs: Vec<RecipeRef> = stages
        .iter()
        .map(|s| RecipeRef {
            author: None,
            name: s.clone(),
            span: Span::default(),
        })
        .collect();
    ForageFile {
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
        // Empty source is safe here: composition bodies have no
        // pause sites, so the line-resolver never reads back from
        // this string to map a byte span onto a source line.
        source: std::sync::Arc::from(""),
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
    let recipe = match forage_core::parse(&deployed.source) {
        Ok(r) => r,
        Err(e) => {
            return Err(RunFailure {
                message: format!("parse deployed source: {e}"),
                diagnostics: 0,
                version: Some(version),
            });
        }
    };
    // The catalog was validated against the source at deploy time;
    // we trust it here without re-validating. A parser version drift
    // since deploy would surface above in `forage_core::parse`.
    let catalog: TypeCatalog = deployed.catalog.into();

    // Inputs come from the explicit `Run.inputs` field set via
    // `configure_run`. Recipes that declare `input` bindings must have
    // them configured on the row — there's no implicit filesystem
    // fallback.
    let inputs: IndexMap<String, EvalValue> = run
        .inputs
        .iter()
        .map(|(k, v)| (k.clone(), EvalValue::from(v)))
        .collect();
    let secrets = load_secrets(&recipe);

    let tables = derive_schema(&recipe, &catalog);
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
    let sink: Arc<dyn forage_http::ProgressSink> = Arc::new(ProgressForwarder {
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
    let snapshot_result = run_stage(
        daemon,
        &recipe,
        &catalog,
        inputs,
        secrets,
        PriorRecords::default(),
        sink.clone(),
        version,
        &engine_options,
        replay_captures.as_deref(),
    )
    .await;
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

/// Run one recipe to a snapshot. The dispatcher: scraping bodies go
/// to the matching engine; composition bodies walk the chain by
/// recursively invoking `run_stage` for each inner stage.
///
/// `prior` carries upstream records when this stage is being driven
/// by a composition; standalone calls (and composition stage 1) pass
/// `PriorRecords::default()`.
///
/// `version` is the version that resolved at the *outer* run's
/// deployment time; inner composition stages resolve their own
/// versions via `daemon.current_deployed`, but failures still
/// reference the outer version since that's what was scheduled.
///
/// `options` and `replay_captures` are decided once at the outermost
/// `execute` and apply uniformly to every stage in a composition
/// chain — sampling caps every stage's top-level for-loop, and replay
/// (when set) feeds the same fixture stream to every HTTP / browser
/// stage. Ephemeral output-store routing is realized in `execute`
/// against the outer `Run.output`, not per-stage.
#[allow(clippy::too_many_arguments)]
fn run_stage<'a>(
    daemon: &'a Arc<Daemon>,
    recipe: &'a ForageFile,
    catalog: &'a TypeCatalog,
    inputs: IndexMap<String, EvalValue>,
    secrets: IndexMap<String, String>,
    prior: PriorRecords,
    sink: Arc<dyn forage_http::ProgressSink>,
    version: u32,
    options: &'a RunOptions,
    replay_captures: Option<&'a [forage_replay::Capture]>,
) -> std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<forage_core::Snapshot, RunFailure>> + Send + 'a>,
> {
    Box::pin(async move {
        match &recipe.body {
            RecipeBody::Composition(comp) => {
                run_composition(
                    daemon,
                    comp,
                    inputs,
                    secrets,
                    sink,
                    version,
                    options,
                    replay_captures,
                )
                .await
            }
            RecipeBody::Scraping(_) | RecipeBody::Empty => {
                run_scraping(
                    daemon,
                    recipe,
                    catalog,
                    inputs,
                    secrets,
                    prior,
                    sink,
                    version,
                    options,
                    replay_captures,
                )
                .await
            }
        }
    })
}

/// Drive a scraping-body recipe against its engine, seeded with any
/// prior records the composition runtime threaded in. When
/// `replay_captures` is `Some`, the transport / browser-replay path is
/// taken instead of live — the same fixture stream is used at every
/// stage of a composition chain.
#[allow(clippy::too_many_arguments)]
async fn run_scraping(
    daemon: &Arc<Daemon>,
    recipe: &ForageFile,
    catalog: &TypeCatalog,
    inputs: IndexMap<String, EvalValue>,
    secrets: IndexMap<String, String>,
    prior: PriorRecords,
    sink: Arc<dyn forage_http::ProgressSink>,
    version: u32,
    options: &RunOptions,
    replay_captures: Option<&[forage_replay::Capture]>,
) -> Result<forage_core::Snapshot, RunFailure> {
    let Some(engine_kind) = recipe.engine_kind() else {
        return Err(RunFailure {
            message: "deployed source has no recipe header".to_string(),
            diagnostics: 0,
            version: Some(version),
        });
    };
    match engine_kind {
        EngineKind::Http => {
            let snapshot_result = match replay_captures {
                Some(captures) => {
                    let transport = ReplayTransport::new(captures.to_vec());
                    let engine = Engine::new(&transport).with_progress(sink.clone());
                    engine
                        .run_with_prior(recipe, catalog, inputs, secrets, options, prior)
                        .await
                }
                None => {
                    let transport = LiveTransport::new().map_err(|e| RunFailure {
                        message: format!("http transport: {e}"),
                        diagnostics: 0,
                        version: Some(version),
                    })?;
                    let engine = Engine::new(&transport).with_progress(sink.clone());
                    engine
                        .run_with_prior(recipe, catalog, inputs, secrets, options, prior)
                        .await
                }
            };
            snapshot_result.map_err(|e| RunFailure {
                message: format!("engine: {e}"),
                diagnostics: 0,
                version: Some(version),
            })
        }
        EngineKind::Browser => {
            if !prior.records.is_empty() {
                // Browser-engine downstream stages aren't supported yet —
                // the browser driver runs a real WebView and has no
                // record-seed entry point. Fail with a clear diagnostic
                // rather than silently dropping the upstream records.
                return Err(RunFailure {
                    message: format!(
                        "compose stage '{}' is browser-engine; browser engines can't yet receive prior records",
                        recipe.recipe_name().unwrap_or("<unknown>"),
                    ),
                    diagnostics: 0,
                    version: Some(version),
                });
            }
            match replay_captures {
                Some(captures) => forage_browser::run_browser_replay(
                    recipe, catalog, captures, inputs, secrets, options,
                )
                .map_err(|e| RunFailure {
                    message: format!("browser: {e}"),
                    diagnostics: 0,
                    version: Some(version),
                }),
                None => {
                    let driver = daemon
                        .browser_driver
                        .lock()
                        .expect("driver poisoned")
                        .clone()
                        .ok_or_else(|| RunFailure {
                            message:
                                "browser engine requires a LiveBrowserDriver — none registered"
                                    .into(),
                            diagnostics: 0,
                            version: Some(version),
                        })?;
                    driver
                        .run_live(recipe, catalog, inputs, secrets, sink.clone(), options)
                        .await
                        .map_err(|e| RunFailure {
                            message: format!("browser: {e}"),
                            diagnostics: 0,
                            version: Some(version),
                        })
                }
            }
        }
    }
}

/// Walk a composition chain: resolve each stage's deployed source,
/// run it, and thread its emissions as the next stage's prior. The
/// composition's own `inputs` flow into stage 1; downstream stages
/// receive only their typed `prior` plus any auto-passthrough inputs
/// the composition explicitly declares (today, none — the
/// composition's input is forwarded only to stage 1).
#[allow(clippy::too_many_arguments)]
async fn run_composition(
    daemon: &Arc<Daemon>,
    comp: &forage_core::ast::Composition,
    inputs: IndexMap<String, EvalValue>,
    secrets: IndexMap<String, String>,
    sink: Arc<dyn forage_http::ProgressSink>,
    version: u32,
    options: &RunOptions,
    replay_captures: Option<&[forage_replay::Capture]>,
) -> Result<forage_core::Snapshot, RunFailure> {
    let mut prior = PriorRecords::default();
    let mut stage_inputs = inputs;
    let mut last_snapshot: Option<forage_core::Snapshot> = None;
    // Hub-dep stages (`author.is_some()`) are rejected at validate time
    // (`ValidationCode::HubDepStageUnsupported`), so by the time a
    // deployed composition reaches this loop every stage is a bare
    // workspace-local name.
    for stage_ref in &comp.stages {
        let dv = match daemon.current_deployed(&stage_ref.name) {
            Ok(Some(dv)) => dv,
            Ok(None) => {
                return Err(RunFailure {
                    message: format!("compose stage '{}' has no deployed version", stage_ref.name,),
                    diagnostics: 0,
                    version: Some(version),
                });
            }
            Err(e) => {
                return Err(RunFailure {
                    message: format!(
                        "compose stage '{}' deployed-version lookup: {e}",
                        stage_ref.name,
                    ),
                    diagnostics: 0,
                    version: Some(version),
                });
            }
        };
        let deployed = daemon
            .load_deployed(&stage_ref.name, dv.version)
            .map_err(|e| RunFailure {
                message: format!(
                    "compose stage '{}' load v{}: {e}",
                    stage_ref.name, dv.version,
                ),
                diagnostics: 0,
                version: Some(version),
            })?;
        let inner_recipe = forage_core::parse(&deployed.source).map_err(|e| RunFailure {
            message: format!(
                "compose stage '{}' parse deployed source: {e}",
                stage_ref.name,
            ),
            diagnostics: 0,
            version: Some(version),
        })?;
        let inner_catalog: TypeCatalog = deployed.catalog.into();
        let inner_secrets = if secrets.is_empty() {
            load_secrets(&inner_recipe)
        } else {
            // Carry the composition's secrets through verbatim — every
            // inner stage that declares a `secret` of the same name sees
            // the supplied value. Inner-only secrets fall through to the
            // env-var convention.
            let mut merged = load_secrets(&inner_recipe);
            for (k, v) in &secrets {
                merged.entry(k.clone()).or_insert_with(|| v.clone());
            }
            merged
        };
        let snapshot = run_stage(
            daemon,
            &inner_recipe,
            &inner_catalog,
            stage_inputs.clone(),
            inner_secrets,
            prior.clone(),
            sink.clone(),
            version,
            options,
            replay_captures,
        )
        .await?;
        // Stage 2+: the recipe consumes the upstream stream and ignores
        // the composition's outer `inputs` (which are stage-1-only).
        stage_inputs.clear();
        prior = derive_prior(&snapshot);
        last_snapshot = Some(snapshot);
    }
    last_snapshot.ok_or_else(|| RunFailure {
        message: "composition has zero stages — validator should have rejected".into(),
        diagnostics: 0,
        version: Some(version),
    })
}

/// Build the `PriorRecords` carrier for the next stage from this
/// stage's snapshot. When the upstream emitted multiple types, the
/// downstream stage's input-slot lookup picks the matching one — but
/// the per-stage validator already pinned each output to a single
/// type, so in practice this is a one-type stream.
fn derive_prior(snap: &forage_core::Snapshot) -> PriorRecords {
    let mut type_name = String::new();
    let mut records: Vec<Record> = Vec::with_capacity(snap.records.len());
    for rec in &snap.records {
        if type_name.is_empty() {
            type_name = rec.type_name.clone();
        }
        records.push(rec.clone());
    }
    PriorRecords { records, type_name }
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
fn load_secrets(recipe: &ForageFile) -> IndexMap<String, String> {
    let mut out = IndexMap::new();
    for s in &recipe.secrets {
        let key = format!("FORAGE_SECRET_{}", s.to_uppercase());
        if let Ok(v) = std::env::var(&key) {
            out.insert(s.clone(), v);
        }
    }
    out
}
