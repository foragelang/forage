//! Tauri commands exposed to the frontend.

use async_trait::async_trait;
use indexmap::IndexMap;
use serde::Serialize;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::Notify;
use ts_rs::TS;

use forage_browser::run_browser_replay;
use forage_core::ast::EngineKind;
use forage_core::{EvalValue, LineMap, Snapshot, parse, validate};
use forage_http::{
    Debugger, Engine, IterationPause, LiveTransport, ProgressSink, ReplayTransport, ResumeAction,
    RunEvent, StepPause,
};
use forage_hub::{AuthStore, AuthTokens};

/// Tauri event name for streaming engine progress to the frontend.
pub const RUN_EVENT: &str = "forage:run-event";
/// Tauri event name for the engine telling the frontend it has paused
/// somewhere — at a `step` boundary or inside a `for`-loop iteration.
/// Payload is `PausePayload` (JSON) with a `kind` discriminator.
pub const DEBUG_PAUSED_EVENT: &str = "forage:debug-paused";

/// What the engine paused on. Wraps the two `forage-http` pause payloads
/// in a tagged union so the frontend can render either shape with one
/// event listener.
#[derive(Serialize, TS)]
#[ts(export)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PausePayload {
    Step(StepPause),
    Iteration(IterationPause),
}

use crate::browser_driver::{LiveRunOptions, run_live as run_browser_live};
use crate::daemon_browser::StudioLiveBrowserDriver;
use crate::state::{StudioState, WorkspaceSession};
use crate::workspace;
use forage_daemon::Daemon;

/// Pull the active workspace session out of state, or surface
/// "no workspace open" to the frontend. The session pairs the daemon
/// and workspace under one `ArcSwapOption`, so callers asking for one
/// always see the matching half — no half-installed mid-swap state.
fn require_session(state: &State<'_, StudioState>) -> Result<Arc<WorkspaceSession>, String> {
    state
        .session
        .load_full()
        .ok_or_else(|| "no workspace open".to_string())
}

/// Convenience wrapper for callers that only need the daemon.
fn require_daemon(state: &State<'_, StudioState>) -> Result<Arc<Daemon>, String> {
    require_session(state).map(|s| s.daemon.clone())
}

/// Convenience wrapper for callers that only need the workspace.
fn require_workspace(
    state: &State<'_, StudioState>,
) -> Result<Arc<forage_core::workspace::Workspace>, String> {
    require_session(state).map(|s| s.workspace.clone())
}

/// Wire Studio's browser driver and run-completed callback into a newly
/// constructed daemon, then start its scheduler. Shared by the
/// `open_workspace` command and any future bootstrapping path so the
/// daemon-attach sequence stays in one place.
pub fn install_daemon(app: &AppHandle, daemon: &Arc<Daemon>) {
    let handle = app.clone();
    daemon.set_browser_driver(Arc::new(StudioLiveBrowserDriver::new(handle.clone())));

    let cb_handle = handle.clone();
    daemon.on_run_completed(Box::new(move |sr| {
        if let Err(e) = cb_handle.emit("forage:daemon-run-completed", sr) {
            tracing::warn!(error = %e, "emit daemon-run-completed failed");
        }
    }));

    daemon.start_scheduler();
}

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct ValidationOutcome {
    pub ok: bool,
    pub diagnostics: Vec<Diagnostic>,
}

/// Structural outline of a recipe — currently just step locations,
/// enough for Studio to anchor breakpoint glyphs and the "reveal paused
/// step" jump without re-implementing a parser in TypeScript. Extend
/// (types, emits, for-loops) as the UI needs them.
#[derive(Serialize, Default, TS)]
#[ts(export)]
pub struct RecipeOutline {
    pub steps: Vec<StepLocation>,
}

#[derive(Serialize, TS)]
#[ts(export)]
pub struct StepLocation {
    pub name: String,
    /// 0-based line of the step declaration's start.
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
}

/// One validation issue with a precise source location. Maps onto
/// Monaco's `IMarkerData` shape on the frontend so squigglies land
/// under the offending token instead of at line 1.
#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct Diagnostic {
    pub severity: &'static str,
    pub code: String,
    pub message: String,
    /// 0-based line/column for the start of the span.
    pub start_line: u32,
    pub start_col: u32,
    /// 0-based line/column for the end of the span (exclusive).
    pub end_line: u32,
    pub end_col: u32,
}

#[tauri::command]
pub fn studio_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Validate a source buffer without touching disk. Studio fires this on
/// every keystroke (debounced) to keep diagnostics live. `save_file`
/// still re-validates after writing so editor markers remain accurate
/// even if the user only saves without typing.
#[tauri::command]
pub fn validate_recipe(source: String) -> ValidationOutcome {
    validate_source(&source)
}

/// Infer the recipe's progress unit (the deepest emit-bearing for-
/// loop scope) from current on-disk source. Frontend caches this per
/// recipe name and uses it to scope the live-run / scheduled-run
/// progress bar to a single type instead of summing all emits.
#[tauri::command]
pub fn recipe_progress_unit(
    state: State<'_, StudioState>,
    name: String,
) -> Result<Option<forage_core::ProgressUnit>, String> {
    let ws = require_workspace(&state)?;
    let source = workspace::read_source(&ws, &name)?;
    let recipe = forage_core::parse(&source).map_err(|e| format!("{e}"))?;
    Ok(forage_core::infer_progress_unit(&recipe))
}

#[tauri::command]
pub fn create_recipe(state: State<'_, StudioState>) -> Result<String, String> {
    let ws = require_workspace(&state)?;
    workspace::create_recipe(&ws.root, None).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_recipe(state: State<'_, StudioState>, name: String) -> Result<(), String> {
    let ws = require_workspace(&state)?;
    workspace::delete_recipe(&ws, &name).map_err(|e| e.to_string())
}

/// Pop up a native context menu (NSMenu on macOS, etc.) at the cursor
/// location with a "Delete recipe…" item. Selection flows back through
/// the global on_menu_event handler, which emits `menu:recipe_delete`
/// with the recipe name as payload.
#[tauri::command]
pub fn show_recipe_context_menu(
    app: AppHandle,
    window: tauri::WebviewWindow,
    name: String,
) -> Result<(), String> {
    let id = format!("recipe_delete:{name}");
    let delete_item = tauri::menu::MenuItemBuilder::with_id(&id, "Delete Recipe…")
        .build(&app)
        .map_err(|e| e.to_string())?;
    let menu = tauri::menu::MenuBuilder::new(&app)
        .item(&delete_item)
        .build()
        .map_err(|e| e.to_string())?;
    // `popup_menu` (no `_at`) routes through muda's `position: None`
    // branch, which reads the cursor directly via
    // `NSEvent::mouseLocation()` and calls
    // `popUpMenuPositioningItem(nil, mouse_location, inView: nil)` in
    // screen coords. See muda f3e4baa
    // src/platform_impl/macos/mod.rs:1204-1211.
    window.popup_menu(&menu).map_err(|e| e.to_string())?;
    // The menu must outlive popup_menu on macOS — muda's NSMenu wrapper
    // doesn't internally retain the Rust handle, so dropping it the
    // instant this function returns means the click event fires into a
    // freed receiver. Stash it on the StudioState until the next popup
    // replaces it.
    *app.state::<crate::state::StudioState>()
        .last_context_menu
        .lock()
        .expect("context menu mutex") = Some(menu);
    Ok(())
}

#[derive(Serialize, TS)]
#[ts(export)]
pub struct RunOutcome {
    pub ok: bool,
    pub snapshot: Option<Snapshot>,
    pub error: Option<String>,
    /// Non-fatal post-run daemon bookkeeping failure. The engine
    /// succeeded but the Run row didn't make it into the daemon
    /// sidebar — the UI surfaces this as a soft banner so the user
    /// isn't left wondering why their recipe didn't show up.
    pub daemon_warning: Option<String>,
}

/// Bridges the engine's `ProgressSink` to a Tauri global event so the
/// frontend gets live updates. Standard `emit()` instead of a command-scoped
/// `Channel<T>` because the latter had observed delivery problems in 2.8
/// where events would not surface despite the run progressing; the global
/// event bus is well-trodden and survives the round-trip cleanly.
///
/// Events are batched before crossing the Tauri channel: the engine can
/// emit thousands of records per second in tight bursts (e.g. a paginated
/// products endpoint near the end of a run), and per-record `app.emit`
/// saturates the webview's IPC channel — the JS side falls behind on
/// `RUN_EVENT` delivery, which also blocks the `run_recipe` invoke
/// response from reaching JS, so the UI gets stuck in "running" state
/// even though the engine has completed. The drainer task aggregates
/// events and emits them as `Vec<RunEvent>` every 50ms or 256 events,
/// keeping the channel responsive and the invoke promise unblocked.
struct EmitterSink {
    tx: tokio::sync::mpsc::UnboundedSender<RunEvent>,
}

impl ProgressSink for EmitterSink {
    fn emit(&self, event: RunEvent) {
        // Send failures are non-fatal (drainer dropped): the run
        // continues even if the UI can't hear it anymore.
        let _ = self.tx.send(event);
    }
}

/// Spawn a drainer task that batches incoming `RunEvent`s and flushes
/// them as `Vec<RunEvent>` on `RUN_EVENT`. Returns the sender side and
/// the task join handle; callers should drop the sink (closing the
/// channel) and await the handle so the final partial batch flushes
/// before `run_recipe` returns.
fn spawn_event_drainer(
    app: AppHandle,
) -> (
    tokio::sync::mpsc::UnboundedSender<RunEvent>,
    tokio::task::JoinHandle<()>,
) {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<RunEvent>();
    let handle = tokio::spawn(async move {
        const MAX_BATCH: usize = 256;
        const FLUSH_INTERVAL: std::time::Duration = std::time::Duration::from_millis(50);
        let mut buf: Vec<RunEvent> = Vec::with_capacity(MAX_BATCH);
        loop {
            // Wait for the first event (or channel close). Without a
            // first-event wait we'd spin every 50ms forever even when
            // idle.
            let first = match rx.recv().await {
                Some(ev) => ev,
                None => break,
            };
            buf.push(first);
            // Drain anything queued *now* without yielding back to the
            // scheduler — coalesces tight engine bursts.
            while let Ok(ev) = rx.try_recv() {
                buf.push(ev);
                if buf.len() >= MAX_BATCH {
                    break;
                }
            }
            // If we still have headroom, wait briefly for more events
            // to ride along. Caps total batch latency at FLUSH_INTERVAL.
            if buf.len() < MAX_BATCH {
                let _ = tokio::time::timeout(FLUSH_INTERVAL, async {
                    while let Some(ev) = rx.recv().await {
                        buf.push(ev);
                        if buf.len() >= MAX_BATCH {
                            return;
                        }
                    }
                })
                .await;
            }
            let batch = std::mem::take(&mut buf);
            let _ = app.emit(RUN_EVENT, &batch);
            buf.reserve(MAX_BATCH);
        }
        // Channel closed; flush any remaining (unlikely — final flush
        // happens at end of the loop above).
        if !buf.is_empty() {
            let _ = app.emit(RUN_EVENT, &buf);
        }
    });
    (tx, handle)
}

/// Bridges the engine's `Debugger` trait to a Tauri event + a oneshot.
///
/// The engine calls `before_step` for *every* step. We fast-path to
/// `Continue` unless the step is on a user-set breakpoint or the user just
/// clicked Step Over from a paused state. When we do pause: install a
/// fresh sender on the shared `DebugSession`, emit `forage:debug-paused`,
/// then await the receiver. The `debug_resume` command pulls the sender
/// back out and wakes us with the chosen action.
///
/// If the receiver drops (e.g. window closed, run cancelled), we default to
/// `Stop` so a stranded engine task doesn't hang on an unresumable pause.
struct StudioDebugger {
    app: AppHandle,
    session: Arc<crate::state::DebugSession>,
}

#[async_trait]
impl Debugger for StudioDebugger {
    async fn before_step(&self, pause: StepPause) -> ResumeAction {
        let state = self.app.state::<crate::state::StudioState>();
        // Hot path: read the breakpoint set without locking. ArcSwap
        // gives a refcounted snapshot in nanoseconds; the comparison
        // against the step name is the dominant cost.
        let on_breakpoint = state.breakpoints.load().contains(&pause.step);
        let step_over = self
            .session
            .step_over_pending
            .swap(false, std::sync::atomic::Ordering::SeqCst);
        if !on_breakpoint && !step_over {
            return ResumeAction::Continue;
        }
        self.wait(PausePayload::Step(pause)).await
    }

    async fn before_iteration(&self, pause: IterationPause) -> ResumeAction {
        let pause_iterations = self
            .session
            .pause_iterations
            .load(std::sync::atomic::Ordering::SeqCst);
        let step_over = self
            .session
            .step_over_pending
            .swap(false, std::sync::atomic::Ordering::SeqCst);
        if !pause_iterations && !step_over {
            return ResumeAction::Continue;
        }
        self.wait(PausePayload::Iteration(pause)).await
    }
}

impl StudioDebugger {
    /// Park the engine task on a fresh oneshot, emit the pause payload to
    /// the frontend, and await the user's resume action. Shared by both
    /// the step and iteration pause sites.
    async fn wait(&self, payload: PausePayload) -> ResumeAction {
        let (tx, rx) = tokio::sync::oneshot::channel();
        // The Mutex is the right primitive here — see DebugSession docs:
        // we need atomic take-and-fire on the resume path so two
        // concurrent debug_resume callers can't both grab the sender.
        *self
            .session
            .pending
            .lock()
            .expect("debug session pending sender") = Some(tx);
        let _ = self.app.emit(DEBUG_PAUSED_EVENT, &payload);
        rx.await.unwrap_or(ResumeAction::Stop)
    }
}

#[tauri::command]
pub async fn run_recipe(
    app: AppHandle,
    state: State<'_, crate::state::StudioState>,
    name: String,
    replay: bool,
) -> Result<RunOutcome, String> {
    tracing::info!(name = %name, replay, "run_recipe");
    let ws = require_workspace(&state)?;
    let daemon = require_daemon(&state)?;
    let source = workspace::read_source(&ws, &name)?;
    let recipe = match parse(&source) {
        Ok(r) => r,
        Err(e) => {
            return Ok(RunOutcome {
                ok: false,
                snapshot: None,
                error: Some(format!("{e}")),
                daemon_warning: None,
            });
        }
    };
    let catalog = match build_catalog(&ws.root, &recipe) {
        Ok(c) => c,
        Err(e) => {
            return Ok(RunOutcome {
                ok: false,
                snapshot: None,
                error: Some(format!("{e}")),
                daemon_warning: None,
            });
        }
    };
    let report = validate(&recipe, &catalog);
    if report.has_errors() {
        let msgs: Vec<String> = report.errors().map(|e| e.message.clone()).collect();
        return Ok(RunOutcome {
            ok: false,
            snapshot: None,
            error: Some(msgs.join("; ")),
            daemon_warning: None,
        });
    }
    // Inputs ride on the daemon's `Run.inputs`. A user with a recipe
    // that takes inputs sets them via `configure_run` once and they
    // apply to every subsequent fire — scheduler tick or Studio
    // Run-from-editor. Recipes without a configured `Run` (or with an
    // empty inputs map) run with no inputs; the engine surfaces a
    // "missing input" error if the recipe declared any.
    let raw_inputs = match daemon.get_run_by_name(&name).map_err(|e| e.to_string())? {
        Some(run) => run.inputs,
        None => IndexMap::new(),
    };
    let mut inputs: IndexMap<String, EvalValue> = IndexMap::new();
    for (k, v) in raw_inputs {
        inputs.insert(k, EvalValue::from(&v));
    }
    let secrets = workspace::read_secrets_from_env(&recipe);
    // Replay reads the recipe-name-keyed JSONL stream Phase 5 introduced
    // (`<root>/_fixtures/<recipe>.jsonl`). A live edit since workspace
    // load may have stripped the header — fall back to empty captures
    // in that case; the validator above will already have flagged the
    // header-less state.
    let captures = if replay {
        match recipe.recipe_name() {
            Some(header) => workspace::read_captures(&ws.root, header),
            None => Vec::new(),
        }
    } else {
        Vec::new()
    };

    let (tx, drainer_handle) = spawn_event_drainer(app.clone());
    let sink: Arc<dyn ProgressSink> = Arc::new(EmitterSink { tx });

    // Install a cancellation handle so `cancel_run` can interrupt this
    // run. Replaces any previous handle — Studio only runs one recipe at
    // a time, so any leftover from a prior aborted run is stale.
    let cancel = Arc::new(Notify::new());
    state.run_cancel.store(Some(cancel.clone()));

    // Install a debugger for every HTTP run — breakpoint hits drive
    // pauses, so the debugger has to be present even if no breakpoints
    // are set yet (the user might toggle one mid-run). Browser-engine
    // runs go through a separate driver that the v1 debugger doesn't
    // hook into.
    let engine_kind = recipe
        .engine_kind()
        .ok_or_else(|| "recipe source has no recipe header".to_string())?;
    let debugger: Option<Arc<dyn Debugger>> = if engine_kind == EngineKind::Http {
        let session = Arc::new(crate::state::DebugSession::default());
        state.debug_session.store(Some(session.clone()));
        Some(Arc::new(StudioDebugger {
            app: app.clone(),
            session,
        }))
    } else {
        None
    };

    let snapshot: Result<Snapshot, String> = match (engine_kind, replay) {
        (EngineKind::Http, true) => {
            let transport = ReplayTransport::new(captures);
            let mut engine = Engine::new(&transport).with_progress(Arc::clone(&sink));
            if let Some(d) = debugger.clone() {
                engine = engine.with_debugger(d);
            }
            tokio::select! {
                biased;
                _ = cancel.notified() => Err("cancelled".into()),
                r = engine.run(&recipe, &catalog, inputs, secrets) => r.map_err(|e| format!("{e}")),
            }
        }
        (EngineKind::Http, false) => {
            let transport = LiveTransport::new().map_err(|e| format!("{e}"))?;
            let mut engine = Engine::new(&transport).with_progress(Arc::clone(&sink));
            if let Some(d) = debugger.clone() {
                engine = engine.with_debugger(d);
            }
            tokio::select! {
                biased;
                _ = cancel.notified() => Err("cancelled".into()),
                r = engine.run(&recipe, &catalog, inputs, secrets) => r.map_err(|e| format!("{e}")),
            }
        }
        (EngineKind::Browser, true) => {
            run_browser_replay(&recipe, &catalog, &captures, inputs, secrets)
                .map_err(|e| format!("{e}"))
        }
        (EngineKind::Browser, false) => {
            // Open a Tauri WebviewWindow + inject the shim; collect
            // captures; route through the replay engine.
            run_browser_live(
                &app,
                &recipe,
                &catalog,
                inputs,
                secrets,
                LiveRunOptions::default(),
            )
            .await
        }
    };

    // Clear the cancellation handle so a stale notify can't fire on the
    // next run, and tear down the debug session so the resume path
    // can't wake into a finished engine. Dropping the session's Arc
    // drops any unfilled oneshot sender along with it.
    state.run_cancel.store(None);
    state.debug_session.store(None);

    // Drop the sink (closing the drainer's channel) and await its
    // final flush before returning. Without this, run_recipe could
    // return its Snapshot to JS while the drainer still holds queued
    // events — the UI would see the completion *before* the final
    // emits, producing a stuck-at-N% progress bar even on a clean
    // run. The await is bounded by FLUSH_INTERVAL since the drainer
    // exits as soon as the channel closes and `buf` is flushed.
    drop(sink);
    let _ = drainer_handle.await;

    match snapshot {
        Ok(mut s) => {
            // The engine evaluates expectations without source access,
            // so its diagnostics carry `line: None`. Re-evaluate here
            // with a `LineMap` so the UI can render a `recipe:L` jump
            // badge on every unmet-expect card.
            let line_map = LineMap::new(&source);
            s.evaluate_expectations(&recipe.expectations, Some(&line_map));
            // Make sure the recipe shows up in the daemon's Runs
            // sidebar after its first dev-run. `ensure_run` is
            // idempotent — repeated dev-runs of the same recipe don't
            // create new rows. A failure here doesn't sink the dev-run
            // (the engine succeeded), but it does ride back on the
            // outcome as a soft warning so the UI can banner it. A
            // silent log line was the old failure mode — the user
            // would see no Run row and have no idea why.
            let daemon_warning = match recipe.recipe_name() {
                Some(header) => match daemon.ensure_run(header) {
                    Ok(_) => None,
                    Err(e) => {
                        tracing::warn!(
                            recipe_name = %header,
                            error = %e,
                            "ensure_run after dev-run failed",
                        );
                        Some(format!("daemon bookkeeping failed: {e}"))
                    }
                },
                None => {
                    tracing::warn!(
                        name = %name,
                        "ensure_run skipped: dev-run target has no recipe header",
                    );
                    Some(
                        "daemon bookkeeping skipped: file has no recipe header"
                            .to_string(),
                    )
                }
            };
            Ok(RunOutcome {
                ok: true,
                snapshot: Some(s),
                error: None,
                daemon_warning,
            })
        }
        Err(e) => Ok(RunOutcome {
            ok: false,
            snapshot: None,
            error: Some(e),
            daemon_warning: None,
        }),
    }
}

/// Resume a paused debug step. `action` is "continue", "step_over", or
/// "stop". No-op when no run is in flight or no pause is pending — the UI
/// can fire and forget without coordinating against state itself.
///
/// "step_over" sets a one-shot flag on the session that forces the *next*
/// step's `before_step` to pause regardless of whether it's on a
/// breakpoint. From the engine's perspective both Continue and StepOver
/// just mean "resume"; the difference lives entirely on the host.
#[tauri::command]
pub fn debug_resume(
    state: State<'_, crate::state::StudioState>,
    action: String,
) -> Result<(), String> {
    let Some(session) = state.debug_session.load_full() else {
        return Ok(());
    };

    let resume = match action.as_str() {
        "continue" => ResumeAction::Continue,
        "step_over" => {
            session
                .step_over_pending
                .store(true, std::sync::atomic::Ordering::SeqCst);
            ResumeAction::Continue
        }
        "stop" => ResumeAction::Stop,
        other => return Err(format!("unknown debug action: {other}")),
    };

    let pending = session
        .pending
        .lock()
        .expect("debug session pending sender")
        .take();
    if let Some(tx) = pending {
        // send() fails only when the receiver has been dropped (run was
        // cancelled). Either way, nothing else to do.
        let _ = tx.send(resume);
    }
    Ok(())
}

/// Replace the current in-memory breakpoint set. Step names not present
/// in the recipe are harmless — the engine simply never reaches them.
///
/// Per-recipe persistence is handled by `set_recipe_breakpoints` /
/// `load_recipe_breakpoints` below. The frontend pushes via *this*
/// command on recipe switch so the engine's hot-path read sees the
/// new recipe's set, then persists the user's edits through the
/// recipe-scoped commands.
#[tauri::command]
pub fn set_breakpoints(
    state: State<'_, crate::state::StudioState>,
    steps: Vec<String>,
) -> Result<(), String> {
    state
        .breakpoints
        .store(Arc::new(steps.into_iter().collect()));
    Ok(())
}

/// Persist a recipe's breakpoint set to the workspace sidecar and push it
/// to the in-memory cache the engine reads on pause. Empty set deletes
/// the recipe's entry so the sidecar doesn't grow stale.
#[tauri::command]
pub fn set_recipe_breakpoints(
    state: State<'_, crate::state::StudioState>,
    name: String,
    steps: Vec<String>,
) -> Result<(), String> {
    let ws = require_workspace(&state)?;
    let mut all = workspace::read_breakpoints(&ws.root).map_err(|e| e.to_string())?;
    if steps.is_empty() {
        all.remove(&name);
    } else {
        all.insert(name, steps.clone());
    }
    workspace::write_breakpoints(&ws.root, &all).map_err(|e| e.to_string())?;
    state
        .breakpoints
        .store(Arc::new(steps.into_iter().collect()));
    Ok(())
}

/// Load the persisted breakpoint set for one recipe. Returns an empty
/// vec when the recipe has no entry — the absence of breakpoints is
/// the default. A malformed sidecar surfaces as an error so the user
/// sees the parse failure instead of silently losing every breakpoint.
#[tauri::command]
pub fn load_recipe_breakpoints(
    state: State<'_, crate::state::StudioState>,
    name: String,
) -> Result<Vec<String>, String> {
    let ws = require_workspace(&state)?;
    let mut map = workspace::read_breakpoints(&ws.root).map_err(|e| e.to_string())?;
    Ok(map.remove(&name).unwrap_or_default())
}

/// Toggle "pause inside every `for`-loop iteration" for the in-flight
/// run. No-op when no run is active. Per-run state — resets to false
/// every time a fresh run starts.
#[tauri::command]
pub fn set_pause_iterations(
    state: State<'_, crate::state::StudioState>,
    enabled: bool,
) -> Result<(), String> {
    if let Some(session) = state.debug_session.load_full() {
        session
            .pause_iterations
            .store(enabled, std::sync::atomic::Ordering::SeqCst);
    }
    Ok(())
}

/// Cancel the currently-running recipe (if any). Idempotent — calling when
/// nothing is running is a no-op. Wakes the `tokio::select!` in
/// `run_recipe`, which drops the engine future and any in-flight reqwest
/// call.
#[tauri::command]
pub fn cancel_run(state: State<'_, crate::state::StudioState>) -> Result<(), String> {
    if let Some(n) = state.run_cancel.load_full() {
        n.notify_one();
    }
    Ok(())
}

/// Publish the recipe named `name` to the hub under `@author/<name>`.
/// The atomic artifact (recipe + workspace shared decls + fixtures +
/// snapshot + base_version) is assembled by `forage-hub`'s shared
/// operations module; this command thin-wraps it for the UI.
///
/// `author` is passed in rather than derived from the manifest because
/// Studio exposes a publish-as-other-author flow for forks (the Tauri
/// command stays stateless w.r.t. manifest drift). `description`,
/// `category`, `tags` ride from the manifest or the publish dialog.
///
/// The hub-side publish slug equals the recipe header name. The
/// workspace's `_fixtures/<name>.jsonl` / `_snapshots/<name>.json` /
/// `.forage/sync/<name>.json` files all key on the same string.
///
/// Errors land as typed [`hub_sync::PublishError`] values so the UI
/// can render the stale-base banner with a "view diff" link instead
/// of dumping a stringified server message into a toast.
#[tauri::command]
pub async fn publish_recipe(
    state: State<'_, crate::state::StudioState>,
    author: String,
    name: String,
    hub_url: String,
    description: String,
    category: String,
    tags: Vec<String>,
) -> Result<crate::hub_sync::PublishOutcome, crate::hub_sync::PublishError> {
    let ws = require_workspace(&state).map_err(|e| crate::hub_sync::PublishError::Other {
        message: e,
    })?;
    // Pre-publish validation: parse + validate the recipe locally so
    // the server doesn't receive a malformed recipe. A broken recipe
    // surfaces as `Other` here, not `StaleBase` — the user fixes the
    // recipe before retrying.
    let source = workspace::read_source(&ws, &name).map_err(|e| {
        crate::hub_sync::PublishError::Other { message: e }
    })?;
    let recipe =
        parse(&source).map_err(|e| crate::hub_sync::PublishError::Other {
            message: format!("parse: {e}"),
        })?;
    let catalog = build_catalog(&ws.root, &recipe).map_err(|e| {
        crate::hub_sync::PublishError::Other {
            message: format!("catalog: {e}"),
        }
    })?;
    if validate(&recipe, &catalog).has_errors() {
        return Err(crate::hub_sync::PublishError::Other {
            message: "recipe failed validation; fix errors before publishing".into(),
        });
    }
    crate::hub_sync::run_publish(
        &ws,
        &hub_url,
        &author,
        &name,
        description,
        category,
        tags,
    )
    .await
}

/// Pull `(author, slug, version?)` from the hub and materialize the
/// version under the active workspace. Mirrors `forage sync` from
/// the CLI.
#[tauri::command]
pub async fn sync_from_hub(
    state: State<'_, crate::state::StudioState>,
    author: String,
    slug: String,
    version: Option<u32>,
    hub_url: String,
) -> Result<crate::hub_sync::SyncOutcomeWire, crate::hub_sync::PublishError> {
    let ws = require_workspace(&state).map_err(|e| crate::hub_sync::PublishError::Other {
        message: e,
    })?;
    crate::hub_sync::run_sync(&ws.root, &hub_url, &author, &slug, version).await
}

/// Fork `(upstream_author, upstream_slug)` to `@me/<as>` and sync the
/// new fork into the active workspace. Mirrors `forage fork` from
/// the CLI.
#[tauri::command]
pub async fn fork_from_hub(
    state: State<'_, crate::state::StudioState>,
    upstream_author: String,
    upstream_slug: String,
    r#as: Option<String>,
    hub_url: String,
) -> Result<crate::hub_sync::SyncOutcomeWire, crate::hub_sync::PublishError> {
    let ws = require_workspace(&state).map_err(|e| crate::hub_sync::PublishError::Other {
        message: e,
    })?;
    crate::hub_sync::run_fork(&ws.root, &hub_url, &upstream_author, &upstream_slug, r#as).await
}

/// Dry-run preview of a `publish_recipe` call: assemble the artifact
/// off-disk and report what would be sent without POSTing. The hub
/// publish slug is the recipe header name (see `publish_recipe`).
#[tauri::command]
pub fn preview_publish(
    state: State<'_, crate::state::StudioState>,
    name: String,
    description: String,
    category: String,
    tags: Vec<String>,
) -> Result<crate::hub_sync::PublishPreview, crate::hub_sync::PublishError> {
    let ws = require_workspace(&state).map_err(|e| crate::hub_sync::PublishError::Other {
        message: e,
    })?;
    crate::hub_sync::preview_publish(&ws, &name, description, category, tags)
}

#[tauri::command]
pub fn auth_whoami(hub_url: String) -> Result<Option<String>, String> {
    let host = host_of(&hub_url);
    let store = AuthStore::new();
    let tokens = store.read(&host).map_err(|e| e.to_string())?;
    Ok(tokens.map(|t| t.login))
}

#[derive(Serialize, TS)]
#[ts(export)]
pub struct DeviceStartOut {
    pub device_code: String,
    pub user_code: String,
    pub verification_url: String,
    pub interval: u64,
    pub expires_in: u64,
}

#[tauri::command]
pub async fn auth_start_device_flow(hub_url: String) -> Result<DeviceStartOut, String> {
    let s = forage_hub::device::start_device(&hub_url)
        .await
        .map_err(|e| e.to_string())?;
    Ok(DeviceStartOut {
        device_code: s.device_code,
        user_code: s.user_code,
        verification_url: s.verification_url,
        interval: s.interval,
        expires_in: s.expires_in,
    })
}

#[derive(Serialize, TS)]
#[ts(export)]
pub struct PollOutcome {
    pub status: String,
    pub login: Option<String>,
}

#[tauri::command]
pub async fn auth_poll_device(hub_url: String, device_code: String) -> Result<PollOutcome, String> {
    let r = forage_hub::device::poll_device(&hub_url, &device_code)
        .await
        .map_err(|e| e.to_string())?;
    if r.status == "ok" {
        if let (Some(access), Some(refresh), Some(user)) = (r.access_token, r.refresh_token, r.user)
        {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            let tokens = AuthTokens {
                access_token: access,
                refresh_token: refresh,
                login: user.login.clone(),
                hub_url,
                issued_at: now,
                expires_at: now + r.expires_in.unwrap_or(3600) as i64,
            };
            AuthStore::new().write(&tokens).map_err(|e| e.to_string())?;
            return Ok(PollOutcome {
                status: "ok".into(),
                login: Some(user.login),
            });
        }
    }
    Ok(PollOutcome {
        status: r.status,
        login: None,
    })
}

#[tauri::command]
pub fn auth_logout(hub_url: String) -> Result<(), String> {
    let host = host_of(&hub_url);
    AuthStore::new().delete(&host).map_err(|e| e.to_string())
}

/// Hover info at a given (line, col) in the source buffer. Powered by
/// `forage_lsp::intel::hover_at` — the same logic the LSP's textDocument/
/// hover handler runs. Returns `null` when the position isn't on a
/// recognizable identifier (transform / type / input / enum / secret /
/// step name).
#[tauri::command]
pub fn recipe_hover(source: String, line: u32, col: u32) -> Option<forage_lsp::intel::HoverInfo> {
    forage_lsp::intel::hover_at(&source, line, col)
}

/// Snapshot of the language's reserved word + transform inventory.
/// Studio fetches this once at startup so Monaco syntax highlighting,
/// completion, and any future linting all draw from the same canonical
/// lists in `forage-core`. No more hand-maintained TS arrays that
/// silently drift when the language gains a keyword.
#[derive(Serialize, TS)]
#[ts(export)]
pub struct LanguageDictionary {
    pub keywords: Vec<&'static str>,
    pub type_keywords: Vec<&'static str>,
    pub transforms: Vec<&'static str>,
}

#[tauri::command]
pub fn language_dictionary() -> LanguageDictionary {
    LanguageDictionary {
        keywords: forage_core::parse::KEYWORDS.to_vec(),
        type_keywords: forage_core::parse::TYPE_KEYWORDS.to_vec(),
        transforms: forage_core::validate::BUILTIN_TRANSFORMS.to_vec(),
    }
}

/// Parser-driven outline of the *current source buffer* (not the
/// last-saved file on disk). Used by Studio to anchor breakpoint glyphs
/// and reveal the paused step without a hand-rolled TS regex. Returns
/// an empty outline on parse failure — the editor falls back to "no
/// breakpoints visible until the source parses" rather than guessing
/// at half-baked syntax.
#[tauri::command]
pub fn recipe_outline(source: String) -> RecipeOutline {
    let Ok(recipe) = parse(&source) else {
        return RecipeOutline::default();
    };
    let line_map = LineMap::new(&source);
    let mut steps = Vec::new();
    collect_step_locations(&recipe.body, &line_map, &mut steps);
    RecipeOutline { steps }
}

fn collect_step_locations(
    body: &[forage_core::ast::Statement],
    line_map: &LineMap,
    out: &mut Vec<StepLocation>,
) {
    use forage_core::ast::Statement;
    for s in body {
        match s {
            Statement::Step(step) => {
                let r = line_map.range(step.span.clone());
                out.push(StepLocation {
                    name: step.name.clone(),
                    start_line: r.start.line,
                    start_col: r.start.character,
                    end_line: r.end.line,
                    end_col: r.end.character,
                });
            }
            Statement::ForLoop { body, .. } => {
                collect_step_locations(body, line_map, out);
            }
            Statement::Emit(_) => {}
        }
    }
}

/// Workspace-aware validation: parses the source, builds the
/// workspace catalog (via `build_catalog`), and attaches workspace
/// errors as document-level diagnostics. Used by `save_file`, which
/// always has a workspace context.
fn validate_source_in_workspace(workspace_root: &Path, source: &str) -> ValidationOutcome {
    let line_map = LineMap::new(source);
    let to_diag = |span: std::ops::Range<usize>,
                   severity: &'static str,
                   code: String,
                   message: String|
     -> Diagnostic {
        let r = line_map.range(span);
        Diagnostic {
            severity,
            code,
            message,
            start_line: r.start.line,
            start_col: r.start.character,
            end_line: r.end.line,
            end_col: r.end.character,
        }
    };
    match parse(source) {
        Ok(r) => {
            let catalog = match build_catalog(workspace_root, &r) {
                Ok(c) => c,
                Err(e) => {
                    let diag = to_diag(0..0, "error", "WorkspaceError".into(), format!("{e}"));
                    return ValidationOutcome {
                        ok: false,
                        diagnostics: vec![diag],
                    };
                }
            };
            let report = validate(&r, &catalog);
            let mut diagnostics: Vec<Diagnostic> = report
                .issues
                .into_iter()
                .map(|i| {
                    let sev = match i.severity {
                        forage_core::Severity::Error => "error",
                        forage_core::Severity::Warning => "warning",
                    };
                    to_diag(i.span, sev, format!("{:?}", i.code), i.message)
                })
                .collect();
            diagnostics.sort_by_key(|d| (d.start_line, d.start_col));
            let ok = !diagnostics.iter().any(|d| d.severity == "error");
            ValidationOutcome { ok, diagnostics }
        }
        Err(e) => {
            let (span, msg) = parse_error_span(&e);
            let diag = to_diag(span, "error", "ParseError".into(), msg);
            ValidationOutcome {
                ok: false,
                diagnostics: vec![diag],
            }
        }
    }
}

fn validate_source(source: &str) -> ValidationOutcome {
    let line_map = LineMap::new(source);
    let to_diag = |span: std::ops::Range<usize>,
                   severity: &'static str,
                   code: String,
                   message: String|
     -> Diagnostic {
        let r = line_map.range(span);
        Diagnostic {
            severity,
            code,
            message,
            start_line: r.start.line,
            start_col: r.start.character,
            end_line: r.end.line,
            end_col: r.end.character,
        }
    };
    match parse(source) {
        Ok(r) => {
            // No filesystem path available here (live-typed buffer);
            // validate against the recipe-local catalog only.
            let catalog = forage_core::TypeCatalog::from_file(&r);
            let report = validate(&r, &catalog);
            let mut diagnostics: Vec<Diagnostic> = report
                .issues
                .into_iter()
                .map(|i| {
                    let sev = match i.severity {
                        forage_core::Severity::Error => "error",
                        forage_core::Severity::Warning => "warning",
                    };
                    to_diag(i.span, sev, format!("{:?}", i.code), i.message)
                })
                .collect();
            // Stable order: file position first, then severity.
            diagnostics.sort_by_key(|d| (d.start_line, d.start_col));
            let ok = !diagnostics.iter().any(|d| d.severity == "error");
            ValidationOutcome { ok, diagnostics }
        }
        Err(e) => {
            // Parse errors carry their own span via ParseError variants.
            let (span, msg) = parse_error_span(&e);
            let diag = to_diag(span, "error", "ParseError".into(), msg);
            ValidationOutcome {
                ok: false,
                diagnostics: vec![diag],
            }
        }
    }
}

fn parse_error_span(e: &forage_core::parse::ParseError) -> (std::ops::Range<usize>, String) {
    use forage_core::parse::ParseError as PE;
    match e {
        PE::UnexpectedToken {
            span,
            expected,
            found,
        } => (
            span.clone(),
            format!("unexpected {found}, expected {expected}"),
        ),
        PE::UnexpectedEof { expected } => (
            0..0,
            format!("unexpected end of input, expected {expected}"),
        ),
        PE::Generic { span, message } => (span.clone(), message.clone()),
        PE::InvalidRegex { span, message } => {
            (span.clone(), format!("invalid regex: {message}"))
        }
        PE::InvalidRegexFlag { span, flag } => {
            (span.clone(), format!("unknown regex flag '{flag}'"))
        }
        PE::Lex(le) => (0..0, format!("{le}")),
    }
}

/// Type catalog for a recipe living under `workspace_root`. If the
/// root sits inside a workspace (ancestor `forage.toml`), the catalog
/// folds in workspace declarations files plus cached hub-dep
/// declarations. Otherwise lonely-recipe mode — the recipe's own
/// types only.
///
/// Re-discovers the workspace from disk on every call so a freshly
/// saved sibling (a new declarations file, a new `share` decl) is
/// visible without restarting Studio.
///
/// Workspace errors (duplicate types across declarations files, parse
/// failures in a sibling declarations file, etc.) are surfaced to the
/// caller instead of being silently swallowed — the user has to see
/// them to fix them.
fn build_catalog(
    workspace_root: &Path,
    recipe: &forage_core::ForageFile,
) -> Result<forage_core::TypeCatalog, forage_core::workspace::WorkspaceError> {
    if let Some(ws) = forage_core::workspace::discover(workspace_root) {
        return ws.catalog(recipe, |p| std::fs::read_to_string(p));
    }
    Ok(forage_core::TypeCatalog::from_file(recipe))
}

fn host_of(url: &str) -> String {
    let after_scheme = url.split("//").nth(1).unwrap_or(url);
    after_scheme
        .split('/')
        .next()
        .unwrap_or(after_scheme)
        .to_string()
}

// ---------------------------------------------------------------------
// Workspace + filesystem + daemon commands.
//
// Studio owns the on-disk workspace via the active `WorkspaceSession`.
// The daemon holds only deployed versions; draft state, file-tree
// listings, and catalog resolution against on-disk declarations all
// go through the Studio-side cache.
// ---------------------------------------------------------------------

use std::path::{Path, PathBuf};

use forage_daemon::{
    DaemonStatus, DeployedVersion, Run, RunConfig, ScheduledRun, validate_cron,
};

use crate::workspace::{
    DeployedState, DraftState, FileNode, RecentWorkspace, RecipeStatus, WorkspaceInfo,
    build_file_tree,
};

/// Snapshot of the loaded workspace: root path, manifest's `name`,
/// and `[deps]`. Returns `None` when no workspace is open — Studio
/// boots into that state and the frontend's top-level branch reads
/// this value to decide whether to render Welcome.
#[tauri::command]
pub fn current_workspace(state: State<'_, StudioState>) -> Option<WorkspaceInfo> {
    state
        .session
        .load_full()
        .map(|s| WorkspaceInfo::from_workspace(&s.workspace))
}

/// Recursive directory tree rooted at the workspace root, with each
/// file classified by [`crate::workspace::FileKind`]. Hidden entries
/// (`.forage/`, dotfiles) are skipped so the tree reflects the user's
/// authored content, not runtime state.
#[tauri::command]
pub fn list_workspace_files(state: State<'_, StudioState>) -> Result<FileNode, String> {
    let ws = require_workspace(&state)?;
    build_file_tree(&ws.root).map_err(|e| e.to_string())
}

/// Re-scan the workspace from disk and replace the cached snapshot.
/// Called from the frontend when a filesystem change (new recipe,
/// renamed declarations file, manifest edit) should be reflected
/// without restarting Studio.
#[tauri::command]
pub fn refresh_workspace(state: State<'_, StudioState>) -> Result<(), String> {
    let prior = require_session(&state)?;
    let fresh = forage_core::workspace::load(&prior.workspace.root).map_err(|e| e.to_string())?;
    state.session.store(Some(Arc::new(WorkspaceSession {
        daemon: prior.daemon.clone(),
        workspace: Arc::new(fresh),
    })));
    Ok(())
}

/// Read an arbitrary file under the workspace by absolute or
/// relative path. Rejects anything that — after `..` collapse and
/// symlink resolution — points outside the workspace root.
#[tauri::command]
pub fn load_file(state: State<'_, StudioState>, path: PathBuf) -> Result<String, String> {
    let target = resolve_existing_in_workspace(&state, &path)?;
    std::fs::read_to_string(&target).map_err(|e| e.to_string())
}

/// Write a file in the workspace and validate it if it's a recipe or
/// declarations file. Path-traversal and symlink-escape guards run
/// against the target's parent directory (which `create_dir_all`
/// just ensured exists), not the target itself — the target may be
/// a brand-new file.
#[tauri::command]
pub fn save_file(
    state: State<'_, StudioState>,
    path: PathBuf,
    source: String,
) -> Result<ValidationOutcome, String> {
    let target = resolve_new_in_workspace(&state, &path)?;
    let root = workspace_root_canonical(&state)?;
    std::fs::write(&target, &source).map_err(|e| e.to_string())?;
    let mut outcome = validate_path(&root, &target, &source);
    append_workspace_shared_diagnostics(&root, &target, &source, &mut outcome);
    Ok(outcome)
}

/// Run the cross-file `DuplicateSharedDeclaration` pass over every
/// `.forage` file under `root`, then append the issues that target the
/// just-saved file to its outcome. Re-loads the workspace from disk so
/// the slice reflects the buffer we just wrote, not the cached Studio
/// view.
fn append_workspace_shared_diagnostics(
    root: &Path,
    saved_path: &Path,
    source: &str,
    outcome: &mut ValidationOutcome,
) {
    let ws = match forage_core::workspace::load(root) {
        Ok(ws) => ws,
        Err(e) => {
            // The save itself succeeded; the cross-file pass is one
            // step on top. Surfacing the workspace load failure into
            // every saved-file outcome would be confusing — the
            // workspace-level error is a separate, latent issue. Log
            // it so it's not invisible.
            tracing::warn!(
                error = %e,
                root = %root.display(),
                "skipping cross-file shared-decl pass: workspace re-load failed"
            );
            return;
        }
    };
    let focal = match parse(source) {
        Ok(f) => f,
        // The focal source already failed to parse; `validate_path`
        // surfaced that parse error. Skipping the cross-file pass is
        // correct — no AST means no shared decls to compare.
        Err(_) => return,
    };
    let canonical_saved = saved_path
        .canonicalize()
        .unwrap_or_else(|_| saved_path.to_path_buf());

    let mut entries: Vec<(std::path::PathBuf, forage_core::ForageFile)> =
        Vec::with_capacity(ws.files.len() + 1);
    let mut focal_seen = false;
    for entry in &ws.files {
        let canonical = entry
            .path
            .canonicalize()
            .unwrap_or_else(|_| entry.path.clone());
        if canonical == canonical_saved {
            entries.push((canonical, focal.clone()));
            focal_seen = true;
            continue;
        }
        let src = match std::fs::read_to_string(&entry.path) {
            Ok(s) => s,
            Err(e) => {
                tracing::debug!(
                    error = %e,
                    path = %entry.path.display(),
                    "skipping sibling in cross-file shared-decl pass: read failed",
                );
                continue;
            }
        };
        let parsed = match parse(&src) {
            Ok(p) => p,
            Err(e) => {
                tracing::debug!(
                    error = %e,
                    path = %entry.path.display(),
                    "skipping sibling in cross-file shared-decl pass: parse failed",
                );
                continue;
            }
        };
        entries.push((canonical, parsed));
    }
    if !focal_seen {
        entries.push((canonical_saved.clone(), focal));
    }
    let refs: Vec<forage_core::validate::WorkspaceFileRef<'_>> = entries
        .iter()
        .map(|(p, f)| forage_core::validate::WorkspaceFileRef { path: p, file: f })
        .collect();
    let by_path = forage_core::validate::validate_workspace_shared(&refs);
    let Some(issues) = by_path.get(&canonical_saved) else {
        return;
    };
    let line_map = LineMap::new(source);
    for issue in issues {
        let r = line_map.range(issue.span.clone());
        let sev = match issue.severity {
            forage_core::Severity::Error => "error",
            forage_core::Severity::Warning => "warning",
        };
        outcome.diagnostics.push(Diagnostic {
            severity: sev,
            code: format!("{:?}", issue.code),
            message: issue.message.clone(),
            start_line: r.start.line,
            start_col: r.start.character,
            end_line: r.end.line,
            end_col: r.end.character,
        });
    }
    outcome.diagnostics.sort_by_key(|d| (d.start_line, d.start_col));
    outcome.ok = !outcome.diagnostics.iter().any(|d| d.severity == "error");
}

/// Resolve a path that must already exist inside the workspace.
/// Used by `load_file`. `canonicalize` follows symlinks, so a
/// `<workspace>/evil -> /etc/passwd` symlink is caught here
/// rather than being silently dereferenced by `read_to_string`.
fn resolve_existing_in_workspace(
    state: &State<'_, StudioState>,
    path: &Path,
) -> Result<PathBuf, String> {
    let ws = require_workspace(state)?;
    resolve_existing(&ws.root, path)
}

/// Resolve a path that may not exist yet (e.g. `save_file` creating
/// a new recipe). Creates the parent directory, then canonicalizes
/// it — that's enough to defeat symlink escapes, since the only
/// way `target` could land outside the root is via a symlinked
/// ancestor.
fn resolve_new_in_workspace(
    state: &State<'_, StudioState>,
    path: &Path,
) -> Result<PathBuf, String> {
    let ws = require_workspace(state)?;
    resolve_new(&ws.root, path)
}

fn workspace_root_canonical(state: &State<'_, StudioState>) -> Result<PathBuf, String> {
    let ws = require_workspace(state)?;
    canonicalize_root(&ws.root)
}

/// Inner helper for `resolve_existing_in_workspace` that takes the
/// workspace root explicitly. Factored out so tests don't need a
/// `State<'_, StudioState>` — they can call this directly.
fn resolve_existing(root: &Path, path: &Path) -> Result<PathBuf, String> {
    let candidate = join_against(root, path)?;
    let canonical = candidate
        .canonicalize()
        .map_err(|e| format!("cannot resolve {}: {e}", candidate.display()))?;
    let root_canonical = canonicalize_root(root)?;
    if !canonical.starts_with(&root_canonical) {
        return Err(format!(
            "path {} escapes workspace root {}",
            canonical.display(),
            root_canonical.display()
        ));
    }
    Ok(canonical)
}

/// Inner helper for `resolve_new_in_workspace`. Same factoring as
/// `resolve_existing`.
fn resolve_new(root: &Path, path: &Path) -> Result<PathBuf, String> {
    let candidate = join_against(root, path)?;
    let parent = candidate
        .parent()
        .ok_or_else(|| format!("path {} has no parent directory", candidate.display()))?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let parent_canonical = parent
        .canonicalize()
        .map_err(|e| format!("cannot resolve {}: {e}", parent.display()))?;
    let root_canonical = canonicalize_root(root)?;
    if !parent_canonical.starts_with(&root_canonical) {
        return Err(format!(
            "path {} escapes workspace root {}",
            parent_canonical.display(),
            root_canonical.display()
        ));
    }
    let file_name = candidate
        .file_name()
        .ok_or_else(|| format!("path {} has no file name", candidate.display()))?;
    Ok(parent_canonical.join(file_name))
}

fn join_against(root: &Path, path: &Path) -> Result<PathBuf, String> {
    if path.as_os_str().is_empty() {
        return Err("empty path".into());
    }
    Ok(if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    })
}

fn canonicalize_root(root: &Path) -> Result<PathBuf, String> {
    root.canonicalize()
        .map_err(|e| format!("cannot canonicalize workspace root {}: {e}", root.display()))
}

/// Validate a file saved under the workspace at `path` (canonical,
/// inside `root`). The decision tree:
///   * non-`.forage` — clean outcome (no diagnostics).
///   * `<root>/<name>.forage` — workspace-aware validation. A
///     `.forage` file may declare a recipe or just hold shared types;
///     both go through the same validator.
///   * `<root>/<slug>/recipe.forage` — the pre-Phase-10 legacy
///     shape. Studio refuses to act against unmigrated workspaces;
///     the diagnostic points the user at `forage migrate`.
///   * any other `.forage` location — unrecognized; surface as a
///     diagnostic so the UI doesn't silently treat sidecars as
///     declarations.
fn validate_path(root: &Path, path: &Path, source: &str) -> ValidationOutcome {
    let extension = path
        .extension()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    if extension != "forage" {
        return ValidationOutcome {
            ok: true,
            diagnostics: Vec::new(),
        };
    }
    if path.file_name().and_then(|s| s.to_str()).is_none() {
        return ValidationOutcome {
            ok: false,
            diagnostics: vec![Diagnostic {
                severity: "error",
                code: "InvalidPath".into(),
                message: format!("path {} has no file name", path.display()),
                start_line: 0,
                start_col: 0,
                end_line: 0,
                end_col: 0,
            }],
        };
    };

    let rel = path.strip_prefix(root).unwrap_or(path);
    let depth = rel.components().count();

    if depth == 1 {
        return validate_source_in_workspace(root, source);
    }

    let r = LineMap::new(source).range(0..0);
    if workspace::is_legacy_recipe_path(root, path) {
        return ValidationOutcome {
            ok: false,
            diagnostics: vec![Diagnostic {
                severity: "error",
                code: "UnmigratedWorkspace".into(),
                message: workspace::unmigrated_workspace_message(root),
                start_line: r.start.line,
                start_col: r.start.character,
                end_line: r.end.line,
                end_col: r.end.character,
            }],
        };
    }

    // Any other `.forage` location is a sidecar — neither at the
    // workspace root nor in a recognized layout. classify_file tags
    // it `Other`; validate_path agrees so the UI doesn't silently
    // treat sidecars as source.
    ValidationOutcome {
        ok: false,
        diagnostics: vec![Diagnostic {
            severity: "error",
            code: "UnrecognizedForageFile".into(),
            message: format!(
                "unrecognized .forage file location: {} — .forage files belong at the workspace root",
                path.display()
            ),
            start_line: r.start.line,
            start_col: r.start.character,
            end_line: r.end.line,
            end_col: r.end.character,
        }],
    }
}

// --- Daemon commands -------------------------------------------------

#[tauri::command]
pub fn daemon_status(state: State<'_, StudioState>) -> Result<DaemonStatus, String> {
    require_daemon(&state)?.status().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_runs(state: State<'_, StudioState>) -> Result<Vec<Run>, String> {
    require_daemon(&state)?.list_runs().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_run(state: State<'_, StudioState>, run_id: String) -> Result<Option<Run>, String> {
    require_daemon(&state)?
        .get_run(&run_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn configure_run(
    state: State<'_, StudioState>,
    name: String,
    cfg: RunConfig,
) -> Result<Run, String> {
    require_daemon(&state)?
        .configure_run(&name, cfg)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn remove_run(state: State<'_, StudioState>, run_id: String) -> Result<(), String> {
    require_daemon(&state)?
        .remove_run(&run_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn trigger_run(
    state: State<'_, StudioState>,
    run_id: String,
) -> Result<ScheduledRun, String> {
    let daemon = require_daemon(&state)?;
    daemon.trigger_run(&run_id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_scheduled_runs(
    state: State<'_, StudioState>,
    run_id: String,
    limit: u32,
    before: Option<i64>,
) -> Result<Vec<ScheduledRun>, String> {
    require_daemon(&state)?
        .list_scheduled_runs(&run_id, limit, before)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn load_run_records(
    state: State<'_, StudioState>,
    scheduled_run_id: String,
    type_name: String,
    limit: u32,
) -> Result<Vec<serde_json::Value>, String> {
    require_daemon(&state)?
        .load_records(&scheduled_run_id, &type_name, limit)
        .map_err(|e| e.to_string())
}

/// Validate a cron expression using the daemon's parser. The frontend
/// uses this as the gate for the schedule editor's Save button so the
/// client and server agree on what counts as valid syntax — `cronstrue`
/// (the client-side humanizer) accepts a wider grammar than the daemon's
/// 5-field parser, and a mismatch would let the user save a schedule
/// that the daemon then rejects at configure time.
#[tauri::command]
pub async fn validate_cron_expr(expr: String) -> Result<(), String> {
    validate_cron(&expr).map_err(|e| e.to_string())
}

/// Promote a draft recipe to a frozen deployed version. Reads the
/// on-disk source for `name`, resolves its catalog against the
/// Studio-side workspace (so workspace declarations and cached hub
/// deps are folded in), and hands the validated pair to the daemon
/// keyed on the recipe's header name. Returns the new
/// `DeployedVersion` row.
#[tauri::command]
pub fn deploy_recipe(
    state: State<'_, StudioState>,
    name: String,
) -> Result<DeployedVersion, String> {
    let ws = require_workspace(&state)?;
    let daemon = require_daemon(&state)?;
    // Source and catalog anchored on the same workspace handle so a
    // mid-deploy refresh can't make the source disagree with the
    // catalog it resolves against.
    let source = workspace::read_source(&ws, &name)?;
    let recipe = parse(&source).map_err(|e| format!("parse: {e}"))?;
    let recipe_name = recipe
        .recipe_name()
        .ok_or_else(|| format!("recipe {name:?} no longer has a header in its source"))?;
    let catalog = ws
        .catalog(&recipe, |p| std::fs::read_to_string(p))
        .map_err(|e| format!("catalog: {e}"))?;
    let wire = forage_core::workspace::SerializableCatalog::from(catalog);
    daemon
        .deploy(recipe_name, source, wire)
        .map_err(|e| e.to_string())
}

/// All deployed versions for one recipe, newest first. Returns an
/// empty vec when the recipe has never been deployed.
#[tauri::command]
pub fn list_deployed_versions(
    state: State<'_, StudioState>,
    name: String,
) -> Result<Vec<DeployedVersion>, String> {
    require_daemon(&state)?
        .deployed_versions(&name)
        .map_err(|e| e.to_string())
}

/// Per-recipe status surface: joins Studio's on-disk view of drafts
/// (valid, broken, missing) with the daemon's view of deployed
/// versions. Returns one entry per recipe known to either side,
/// keyed on the recipe's header name and ordered alphabetically.
#[tauri::command]
pub fn list_recipe_statuses(
    state: State<'_, StudioState>,
) -> Result<Vec<RecipeStatus>, String> {
    let ws = require_workspace(&state)?;
    let daemon = require_daemon(&state)?;
    build_recipe_statuses(&ws, &daemon)
}

/// Pure join of workspace drafts + daemon deployments, factored out
/// of the Tauri command so tests can drive both sides without
/// constructing a real `StudioState`.
fn build_recipe_statuses(
    ws: &forage_core::workspace::Workspace,
    daemon: &forage_daemon::Daemon,
) -> Result<Vec<RecipeStatus>, String> {
    use std::collections::BTreeMap;

    let mut by_name: BTreeMap<String, (DraftState, DeployedState)> = BTreeMap::new();

    // `DraftState.path` is workspace-relative so the JS side can join
    // it against `FileNode.path` (the file tree shape) without having
    // to thread the workspace root through every callsite.
    let rel = |p: &std::path::Path| -> PathBuf {
        p.strip_prefix(&ws.root)
            .map(std::path::Path::to_path_buf)
            .unwrap_or_else(|_| p.to_path_buf())
    };

    // Parsed recipes contribute their header name + a Valid draft
    // state. Broken files have no header name to key on (the parser
    // bailed), so they fall back to their file basename — the user
    // can still locate them in the file tree, and the daemon's view
    // never references a broken recipe (deploy requires a clean
    // parse).
    for recipe in ws.recipes() {
        by_name.insert(
            recipe.name().to_string(),
            (
                DraftState::Valid {
                    path: rel(recipe.path),
                },
                DeployedState::None,
            ),
        );
    }
    for broken in ws.broken() {
        let key = broken
            .path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| broken.path.display().to_string());
        by_name.insert(
            key,
            (
                DraftState::Broken {
                    path: rel(broken.path),
                    error: broken.error.to_string(),
                },
                DeployedState::None,
            ),
        );
    }

    let deployments = daemon.deployed_names().map_err(|e| e.to_string())?;
    for dv in deployments {
        let entry = by_name.entry(dv.recipe_name.clone()).or_insert((
            DraftState::Missing,
            DeployedState::None,
        ));
        entry.1 = DeployedState::Deployed {
            version: dv.version,
            deployed_at: dv.deployed_at,
        };
    }

    Ok(by_name
        .into_iter()
        .map(|(name, (draft, deployed))| RecipeStatus {
            name,
            draft,
            deployed,
        })
        .collect())
}

// ---------------------------------------------------------------------
// Workspace lifecycle: open / new / close, recents, menu wiring.
//
// All four commands serialize through `state.workspace_switch` so a
// concurrent ⌘O firing while a prior open is still resolving can't
// half-install a daemon. The mutex also makes the in-place
// "switch workspace" operation safe: we close the previous daemon and
// install the new one without ever exposing a state where neither (or
// both) are live to readers.
// ---------------------------------------------------------------------

/// Drop the current workspace (if any), close its daemon, then install
/// a fresh workspace + daemon for `root`. Updates the recents sidecar
/// and emits a `forage:workspace-opened` event for the frontend.
#[tauri::command]
pub async fn open_workspace(
    state: State<'_, StudioState>,
    app: AppHandle,
    path: PathBuf,
) -> Result<WorkspaceInfo, String> {
    let _guard = state.workspace_switch.lock().await;
    open_workspace_inner(&state, &app, path).await
}

/// Scaffold a new workspace at `path` (creates the directory if
/// missing, writes an empty `forage.toml`), then dispatch into
/// `open_workspace_inner` to install it. Refuses paths that already
/// hold a `forage.toml` — the user should pick Open in that case.
#[tauri::command]
pub async fn new_workspace(
    state: State<'_, StudioState>,
    app: AppHandle,
    path: PathBuf,
) -> Result<WorkspaceInfo, String> {
    let _guard = state.workspace_switch.lock().await;
    new_workspace_inner(&state, &app, path).await
}

/// Test-reachable core of `new_workspace`. The command body is just
/// `lock + new_workspace_inner` so tests can assert the manifest-already-
/// exists rejection through the same code path users hit, not via a
/// helper one layer down.
async fn new_workspace_inner(
    state: &StudioState,
    app: &AppHandle,
    path: PathBuf,
) -> Result<WorkspaceInfo, String> {
    scaffold_new_workspace(&path)?;
    open_workspace_inner(state, app, path).await
}

/// Scaffold a fresh workspace on disk so `open_workspace_inner` can
/// take over. Pulled out so the `AlreadyExists` rejection — the
/// `new_workspace` command's user-facing contract — is testable
/// without a Tauri runtime.
fn scaffold_new_workspace(path: &Path) -> Result<(), String> {
    workspace::write_empty_manifest(path).map_err(|e| e.to_string())
}

/// Close the active workspace. Idempotent: a second close on an
/// already-empty state returns `Ok(())` without panicking.
#[tauri::command]
pub async fn close_workspace(
    state: State<'_, StudioState>,
    app: AppHandle,
) -> Result<(), String> {
    let _guard = state.workspace_switch.lock().await;
    close_workspace_inner(&state, Some(&app));
    Ok(())
}

/// Recents list, filtered down to entries whose path still exists on
/// disk. The Welcome view fetches this through TanStack Query; the
/// switcher popover ignores it.
#[tauri::command]
pub fn list_recent_workspaces() -> Result<Vec<RecentWorkspace>, String> {
    Ok(workspace::read_recents())
}

/// The transactional core of opening a workspace. Caller must hold
/// `state.workspace_switch`. Closes any prior workspace+daemon first
/// so the swap is atomic from the frontend's perspective.
async fn open_workspace_inner(
    state: &StudioState,
    app: &AppHandle,
    path: PathBuf,
) -> Result<WorkspaceInfo, String> {
    // Close the previous workspace first. If the user is switching,
    // the old daemon's scheduler stops before the new one starts.
    close_workspace_inner(state, Some(app));

    validate_workspace_path(&path)?;

    let workspace = forage_core::workspace::load(&path)
        .map_err(|e| format!("load workspace at {}: {e}", path.display()))?;
    let daemon = forage_daemon::Daemon::open(path.clone())
        .map_err(|e| format!("open daemon at {}: {e}", path.display()))?;

    install_daemon(app, &daemon);

    // Cache before recording so the recents row reflects what the user
    // actually opened (manifest name + recipe count both come off the
    // loaded workspace).
    let info = WorkspaceInfo::from_workspace(&workspace);
    let display_name = workspace::derive_workspace_name(&workspace);
    let recipe_count = workspace.recipes().count() as u32;
    state.session.store(Some(Arc::new(WorkspaceSession {
        daemon,
        workspace: Arc::new(workspace),
    })));

    if let Err(e) = workspace::record_recent(&path, display_name, recipe_count) {
        tracing::warn!(error = %e, path = %path.display(), "record_recent failed");
    }

    set_close_workspace_enabled(state, true);
    let _ = app.emit("forage:workspace-opened", &info);
    Ok(info)
}

/// Path-side preconditions for `open_workspace`: the directory must
/// exist and contain a `forage.toml`. Factored out so tests can hit
/// the rejection branches without needing a `StudioState` or
/// `AppHandle`.
fn validate_workspace_path(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Err(format!("workspace path does not exist: {}", path.display()));
    }
    if !path.join("forage.toml").exists() {
        return Err(format!(
            "{} is not a workspace (missing forage.toml)",
            path.display()
        ));
    }
    Ok(())
}

/// The transactional core of closing the active workspace. Caller must
/// hold `state.workspace_switch`. Safe to call when no workspace is
/// open — drops to a no-op. `app` is `Option` so tests can drive the
/// state-mutation path without a Tauri runtime; production callers
/// always pass `Some(handle)` so the `forage:workspace-closed` event
/// fires and the menu item updates.
fn close_workspace_inner(state: &StudioState, app: Option<&AppHandle>) {
    // Tear down any in-flight run before the daemon shuts down — the
    // engine task would otherwise keep running against a closed
    // workspace.
    if let Some(n) = state.run_cancel.swap(None) {
        n.notify_one();
    }
    state.debug_session.store(None);

    let prior = state.session.swap(None);
    let was_open = prior.is_some();
    if let Some(session) = prior {
        // `Daemon::close` consumes `Arc<Self>` — clone out so the
        // session Arc itself can drop normally.
        session.daemon.clone().close();
    }

    if was_open {
        set_close_workspace_enabled(state, false);
        if let Some(app) = app {
            let _ = app.emit("forage:workspace-closed", ());
        }
    }
}

fn set_close_workspace_enabled(state: &StudioState, enabled: bool) {
    if let Some(item) = state
        .menu_close_workspace
        .lock()
        .expect("menu_close_workspace mutex")
        .as_ref()
    {
        if let Err(e) = item.set_enabled(enabled) {
            tracing::warn!(error = %e, "set_enabled on Close Workspace menu item failed");
        }
    }
}

#[cfg(test)]
mod tests {
    //! Studio-side smoke tests for the daemon wiring. The commands
    //! themselves take `State<'_, StudioState>` which can't be
    //! constructed without a running Tauri app — they're thin
    //! delegations to `Daemon`, so we exercise the same daemon API
    //! the command bodies call and assert the round-trips that
    //! Studio depends on.
    //!
    //! Coverage:
    //!   * `configure_run` → `list_runs` round-trip surfaces the new Run.
    //!   * `trigger_run` produces a `ScheduledRun` that
    //!     `list_scheduled_runs` returns.
    //!
    //! The browser-driver path is verified at the trait level only —
    //! a Tauri webview can't be opened from `cargo test`, so the
    //! daemon's `LiveBrowserDriver` slot is left empty; HTTP recipes
    //! cover the rest of the surface.
    use std::path::Path;

    use forage_daemon::{Cadence, Daemon, Outcome, RunConfig, Trigger};
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const RECIPE: &str = r#"recipe "items"
engine http

type Item {
    id: String
}

step list {
    method "GET"
    url    "https://example.test/items"
}

for $i in $list.items[*] {
    emit Item {
        id ← $i.id
    }
}
"#;

    fn write_workspace(root: &Path, name: &str, recipe_source: &str) {
        std::fs::create_dir_all(root).unwrap();
        std::fs::write(
            root.join("forage.toml"),
            "description = \"\"\ncategory = \"\"\ntags = []\n",
        )
        .unwrap();
        std::fs::write(root.join(format!("{name}.forage")), recipe_source).unwrap();
    }

    fn rewrite_url(path: &Path, url: &str) {
        let src = std::fs::read_to_string(path).unwrap();
        std::fs::write(path, src.replace("https://example.test/items", url)).unwrap();
    }

    fn deploy_from_disk(daemon: &Daemon, ws_root: &Path, name: &str) {
        let recipe_path = ws_root.join(format!("{name}.forage"));
        let source = std::fs::read_to_string(&recipe_path).unwrap();
        let recipe = forage_core::parse(&source).unwrap();
        let workspace = forage_core::workspace::load(ws_root).unwrap();
        let catalog = workspace
            .catalog(&recipe, |p| std::fs::read_to_string(p))
            .unwrap();
        let wire = forage_core::workspace::SerializableCatalog::from(catalog);
        daemon.deploy(name, source, wire).expect("deploy");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn configure_run_then_list_runs_round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        let ws_root = tmp.path().to_path_buf();
        let slug = "items";
        write_workspace(&ws_root, slug, RECIPE);

        let daemon = Daemon::open(ws_root.clone()).expect("open daemon");

        let cfg = RunConfig {
            cadence: Cadence::Manual,
            output: ws_root.join(".forage").join("data").join("items.sqlite"),
            enabled: true,
            inputs: indexmap::IndexMap::new(),
        };
        let created = daemon
            .configure_run(slug, cfg.clone())
            .expect("configure_run");

        let listed = daemon.list_runs().expect("list_runs");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, created.id);
        assert_eq!(listed[0].recipe_name, slug);
        assert!(listed[0].enabled);
        // Bare configure (no prior deploy) leaves the pointer
        // unset — pre-deploy scheduled fires record a clean
        // "recipe not deployed" failure instead of crashing.
        assert!(listed[0].deployed_version.is_none());

        // Repeated configure on the same slug is an update, not an
        // insert — list_runs should still return one row, and the id
        // should be stable.
        let updated_cfg = RunConfig {
            enabled: false,
            ..cfg
        };
        let updated = daemon
            .configure_run(slug, updated_cfg)
            .expect("configure_run update");
        assert_eq!(updated.id, created.id);
        let after = daemon.list_runs().expect("list_runs after update");
        assert_eq!(after.len(), 1);
        assert!(!after[0].enabled);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn trigger_run_produces_listable_scheduled_run() {
        let tmp = tempfile::tempdir().unwrap();
        let ws_root = tmp.path().to_path_buf();
        let slug = "items";
        write_workspace(&ws_root, slug, RECIPE);

        // Point the recipe at a wiremock server that emits two
        // `items` so we can assert the `Item` count downstream.
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [{"id": "a"}, {"id": "b"}],
            })))
            .mount(&mock)
            .await;
        let recipe_path = ws_root.join(format!("{slug}.forage"));
        rewrite_url(&recipe_path, &format!("{}/items", mock.uri()));

        let daemon = Daemon::open(ws_root.clone()).expect("open daemon");
        let cfg = RunConfig {
            cadence: Cadence::Manual,
            output: ws_root.join(".forage").join("data").join("items.sqlite"),
            enabled: true,
            inputs: indexmap::IndexMap::new(),
        };
        let run = daemon.configure_run(slug, cfg).expect("configure_run");
        deploy_from_disk(&daemon, &ws_root, slug);

        let sr = daemon.trigger_run(&run.id).await.expect("trigger_run");
        assert_eq!(sr.outcome, Outcome::Ok, "stall: {:?}", sr.stall);
        assert_eq!(sr.trigger, Trigger::Manual);
        assert_eq!(sr.counts.get("Item").copied(), Some(2));

        let listed = daemon
            .list_scheduled_runs(&run.id, 10, None)
            .expect("list_scheduled_runs");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, sr.id);

        // `load_records` round-trips the actual emitted rows back
        // from the output store.
        let records = daemon
            .load_records(&sr.id, "Item", 10)
            .expect("load_records");
        assert_eq!(records.len(), 2);
    }

    /// Studio's `run_recipe` reads engine inputs from the daemon's
    /// `Run.inputs` field. A `configure_run` that stamps an input must
    /// be readable back through `get_run_by_name` keyed on the recipe
    /// header — that's the exact lookup the command does before it
    /// hands the bindings to the engine.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn run_recipe_reads_inputs_from_daemon_run() {
        let tmp = tempfile::tempdir().unwrap();
        let ws_root = tmp.path().to_path_buf();
        let slug = "tenant-items";
        write_workspace(&ws_root, slug, RECIPE);

        let daemon = Daemon::open(ws_root.clone()).expect("open daemon");
        let mut inputs = indexmap::IndexMap::new();
        inputs.insert(
            "tenant".to_string(),
            serde_json::Value::String("acme".into()),
        );
        let cfg = RunConfig {
            cadence: Cadence::Manual,
            output: ws_root.join(".forage").join("data").join("items.sqlite"),
            enabled: true,
            inputs: inputs.clone(),
        };
        daemon.configure_run(slug, cfg).expect("configure_run");

        // The same lookup `run_recipe` performs before calling the
        // engine. Pre-Phase-10 this read came off
        // `<slug>/fixtures/inputs.json`; now the row is authoritative.
        let run = daemon
            .get_run_by_name(slug)
            .expect("get_run_by_name")
            .expect("Run row present after configure");
        assert_eq!(run.inputs, inputs);

        // A recipe that hasn't been configured yet hands `run_recipe`
        // an empty map — the engine then surfaces a clean
        // "missing input" error for recipes that declare any.
        assert!(
            daemon
                .get_run_by_name("never-configured")
                .expect("get_run_by_name")
                .is_none()
        );
    }

    use super::build_recipe_statuses;
    use crate::workspace::{self, DeployedState, DraftState};

    /// A workspace whose source file basename differs from the
    /// recipe header (`foo.forage` containing `recipe "bar"`)
    /// surfaces one status entry keyed on `"bar"`, and a deploy made
    /// under that header name shows up paired in the same entry.
    /// Pre-Phase-4 the join would have used the file basename on the
    /// draft side and the daemon's path-derived slug on the deployed
    /// side; the same recipe would have appeared as two unrelated
    /// entries.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn list_recipe_statuses_joins_on_header_name() {
        let tmp = tempfile::tempdir().unwrap();
        let ws_root = tmp.path().to_path_buf();
        std::fs::write(
            ws_root.join("forage.toml"),
            "description = \"\"\ncategory = \"\"\ntags = []\n",
        )
        .unwrap();
        // File basename `foo`; recipe header `bar`.
        std::fs::write(
            ws_root.join("foo.forage"),
            "recipe \"bar\"\nengine http\n",
        )
        .unwrap();

        let daemon = Daemon::open(ws_root.clone()).expect("open daemon");
        let workspace = forage_core::workspace::load(&ws_root).expect("load workspace");
        let recipe = forage_core::parse(&std::fs::read_to_string(ws_root.join("foo.forage")).unwrap())
            .expect("parse");
        let catalog = workspace
            .catalog(&recipe, |p| std::fs::read_to_string(p))
            .expect("catalog");
        let wire = forage_core::workspace::SerializableCatalog::from(catalog);
        daemon
            .deploy("bar", "recipe \"bar\"\nengine http\n".to_string(), wire)
            .expect("deploy");

        let statuses = build_recipe_statuses(&workspace, &daemon).expect("build_recipe_statuses");
        assert_eq!(
            statuses.len(),
            1,
            "draft + deployment must collapse into one entry: {statuses:?}",
        );
        let status = &statuses[0];
        assert_eq!(status.name, "bar", "status keys on the header name");
        let DraftState::Valid { ref path } = status.draft else {
            panic!("draft side picked up the parsed recipe: {status:?}");
        };
        assert_eq!(
            path,
            std::path::Path::new("foo.forage"),
            "DraftState.path is workspace-relative so the UI can join it against FileNode.path",
        );
        assert!(
            matches!(status.deployed, DeployedState::Deployed { version: 1, .. }),
            "deployment side joined under the header name: {status:?}",
        );
    }

    const FOO_BAR_RECIPE: &str = r#"recipe "bar"
engine http

type Item {
    id: String
}

step list {
    method "GET"
    url    "https://example.test/items"
}

for $i in $list.items[*] {
    emit Item {
        id ← $i.id
    }
}
"#;

    /// A file whose basename (`foo.forage`) differs from its recipe
    /// header (`recipe "bar"`) is reachable end-to-end through the
    /// recipe-name-keyed wire shape Phase 7 introduced. The
    /// resolver, the source read, the deploy, the scheduled-run
    /// trigger, and the recipe-name stamp on the resulting record
    /// all key on `"bar"`, never on `"foo"`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn foo_forage_with_recipe_bar_round_trips_by_name() {
        let tmp = tempfile::tempdir().unwrap();
        let ws_root = tmp.path().to_path_buf();
        std::fs::write(
            ws_root.join("forage.toml"),
            "description = \"\"\ncategory = \"\"\ntags = []\n",
        )
        .unwrap();
        let foo_path = ws_root.join("foo.forage");
        std::fs::write(&foo_path, FOO_BAR_RECIPE).unwrap();

        // Wire the recipe at a wiremock that returns two items so the
        // scheduled-run record carries verifiable counts.
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [{"id": "a"}, {"id": "b"}],
            })))
            .mount(&mock)
            .await;
        rewrite_url(&foo_path, &format!("{}/items", mock.uri()));

        let ws = forage_core::workspace::load(&ws_root).expect("load workspace");

        // The Studio resolver path: name → file path → source. The
        // helper has to consult `Workspace::recipe_by_name`; if it
        // ever falls back to the basename it'd hit `foo.forage` via
        // the slug-derivation that pre-Phase-7 builds used to do.
        // `Workspace::load` canonicalizes, so compare canonical
        // paths — the test fixture's path may carry a `/private`
        // prefix on macOS.
        let resolved = workspace::resolve_recipe_path(&ws, "bar").expect("resolve by name");
        assert_eq!(resolved.canonicalize().unwrap(), foo_path.canonicalize().unwrap());
        let source = workspace::read_source(&ws, "bar").expect("read source by name");
        assert!(source.contains("recipe \"bar\""));

        // Deploy + configure_run + trigger_run, all keyed on `"bar"`.
        let daemon = Daemon::open(ws_root.clone()).expect("open daemon");
        let recipe = forage_core::parse(&source).expect("parse");
        let catalog = ws
            .catalog(&recipe, |p| std::fs::read_to_string(p))
            .expect("catalog");
        let wire = forage_core::workspace::SerializableCatalog::from(catalog);
        daemon.deploy("bar", source, wire).expect("deploy");

        let cfg = RunConfig {
            cadence: Cadence::Manual,
            output: ws_root.join(".forage").join("data").join("bar.sqlite"),
            enabled: true,
            inputs: indexmap::IndexMap::new(),
        };
        let run = daemon.configure_run("bar", cfg).expect("configure_run");
        assert_eq!(run.recipe_name, "bar");

        let sr = daemon.trigger_run(&run.id).await.expect("trigger_run");
        assert_eq!(sr.outcome, Outcome::Ok, "stall: {:?}", sr.stall);
        assert_eq!(sr.counts.get("Item").copied(), Some(2));

        // `delete_recipe` by name removes the actual file backing the
        // recipe header — `foo.forage`, not a `bar/` directory.
        workspace::delete_recipe(&ws, "bar").expect("delete by name");
        assert!(!foo_path.exists());
    }

    use super::{resolve_existing, resolve_new, validate_path};

    /// A symlink inside the workspace pointing outside it must be
    /// rejected by `resolve_existing` — otherwise `load_file` would
    /// happily read `/etc/passwd` through it.
    #[cfg(unix)]
    #[test]
    fn resolve_existing_rejects_symlink_escape() {
        let tmp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.txt"), "🤫").unwrap();
        std::os::unix::fs::symlink(outside.path(), tmp.path().join("evil")).unwrap();

        let err = resolve_existing(tmp.path(), Path::new("evil/secret.txt"))
            .expect_err("symlink escape must be rejected");
        assert!(
            err.contains("escapes workspace root"),
            "unexpected error: {err}"
        );
    }

    /// `resolve_new` is the `save_file` path. A symlink to outside
    /// the workspace as the *parent* must be rejected so writes
    /// can't reach `/tmp/whatever/x` via `<ws>/evil/x`.
    #[cfg(unix)]
    #[test]
    fn resolve_new_rejects_symlinked_parent() {
        let tmp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), tmp.path().join("evil")).unwrap();

        let err = resolve_new(tmp.path(), Path::new("evil/new.txt"))
            .expect_err("symlinked parent must be rejected");
        assert!(
            err.contains("escapes workspace root"),
            "unexpected error: {err}"
        );
        // The write must not have landed in the outside dir either.
        assert!(!outside.path().join("new.txt").exists());
    }

    /// Empty / file-name-less paths are rejected before they ever
    /// hit the filesystem, so `path == "" => root` doesn't fall
    /// through and confuse the rest of the pipeline.
    #[test]
    fn resolve_rejects_empty_path() {
        let tmp = tempfile::tempdir().unwrap();
        let err =
            resolve_existing(tmp.path(), Path::new("")).expect_err("empty path must be rejected");
        assert!(err.contains("empty path"), "unexpected error: {err}");
        let err = resolve_new(tmp.path(), Path::new("")).expect_err("empty path must be rejected");
        assert!(err.contains("empty path"), "unexpected error: {err}");
    }

    /// A sidecar `.forage` file deeper than the workspace root is
    /// unclassified — validate_path surfaces a diagnostic instead of
    /// silently treating it as a declarations file. (Distinct from a
    /// legacy `<slug>/recipe.forage`, which gets the migration
    /// prompt; see the unmigrated-workspace test below.)
    #[test]
    fn validate_path_rejects_sidecar_forage_in_subfolder() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("trilogy")).unwrap();
        let sidecar = root.join("trilogy").join("sidecar.forage");
        std::fs::write(&sidecar, "type X { id: String }\n").unwrap();

        let outcome = validate_path(root, &sidecar, "type X { id: String }\n");
        assert!(!outcome.ok, "sidecar must not validate clean");
        assert_eq!(outcome.diagnostics.len(), 1);
        assert_eq!(outcome.diagnostics[0].code, "UnrecognizedForageFile");
    }

    /// A `.forage` file at the pre-Phase-10 legacy slot
    /// (`<root>/<slug>/recipe.forage`) is an unmigrated workspace.
    /// validate_path surfaces the migration prompt so the UI can route
    /// the user at `forage migrate` instead of trying to parse against
    /// a shape Studio no longer supports.
    #[test]
    fn validate_path_flags_legacy_recipe_path_as_unmigrated() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("legacy")).unwrap();
        let legacy = root.join("legacy").join("recipe.forage");
        std::fs::write(&legacy, "recipe \"legacy\"\nengine http\n").unwrap();

        let outcome = validate_path(root, &legacy, "recipe \"legacy\"\nengine http\n");
        assert!(!outcome.ok, "legacy path must not validate clean");
        assert_eq!(outcome.diagnostics.len(), 1);
        assert_eq!(outcome.diagnostics[0].code, "UnmigratedWorkspace");
        assert!(
            outcome.diagnostics[0].message.contains("forage migrate"),
            "expected migrate prompt; got {:?}",
            outcome.diagnostics[0].message,
        );
    }

    /// Root-level `.forage` files run through workspace-aware
    /// validation. A header-less file with only a clean type
    /// declaration produces no diagnostics.
    #[test]
    fn validate_path_treats_root_forage_as_source() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let decl = root.join("cannabis.forage");
        std::fs::write(&decl, "type Dispensary { id: String }\n").unwrap();

        let outcome = validate_path(root, &decl, "type Dispensary { id: String }\n");
        assert!(
            outcome.ok,
            "header-less file should validate clean: {outcome:?}"
        );
        assert!(outcome.diagnostics.is_empty());
    }

    /// A header-less file with a stray recipe-context form (here an
    /// `auth` block without a `recipe` header) must surface
    /// `RecipeContextWithoutHeader`. The greenfield parser accepts the
    /// shape; the validator is where this lands.
    #[test]
    fn validate_path_flags_recipe_context_in_header_less_file() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::write(root.join("forage.toml"), "description = \"\"\ncategory = \"\"\ntags = []\n").unwrap();
        let decl = root.join("stray.forage");
        let src = "auth.staticHeader { name: \"X-API-Key\", value: \"abc\" }\n";
        std::fs::write(&decl, src).unwrap();

        let outcome = validate_path(root, &decl, src);
        assert!(
            !outcome.ok,
            "stray auth without a recipe header must not validate clean: {outcome:?}",
        );
        assert!(
            outcome
                .diagnostics
                .iter()
                .any(|d| d.code == "RecipeContextWithoutHeader"),
            "expected RecipeContextWithoutHeader; got {outcome:?}",
        );
    }

    use super::append_workspace_shared_diagnostics;

    /// Saving a second file that declares `share fn upper(...)` when a
    /// sibling already declares the same share decl must surface
    /// `DuplicateSharedDeclaration` against the just-saved file — the
    /// cross-file pass is wired into the save outcome, not just the
    /// per-file validator. `fn` chosen over `type` so the workspace
    /// type-catalog's cross-file dedup doesn't pre-empt this check.
    #[test]
    fn save_surfaces_workspace_share_collision() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::write(
            root.join("forage.toml"),
            "description = \"\"\ncategory = \"\"\ntags = []\n",
        )
        .unwrap();
        std::fs::write(root.join("a.forage"), "share fn upper($x) { $x }\n").unwrap();

        // Simulate `save_file` on b.forage. After writing the focal
        // file, the helper sees both files on disk and routes the
        // collision back to b.forage's outcome.
        let b_path = root.join("b.forage");
        let b_src = "share fn upper($x) { $x }\n";
        std::fs::write(&b_path, b_src).unwrap();
        let mut outcome = validate_path(root, &b_path, b_src);
        assert!(
            outcome.ok,
            "per-file validator alone should still pass on b.forage: {outcome:?}",
        );
        append_workspace_shared_diagnostics(root, &b_path, b_src, &mut outcome);
        assert!(
            !outcome.ok,
            "cross-file share collision must mark b.forage as failed: {outcome:?}",
        );
        assert!(
            outcome
                .diagnostics
                .iter()
                .any(|d| d.code == "DuplicateSharedDeclaration"),
            "expected DuplicateSharedDeclaration; got {outcome:?}",
        );
    }

    use super::{
        StudioState, close_workspace_inner, scaffold_new_workspace, validate_workspace_path,
    };

    /// `open_workspace` must refuse a directory that isn't a workspace.
    /// The check is the first thing `open_workspace_inner` does after
    /// closing any prior session; surfacing it via the extracted helper
    /// keeps the assertion on the production rejection branch.
    #[test]
    fn open_workspace_rejects_dir_without_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        let err = validate_workspace_path(tmp.path())
            .expect_err("dir without forage.toml must be rejected");
        assert!(
            err.contains("missing forage.toml"),
            "unexpected error: {err}"
        );
    }

    /// `close_workspace` is idempotent: a second close on an
    /// already-empty state is a no-op, not a panic. The frontend can
    /// fire ⌘W from a Welcome screen without special-casing it.
    #[test]
    fn close_workspace_idempotent() {
        let state = StudioState::new_empty();
        // No prior session installed — both calls hit the `was_open ==
        // false` branch and return without touching the AppHandle.
        close_workspace_inner(&state, None);
        close_workspace_inner(&state, None);
        assert!(state.session.load().is_none());
    }

    /// `new_workspace` must refuse a directory that already has a
    /// `forage.toml` — the user should pick Open instead. Test goes
    /// through `scaffold_new_workspace` so the assertion sits on the
    /// `String` error the command surfaces to the frontend, not the
    /// underlying `io::Error`.
    #[test]
    fn new_workspace_rejects_dir_with_existing_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("forage.toml"), "").unwrap();
        let err = scaffold_new_workspace(tmp.path())
            .expect_err("dir with existing manifest must be rejected");
        assert!(
            err.contains("already has a forage.toml"),
            "unexpected error: {err}"
        );
    }
}
