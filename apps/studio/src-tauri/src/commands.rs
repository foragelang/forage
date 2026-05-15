//! Tauri commands exposed to the frontend.

use async_trait::async_trait;
use indexmap::IndexMap;
use serde::Serialize;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindowBuilder};
use tokio::sync::Notify;
use ts_rs::TS;

use forage_browser::run_browser_replay;
use forage_core::ast::EngineKind;
use forage_core::eval::default_registry;
use forage_core::parse::parse_extraction;
use forage_core::{
    EvalValue, Evaluator, LineMap, RunOptions, Scope, Snapshot, parse, validate,
};
use forage_http::{
    Debugger, EmitPause, Engine, ForLoopPause, LiveTransport, ProgressSink, ReplayTransport,
    ResumeAction, RunEvent, StepPause, StepResponse,
};
use forage_hub::{AuthStore, AuthTokens};

use crate::state::StepKind;

/// Tauri event name for streaming engine progress to the frontend.
pub const RUN_EVENT: &str = "forage:run-event";
/// Tauri event name for the engine telling the frontend it has paused
/// somewhere — at a `step` boundary, an `emit`, or on `for`-loop entry.
/// Payload is `PausePayload` (JSON) with a `kind` discriminator. The
/// studio's debugger short-circuits `before_iteration` to Continue so
/// the per-iteration pause site doesn't surface here.
pub const DEBUG_PAUSED_EVENT: &str = "forage:debug-paused";
/// Fired once at the top of every `run_recipe` invocation, before the
/// engine starts. Payload `{ run_id }`. The frontend uses the run_id
/// to correlate subsequent step-response events to one run and to
/// reset any pop-out window's local response cache.
pub const RUN_BEGIN_EVENT: &str = "forage:run-begin";
/// Fired after every step's response is captured (whether the run
/// proceeds or aborts on a 4xx/5xx). Payload `StepResponseEvent`.
pub const RUN_STEP_RESPONSE_EVENT: &str = "forage:run-step-response";
/// Fired when `debug_resume` wakes the engine. Subscribers can clear
/// pop-out-window pause state without waiting for the eventual run
/// success / failure event. Payload `{ run_id, action }`.
pub const RUN_DEBUG_RESUMED_EVENT: &str = "forage:run-debug-resumed";

/// What the engine paused on. Wraps the three `forage-http` pause
/// payloads the studio surfaces in a tagged union so the frontend can
/// render any shape with one event listener. The studio doesn't
/// surface `iteration` pauses (every per-iteration breakpoint is
/// expressed as a line-keyed BP on a body statement); the engine still
/// fires `before_iteration`, but the studio's debugger short-circuits
/// it to Continue.
#[derive(Serialize, TS)]
#[ts(export)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PausePayload {
    Step(StepPause),
    Emit(EmitPause),
    ForLoop(ForLoopPause),
}

/// Wire shape for `RUN_STEP_RESPONSE_EVENT`. Carries the captured
/// response alongside its step name + the run id minted at
/// `run_recipe` start so the frontend (and any pop-out window) can
/// associate the event with the correct run.
#[derive(Serialize, TS)]
#[ts(export)]
pub struct StepResponseEvent {
    pub run_id: String,
    pub step: String,
    pub response: StepResponse,
}

/// Wire shape for `RUN_BEGIN_EVENT` / `RUN_DEBUG_RESUMED_EVENT`.
#[derive(Serialize, TS)]
#[ts(export)]
pub struct RunBeginEvent {
    pub run_id: String,
}

#[derive(Serialize, TS)]
#[ts(export)]
pub struct RunDebugResumedEvent {
    pub run_id: String,
    pub action: ResumeAction,
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

/// Structural outline of a recipe — every pause-able statement
/// (`step` / `emit` / `for`) with its source location. Studio anchors
/// gutter affordances and the "reveal paused statement" jump on this.
/// The validator-clean `parse` returns either an `Empty` body or a
/// `Scraping` one; composition bodies have no pause points, so the
/// outline for those is the empty list.
#[derive(Serialize, Default, TS)]
#[ts(export)]
pub struct RecipeOutline {
    pub pause_points: Vec<PausePoint>,
}

/// One pause-able statement in source order. The variant tells the UI
/// which gutter glyph to draw (step / emit / for) and the inner fields
/// carry the identifier the gutter tooltip shows.
#[derive(Serialize, TS)]
#[ts(export)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PausePoint {
    Step {
        /// Step identifier (`step <name> { … }`).
        name: String,
        /// 0-based line/col of the `step` keyword's span start.
        start_line: u32,
        start_col: u32,
        end_line: u32,
        end_col: u32,
    },
    Emit {
        /// Record type being emitted (`emit <TypeName> { … }`).
        type_name: String,
        start_line: u32,
        start_col: u32,
        end_line: u32,
        end_col: u32,
    },
    For {
        /// Loop variable name from `for $<variable> in …`.
        variable: String,
        start_line: u32,
        start_col: u32,
        end_line: u32,
        end_col: u32,
    },
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
///
/// The sink additionally bridges the engine's `step_response_full_body`
/// hook: every step's uncapped response body lands in
/// `<workspace>/.forage/runs/<run_id>/responses/<step>.raw` so the
/// "load full" UI affordance can read it back when the user clicks it
/// against a 1 MiB truncated response. The wire-side `body_raw` stays
/// truncated; only the disk artifact carries the full bytes.
struct EmitterSink {
    tx: tokio::sync::mpsc::UnboundedSender<RunEvent>,
    app: AppHandle,
    run_id: String,
    /// Workspace root captured at sink construction. The full-body
    /// path build resolves against this so a workspace switch
    /// mid-run can't redirect bytes onto a freshly opened workspace.
    workspace_root: std::path::PathBuf,
}

impl ProgressSink for EmitterSink {
    fn emit(&self, event: RunEvent) {
        // Send failures are non-fatal (drainer dropped): the run
        // continues even if the UI can't hear it anymore.
        let _ = self.tx.send(event);
    }

    fn step_response_captured(&self, step: &str, response: &StepResponse) {
        // Broadcast the wire-sized capture so the UI's Inspector
        // "Responses" tab + the pop-out window can render the step's
        // shape independent of any pause. Direct app.emit rather than
        // riding the RunEvent channel because the payload is the
        // already-truncated StepResponse, not the run-event variant
        // shape.
        emit_step_response(&self.app, &self.run_id, step, response);
    }

    fn step_response_full_body(&self, step: &str, body: &[u8]) {
        // ULIDs are 26 chars of Crockford base32; step names follow
        // the identifier grammar. Both are validated at the path-build
        // site so a malformed run_id or step name can't escape the
        // workspace via path traversal.
        let path = match crate::run_artifacts::full_body_path(
            &self.workspace_root,
            &self.run_id,
            step,
        ) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(
                    run_id = %self.run_id,
                    step = %step,
                    error = %e,
                    "step_response_full_body: rejected path"
                );
                return;
            }
        };
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                tracing::warn!(
                    path = %parent.display(),
                    error = %e,
                    "step_response_full_body: create dir failed"
                );
                return;
            }
        }
        if let Err(e) = std::fs::write(&path, body) {
            tracing::warn!(
                path = %path.display(),
                error = %e,
                "step_response_full_body: write failed"
            );
        }
    }
}

/// Convenience hook the engine doesn't know about: emit one
/// `RUN_STEP_RESPONSE_EVENT` per captured step response. Called by
/// `run_recipe` after polling `step_responses` for new entries during
/// the run; surfaces 4xx/5xx captures + every healthy response with
/// the same payload shape.
fn emit_step_response(
    app: &AppHandle,
    run_id: &str,
    step: &str,
    response: &StepResponse,
) {
    let _ = app.emit(
        RUN_STEP_RESPONSE_EVENT,
        &StepResponseEvent {
            run_id: run_id.to_string(),
            step: step.to_string(),
            response: response.clone(),
        },
    );
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
/// Every pause site (`before_step` / `before_emit` / `before_for_loop`)
/// flows through one `should_pause(start_line)` gate against the
/// shared line-keyed breakpoint set and the per-session `step_kind`
/// one-shot. The engine fires `before_iteration` too but the studio
/// short-circuits it to Continue — every per-iteration pause is
/// expressed as a line-keyed BP on a body statement.
///
/// When we do pause: stash a clone of the live scope on the session
/// (the watch / REPL commands evaluate against it), install a fresh
/// resume oneshot, emit `forage:debug-paused`, then await the receiver.
///
/// If the receiver drops (e.g. window closed, run cancelled), we default to
/// `Stop` so a stranded engine task doesn't hang on an unresumable pause.
struct StudioDebugger {
    app: AppHandle,
    session: Arc<crate::state::DebugSession>,
}

#[async_trait]
impl Debugger for StudioDebugger {
    async fn before_step(&self, pause: StepPause, scope: &Scope) -> ResumeAction {
        if !self.should_pause(pause.start_line) {
            return ResumeAction::Continue;
        }
        self.wait(PausePayload::Step(pause), scope).await
    }

    async fn before_emit(&self, pause: EmitPause, scope: &Scope) -> ResumeAction {
        if !self.should_pause(pause.start_line) {
            return ResumeAction::Continue;
        }
        self.wait(PausePayload::Emit(pause), scope).await
    }

    async fn before_for_loop(&self, pause: ForLoopPause, scope: &Scope) -> ResumeAction {
        if !self.should_pause(pause.start_line) {
            return ResumeAction::Continue;
        }
        self.wait(PausePayload::ForLoop(pause), scope).await
    }

    // Iteration pauses are not surfaced — the line-keyed model expresses
    // per-iteration breakpoints as BPs on body statements. The default
    // trait impl (Continue) is exactly what we want; we override
    // nothing here so the engine's call short-circuits at the trait
    // boundary without consulting our state.
}

impl StudioDebugger {
    /// One line-keyed gate covering every pause site. Reads the
    /// breakpoint set lock-free (ArcSwap) and consumes the one-shot
    /// `step_kind` if it was set — both Step Over and Step In force
    /// the next pause regardless of breakpoint, then revert to
    /// BP-only behavior.
    fn should_pause(&self, start_line: u32) -> bool {
        let state = self.app.state::<crate::state::StudioState>();
        let on_breakpoint = state.breakpoints.load().contains(&start_line);
        let pending = self
            .session
            .step_kind
            .swap(StepKind::None as u8, std::sync::atomic::Ordering::SeqCst);
        let stepping = !matches!(StepKind::from_u8(pending), StepKind::None);
        on_breakpoint || stepping
    }

    /// Park the engine task on a fresh oneshot, emit the pause payload
    /// to the frontend, stash a snapshot of the live scope on the
    /// session for the watch / REPL evaluators, and await the user's
    /// resume action. Shared by every surfaced pause site.
    async fn wait(&self, payload: PausePayload, scope: &Scope) -> ResumeAction {
        let (tx, rx) = tokio::sync::oneshot::channel();
        // The Mutex is the right primitive here — see DebugSession docs:
        // we need atomic take-and-fire on the resume path so two
        // concurrent debug_resume callers can't both grab the sender.
        *self
            .session
            .pending
            .lock()
            .expect("debug session pending sender") = Some(tx);
        *self
            .session
            .paused_scope
            .lock()
            .expect("debug session paused_scope") = Some(scope.clone());
        let _ = self.app.emit(DEBUG_PAUSED_EVENT, &payload);
        let action = rx.await.unwrap_or(ResumeAction::Stop);
        // Clear the scope on resume so a stale snapshot can't survive
        // a continue. The next pause writes a fresh one.
        *self
            .session
            .paused_scope
            .lock()
            .expect("debug session paused_scope") = None;
        action
    }
}

/// Toolbar-level flag state. The frontend's run toolbar wires three
/// independent toggles + a preset selector to this shape; the dev /
/// prod presets are sugar at the React layer, so the backend only
/// sees the resolved values. `None` for every field reverts to the
/// dev defaults (sample 10, replay against fixtures, ephemeral).
#[derive(Debug, Clone, serde::Deserialize, TS)]
#[ts(export)]
pub struct RunRecipeFlags {
    /// Cap each top-level for-loop at this many items. Absent =
    /// preset default.
    pub sample_limit: Option<u32>,
    /// Replay against the workspace's `_fixtures/<recipe>.jsonl`
    /// instead of hitting the network. Absent = preset default.
    pub replay: Option<bool>,
    /// Skip the persistent output store. Absent = preset default.
    pub ephemeral: Option<bool>,
}

impl RunRecipeFlags {
    /// Fill in each `None` from the dev preset's defaults so the
    /// engine sees a fully-resolved shape.
    fn resolve(self) -> (Option<u32>, bool, bool) {
        let defaults = forage_daemon::RunFlags::dev();
        let sample = self.sample_limit.or(defaults.sample_limit);
        let replay = self.replay.unwrap_or(defaults.replay.is_some());
        let ephemeral = self.ephemeral.unwrap_or(defaults.ephemeral);
        (sample, replay, ephemeral)
    }
}

#[tauri::command]
pub async fn run_recipe(
    app: AppHandle,
    state: State<'_, crate::state::StudioState>,
    name: String,
    flags: Option<RunRecipeFlags>,
) -> Result<RunOutcome, String> {
    // The editor's "Run" button defaults to the dev preset; the run
    // toolbar exposes three toggles + a preset selector and the
    // resolved values arrive here as `flags`. Missing fields fall
    // back to the dev preset's defaults so the frontend can omit
    // anything it isn't overriding.
    let raw_flags = flags.unwrap_or(RunRecipeFlags {
        sample_limit: None,
        replay: None,
        ephemeral: None,
    });
    let (sample_limit, replay, ephemeral) = raw_flags.resolve();
    tracing::info!(
        name = %name,
        sample_limit = ?sample_limit,
        replay,
        ephemeral,
        "run_recipe",
    );
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
    let signatures = build_signatures(&ws.root);
    let report = validate(&recipe, &catalog, &signatures);
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
    // Replay reads the recipe-name-keyed JSONL stream
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

    // Mint a fresh run id at run start. Used by the
    // `RUN_BEGIN_EVENT` / `RUN_STEP_RESPONSE_EVENT` /
    // `RUN_DEBUG_RESUMED_EVENT` events so the frontend (and the
    // optional pop-out Response window) can correlate captures with
    // the run they came from. Also keyed by `EmitterSink` for the
    // on-disk full-body stash. ULID is monotonically sortable so
    // simultaneous runs in different windows wouldn't collide either
    // — Studio runs one recipe at a time today, but the choice
    // anticipates the parallel case.
    let run_id = ulid::Ulid::new().to_string();
    let _ = app.emit(
        RUN_BEGIN_EVENT,
        &RunBeginEvent {
            run_id: run_id.clone(),
        },
    );

    let (tx, drainer_handle) = spawn_event_drainer(app.clone());
    let sink: Arc<dyn ProgressSink> = Arc::new(EmitterSink {
        tx,
        app: app.clone(),
        run_id: run_id.clone(),
        workspace_root: ws.root.clone(),
    });

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
        let session = Arc::new(crate::state::DebugSession {
            pending: std::sync::Mutex::new(None),
            step_kind: std::sync::atomic::AtomicU8::new(StepKind::None as u8),
            paused_scope: std::sync::Mutex::new(None),
            run_id: run_id.clone(),
        });
        state.debug_session.store(Some(session.clone()));
        Some(Arc::new(StudioDebugger {
            app: app.clone(),
            session,
        }))
    } else {
        None
    };

    let run_options = RunOptions { sample_limit };
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
                r = engine.run(&recipe, &catalog, inputs, secrets, &run_options) => r.map_err(|e| format!("{e}")),
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
                r = engine.run(&recipe, &catalog, inputs, secrets, &run_options) => r.map_err(|e| format!("{e}")),
            }
        }
        (EngineKind::Browser, true) => {
            run_browser_replay(&recipe, &catalog, &captures, inputs, secrets, &run_options)
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
                &run_options,
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
                    Ok(run) => {
                        // Non-ephemeral editor runs land the snapshot
                        // in the same persistent `.forage/data/<recipe>.sqlite`
                        // a scheduled fire would write to. Ephemeral
                        // (the dev preset default) skips the write so
                        // playground runs don't bleed into the prod
                        // table. Either way, the in-memory snapshot
                        // still rides back on the outcome.
                        if !ephemeral {
                            if let Err(e) = persist_snapshot(&recipe, &catalog, &run, &s) {
                                tracing::warn!(
                                    recipe_name = %header,
                                    error = %e,
                                    "persist after dev-run failed",
                                );
                                Some(format!("persist failed: {e}"))
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    }
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

/// Write the in-memory snapshot to the Run's configured output store.
/// Used by `run_recipe` when the toolbar's "ephemeral" toggle is off
/// so a Studio-driven run lands records in the same SQLite table the
/// scheduler would have populated.
fn persist_snapshot(
    recipe: &forage_core::ForageFile,
    catalog: &forage_core::TypeCatalog,
    run: &forage_daemon::Run,
    snapshot: &Snapshot,
) -> Result<(), String> {
    let tables = forage_daemon::derive_schema(recipe, catalog);
    let mut store = forage_daemon::OutputStore::open(&run.output, tables)
        .map_err(|e| format!("open output store: {e}"))?;
    let scheduled_run_id = ulid::Ulid::new().to_string();
    let at_ms = chrono::Utc::now().timestamp_millis();
    let mut tx = store.begin_tx().map_err(|e| format!("begin tx: {e}"))?;
    for rec in &snapshot.records {
        tx.write_record(
            &scheduled_run_id,
            at_ms,
            &rec.id,
            &rec.type_name,
            &rec.fields,
        )
        .map_err(|e| format!("write record {}: {e}", rec.id))?;
    }
    tx.commit().map_err(|e| format!("commit: {e}"))?;
    Ok(())
}

/// Resume a paused debug step. `action` is `"continue"`, `"step_over"`,
/// `"step_in"`, or `"stop"`. No-op when no run is in flight or no pause
/// is pending — the UI can fire and forget without coordinating
/// against state itself.
///
/// Step Over and Step In set a one-shot `step_kind` flag on the
/// session that forces the *next* pause-able statement to pause
/// regardless of whether it's on a breakpoint. The engine currently
/// treats `StepOver` and `StepIn` as equivalent; the wire-distinct
/// shape carries through so a future body-suppression pass can
/// differentiate them without a wire break.
#[tauri::command]
pub fn debug_resume(
    app: AppHandle,
    state: State<'_, crate::state::StudioState>,
    action: String,
) -> Result<(), String> {
    let Some(session) = state.debug_session.load_full() else {
        return Ok(());
    };

    let (resume, step_kind) = match action.as_str() {
        "continue" => (ResumeAction::Continue, StepKind::None),
        "step_over" => (ResumeAction::StepOver, StepKind::Over),
        "step_in" => (ResumeAction::StepIn, StepKind::In),
        "stop" => (ResumeAction::Stop, StepKind::None),
        other => return Err(format!("unknown debug action: {other}")),
    };
    session
        .step_kind
        .store(step_kind as u8, std::sync::atomic::Ordering::SeqCst);

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
    // Notify any pop-out Response window so its local pause state
    // clears alongside the main window's. The main window's store
    // listens to the pause event flow directly; the pop-out has no
    // other channel to clear pause state on a resume.
    let _ = app.emit(
        RUN_DEBUG_RESUMED_EVENT,
        &RunDebugResumedEvent {
            run_id: session.run_id.clone(),
            action: resume,
        },
    );
    Ok(())
}

/// Replace the current in-memory breakpoint set. Lines not on any
/// pause-able statement are harmless — the engine simply never reaches
/// a pause site at that line.
///
/// Per-recipe persistence is handled by `set_recipe_breakpoints` /
/// `load_recipe_breakpoints` below. The frontend pushes via *this*
/// command on recipe switch so the engine's hot-path read sees the
/// new recipe's set, then persists the user's edits through the
/// recipe-scoped commands.
#[tauri::command]
pub fn set_breakpoints(
    state: State<'_, crate::state::StudioState>,
    lines: Vec<u32>,
) -> Result<(), String> {
    state
        .breakpoints
        .store(Arc::new(lines.into_iter().collect()));
    Ok(())
}

/// Persist a recipe's breakpoint set to the workspace sidecar and push it
/// to the in-memory cache the engine reads on pause. Empty set deletes
/// the recipe's entry so the sidecar doesn't grow stale.
#[tauri::command]
pub fn set_recipe_breakpoints(
    state: State<'_, crate::state::StudioState>,
    name: String,
    lines: Vec<u32>,
) -> Result<(), String> {
    let ws = require_workspace(&state)?;
    let mut all = workspace::read_breakpoints(&ws.root).map_err(|e| e.to_string())?;
    if lines.is_empty() {
        all.remove(&name);
    } else {
        all.insert(name, lines.clone());
    }
    workspace::write_breakpoints(&ws.root, &all).map_err(|e| e.to_string())?;
    state
        .breakpoints
        .store(Arc::new(lines.into_iter().collect()));
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
) -> Result<Vec<u32>, String> {
    let ws = require_workspace(&state)?;
    let mut map = workspace::read_breakpoints(&ws.root).map_err(|e| e.to_string())?;
    Ok(map.remove(&name).unwrap_or_default())
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

/// Evaluate one Forage extraction expression against the paused scope.
/// The watch panel and the REPL both go through this — they parse the
/// user's input, evaluate against the live scope the studio debugger
/// stashed on `DebugSession.paused_scope`, and return the result as a
/// JSON value. Errors as a String the UI surfaces inline (parse
/// failure, not-paused, evaluator error).
#[tauri::command]
pub async fn eval_watch_expression(
    state: State<'_, crate::state::StudioState>,
    expr_source: String,
) -> Result<serde_json::Value, String> {
    let Some(session) = state.debug_session.load_full() else {
        return Err("not paused".to_string());
    };
    let scope = {
        let guard = session
            .paused_scope
            .lock()
            .expect("debug session paused_scope");
        guard.clone()
    };
    let Some(scope) = scope else {
        return Err("not paused".to_string());
    };
    let expr = parse_extraction(&expr_source).map_err(|e| format!("parse: {e}"))?;
    // Watches evaluate against the built-in registry only — user
    // functions live in a recipe-specific scope we don't have on
    // hand here. Studio doesn't yet surface watch errors that come
    // from "missing user fn"; the watch source is a one-shot
    // expression the user types in the panel, and the failure path
    // surfaces the engine's error verbatim.
    let registry = default_registry();
    let evaluator = Evaluator::new(registry);
    let value = evaluator
        .eval_extraction(&expr, &scope)
        .map_err(|e| format!("eval: {e}"))?;
    serde_json::to_value(value.into_json()).map_err(|e| format!("encode: {e}"))
}

/// Read the uncapped response body from disk for one `(run_id, step)`
/// pair. The engine's `step_response_full_body` hook wrote it; this
/// command reads it back when the user clicks "Load full" on a
/// truncated response in the debugger.
///
/// Both arguments are validated against their grammar at the
/// path-build site (`run_artifacts::full_body_path`) so a malformed
/// value can't escape the workspace.
#[tauri::command]
pub fn load_full_step_body(
    state: State<'_, crate::state::StudioState>,
    run_id: String,
    step_name: String,
) -> Result<String, String> {
    let ws = require_workspace(&state)?;
    let path = crate::run_artifacts::full_body_path(&ws.root, &run_id, &step_name)?;
    let bytes = std::fs::read(&path).map_err(|e| format!("read full body: {e}"))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Open (or focus) the pop-out Response viewer. The window has a
/// fixed label `response-viewer` so a second call focuses the
/// existing window rather than spawning a duplicate. The window's
/// React entry (`response.tsx`) subscribes to the same event flow
/// as the main window — `RUN_STEP_RESPONSE_EVENT` and
/// `DEBUG_PAUSED_EVENT` for additive state,
/// `RUN_DEBUG_RESUMED_EVENT` / `RUN_BEGIN_EVENT` /
/// `workspace-closed` for subtractive state.
#[tauri::command]
pub async fn open_response_window(app: AppHandle) -> Result<(), String> {
    if let Some(existing) = app.get_webview_window("response-viewer") {
        let _ = existing.set_focus();
        return Ok(());
    }
    WebviewWindowBuilder::new(
        &app,
        "response-viewer",
        tauri::WebviewUrl::App("response.html".into()),
    )
    .title("Response viewer")
    .inner_size(900.0, 640.0)
    .build()
    .map_err(|e| format!("open response window: {e}"))?;
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
    let signatures = build_signatures(&ws.root);
    if validate(&recipe, &catalog, &signatures).has_errors() {
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

/// Dry-run preview of a `publish_recipe` call: assemble the publish
/// plan off-disk and report what would be sent without POSTing. The
/// preview surfaces the per-type publishes that ride alongside the
/// recipe; the hub publish slug remains the recipe header name (see
/// `publish_recipe`).
#[tauri::command]
pub fn preview_publish(
    state: State<'_, crate::state::StudioState>,
    author: String,
    name: String,
    description: String,
    category: String,
    tags: Vec<String>,
) -> Result<crate::hub_sync::PublishPreview, crate::hub_sync::PublishError> {
    let ws = require_workspace(&state).map_err(|e| crate::hub_sync::PublishError::Other {
        message: e,
    })?;
    crate::hub_sync::preview_publish(&ws, &author, &name, description, category, tags)
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
/// and reveal the paused statement without a hand-rolled TS regex.
/// Returns an empty outline on parse failure — the editor falls back to
/// "no pause points visible until the source parses" rather than
/// guessing at half-baked syntax.
#[tauri::command]
pub fn recipe_outline(source: String) -> RecipeOutline {
    let Ok(recipe) = parse(&source) else {
        return RecipeOutline::default();
    };
    let line_map = LineMap::new(&source);
    let mut pause_points = Vec::new();
    collect_pause_points(recipe.body.statements(), &line_map, &mut pause_points);
    RecipeOutline { pause_points }
}

fn collect_pause_points(
    body: &[forage_core::ast::Statement],
    line_map: &LineMap,
    out: &mut Vec<PausePoint>,
) {
    use forage_core::ast::Statement;
    for s in body {
        match s {
            Statement::Step(step) => {
                let r = line_map.range(step.span.clone());
                out.push(PausePoint::Step {
                    name: step.name.clone(),
                    start_line: r.start.line,
                    start_col: r.start.character,
                    end_line: r.end.line,
                    end_col: r.end.character,
                });
            }
            Statement::Emit(em) => {
                let r = line_map.range(em.span.clone());
                out.push(PausePoint::Emit {
                    type_name: em.type_name.clone(),
                    start_line: r.start.line,
                    start_col: r.start.character,
                    end_line: r.end.line,
                    end_col: r.end.character,
                });
            }
            Statement::ForLoop {
                variable,
                body,
                span,
                ..
            } => {
                let r = line_map.range(span.clone());
                out.push(PausePoint::For {
                    variable: variable.clone(),
                    start_line: r.start.line,
                    start_col: r.start.character,
                    end_line: r.end.line,
                    end_col: r.end.character,
                });
                collect_pause_points(body, line_map, out);
            }
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
            let signatures = build_signatures(workspace_root);
            let report = validate(&r, &catalog, &signatures);
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
            let signatures = forage_core::RecipeSignatures::default();
            let report = validate(&r, &catalog, &signatures);
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

/// Recipe-signature lookup for composition validation. Lonely-recipe
/// mode (no workspace marker) yields an empty map — composition stage
/// references then surface as `UnknownComposeStage` against an empty
/// catalog, which is the correct diagnostic for that case.
fn build_signatures(workspace_root: &Path) -> forage_core::RecipeSignatures {
    forage_core::workspace::discover(workspace_root)
        .map(|ws| ws.recipe_signatures())
        .unwrap_or_default()
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
    // Any non-root `.forage` location is a sidecar — classify_file
    // tags it `Other`; validate_path agrees so the UI doesn't silently
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
    flags: Option<forage_daemon::RunFlags>,
) -> Result<ScheduledRun, String> {
    let daemon = require_daemon(&state)?;
    // Deployment-view "Run now" passes `None` to mean "production
    // trigger" — same shape the scheduler fires. Studio's editor "Run"
    // button goes through `run_recipe`, which builds the dev preset
    // itself. The optional `flags` argument lets a future call site
    // (e.g. a record-button or a sampling toggle next to the run list)
    // override without forking the command.
    let flags = flags.unwrap_or_else(forage_daemon::RunFlags::prod);
    daemon
        .trigger_run(&run_id, flags)
        .await
        .map_err(|e| e.to_string())
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

/// Project the scheduled-run's persisted records into a JSON-LD
/// document using the recipe's type alignments. Studio's run drawer
/// calls this when the format toggle is set to JSON-LD; the daemon
/// owns the catalog (and thus the alignment metadata), so the
/// conversion lives there rather than being re-derived in TypeScript.
#[tauri::command]
pub fn load_run_jsonld(
    state: State<'_, StudioState>,
    scheduled_run_id: String,
) -> Result<forage_core::JsonLdDocument, String> {
    let daemon = require_daemon(&state)?;
    let snapshot = daemon
        .load_run_snapshot(&scheduled_run_id)
        .map_err(|e| e.to_string())?;
    Ok(snapshot.to_jsonld())
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
    let signatures = ws.recipe_signatures();
    daemon
        .deploy(recipe_name, source, wire, &signatures)
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

// --- Notebook commands ----------------------------------------------------
//
// The notebook is a third Studio view that composes deployed recipes
// into a linear pipeline. Three commands cover the lifecycle:
//
// - `notebook_run` walks the chain through `Daemon::run_composition`
//   without persisting; the returned snapshot drives the inspector.
// - `notebook_compose_source` renders the synthetic `.forage` source
//   for a `(name, stages)` pair so the UI can preview / diff the
//   recipe a publish would create.
// - `notebook_save` writes that source as a workspace recipe file, at
//   which point the existing `publish_recipe` / deploy / scheduled-run
//   surfaces all work against it — the notebook publishes *as a
//   recipe*; there is no separate "notebook" hub citizen.

#[tauri::command]
pub async fn notebook_run(
    state: State<'_, StudioState>,
    name: String,
    stages: Vec<String>,
    flags: Option<RunRecipeFlags>,
) -> Result<RunOutcome, String> {
    let raw_flags = flags.unwrap_or(RunRecipeFlags {
        sample_limit: None,
        replay: None,
        ephemeral: None,
    });
    let (sample_limit, replay, _ephemeral) = raw_flags.resolve();
    // The notebook treats every run as ephemeral. The `ephemeral`
    // toggle on the toolbar still drives the inspector layout (and the
    // backend logs), but the daemon never writes notebook records to a
    // persistent store — that's reserved for "Publish notebook" + the
    // resulting deployed recipe.
    tracing::info!(
        notebook = %name,
        stage_count = stages.len(),
        sample_limit = ?sample_limit,
        replay,
        "notebook_run",
    );
    if stages.is_empty() {
        return Ok(RunOutcome {
            ok: false,
            snapshot: None,
            error: Some("notebook has no stages".into()),
            daemon_warning: None,
        });
    }
    let daemon = require_daemon(&state)?;

    // Replay reads each stage's existing `_fixtures/<stage>.jsonl`. A
    // composed pipeline doesn't have a fixture of its own — the
    // composition runtime reuses the per-stage fixtures, fed through
    // the shared replay path. Picking stage 1's fixture (if any) is
    // the closest analogue to the editor's recipe-keyed lookup.
    let replay_path = if replay {
        let ws = require_workspace(&state)?;
        let stage1 = stages.first().expect("non-empty stages checked above");
        let p = forage_core::workspace::fixtures_path(&ws.root, stage1);
        if p.exists() { Some(p) } else { None }
    } else {
        None
    };

    let run_flags = forage_daemon::RunFlags {
        sample_limit,
        replay: replay_path,
        ephemeral: true,
    };

    match daemon
        .run_composition(&name, stages, IndexMap::new(), run_flags)
        .await
    {
        Ok(snapshot) => Ok(RunOutcome {
            ok: true,
            snapshot: Some(snapshot),
            error: None,
            daemon_warning: None,
        }),
        Err(e) => Ok(RunOutcome {
            ok: false,
            snapshot: None,
            error: Some(format!("{e}")),
            daemon_warning: None,
        }),
    }
}

/// Render the `.forage` source a notebook would publish. Pure
/// function — takes the recipe header name, the ordered stage
/// names, and the tail stage's output type, and returns a
/// parseable composition recipe. The frontend uses this to preview
/// the publish payload and to write the synthetic file via
/// `notebook_save`.
///
/// `output_type` is the tail stage's output type name. It rides
/// onto the synthesized recipe as `emits T` so the validator can
/// check the composition's chain and the daemon can build an output-
/// store schema. The frontend already knows this from the picker's
/// `RecipeSignatureWire`; passing `None` synthesizes a recipe with
/// no `emits` clause that runs ephemerally but can't persist records
/// (the daemon's `derive_schema` returns no tables for it).
#[tauri::command]
pub fn notebook_compose_source(
    name: String,
    stages: Vec<String>,
    output_type: Option<String>,
) -> String {
    render_composition_source(&name, &stages, output_type.as_deref())
}

/// Persist a notebook as a `.forage` recipe file at the workspace
/// root. The `name` is the recipe header name and also the file
/// stem (`<workspace>/<name>.forage`). On success the file lands in
/// the workspace's normal recipe set — the publish flow, the deploy
/// flow, and the editor view all see it like any other recipe.
///
/// Refuses to overwrite an existing recipe file: the user picks a
/// fresh name (or deletes the prior one) so a notebook save can't
/// silently clobber an authored recipe.
#[tauri::command]
pub fn notebook_save(
    state: State<'_, StudioState>,
    name: String,
    stages: Vec<String>,
    output_type: Option<String>,
) -> Result<NotebookSaveOutcome, String> {
    if stages.is_empty() {
        return Err("notebook has no stages".into());
    }
    let ws = require_workspace(&state)?;
    let source = render_composition_source(&name, &stages, output_type.as_deref());
    let target = ws.root.join(format!("{name}.forage"));
    if target.exists() {
        return Err(format!(
            "{} already exists; pick a different notebook name or delete the existing recipe first",
            target.display()
        ));
    }
    std::fs::write(&target, &source).map_err(|e| format!("write {}: {e}", target.display()))?;
    Ok(NotebookSaveOutcome {
        path: target,
        source,
    })
}

/// Recipe-shape catalog the notebook's picker reads to filter by
/// output type. Workspace-only — the hub equivalent is
/// `discover_hub_recipes_by_output`.
#[tauri::command]
pub fn list_workspace_recipe_signatures(
    state: State<'_, StudioState>,
) -> Result<Vec<RecipeSignatureWire>, String> {
    let ws = require_workspace(&state)?;
    let signatures = ws.recipe_signatures();
    let mut out: Vec<RecipeSignatureWire> = signatures
        .iter()
        .map(|(name, sig)| RecipeSignatureWire {
            name: name.clone(),
            inputs: sig
                .inputs
                .iter()
                .map(|i| InputSlotWire {
                    name: i.name.clone(),
                    ty: render_field_type(&i.ty),
                    optional: i.optional,
                })
                .collect(),
            // Chain-resolved: a composition recipe without a declared
            // `emits` clause reports its terminal stage's output here
            // so the notebook picker's "produces T" filter finds it.
            outputs: signatures
                .resolve_output_types(name)
                .into_iter()
                .collect(),
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// Parse one recipe source and return its typed-shape projection.
/// The notebook picker calls this per hub package: every fetched
/// `PackageVersion.recipe` rides through here so the picker can
/// render input/output types without re-implementing the parser in
/// TypeScript. `None` when the source fails to parse; the caller
/// falls back to listing the recipe without typed metadata.
#[tauri::command]
pub fn parse_recipe_signature(source: String) -> Option<RecipeSignatureWire> {
    let parsed = parse(&source).ok()?;
    let name = parsed.recipe_name()?.to_string();
    Some(RecipeSignatureWire {
        name,
        inputs: parsed
            .inputs
            .iter()
            .map(|i| InputSlotWire {
                name: i.name.clone(),
                ty: render_field_type(&i.ty),
                optional: i.optional,
            })
            .collect(),
        outputs: parsed.resolved_output_types().into_iter().collect(),
    })
}

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct RecipeSignatureWire {
    pub name: String,
    pub inputs: Vec<InputSlotWire>,
    /// Types this recipe emits, in alphabetical order. Resolved from
    /// `emits T | U | …` when the source declares one, otherwise
    /// inferred from the body's `emit X { … }` statements. Empty for
    /// composition recipes that don't declare `emits` (the chain's
    /// final stage carries the actual types).
    pub outputs: Vec<String>,
}

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct InputSlotWire {
    pub name: String,
    /// Rendered shape: `Product`, `[Product]`, `Product?`. The
    /// picker matches stage N's output against stage N+1's input by
    /// this string; the rendering matches the language so the user
    /// reads the same shape in the picker that they'd write in a
    /// recipe.
    pub ty: String,
    pub optional: bool,
}

fn render_field_type(ty: &forage_core::ast::FieldType) -> String {
    use forage_core::ast::FieldType;
    match ty {
        FieldType::String => "String".into(),
        FieldType::Int => "Int".into(),
        FieldType::Double => "Double".into(),
        FieldType::Bool => "Bool".into(),
        FieldType::Array(inner) => format!("[{}]", render_field_type(inner)),
        FieldType::Record(name) => name.clone(),
        FieldType::EnumRef(name) => name.clone(),
        FieldType::Ref(name) => format!("Ref<{name}>"),
    }
}

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct NotebookSaveOutcome {
    /// Absolute path of the recipe file that was written. The
    /// frontend uses this to switch the editor view onto the new
    /// recipe so the user can inspect / edit before publishing.
    #[ts(type = "string")]
    pub path: PathBuf,
    /// The synthesized recipe source — same value
    /// `notebook_compose_source` would have returned. The frontend
    /// renders this in the publish-preview pane without re-fetching.
    pub source: String,
}

fn render_composition_source(
    name: &str,
    stages: &[String],
    output_type: Option<&str>,
) -> String {
    let mut body = String::new();
    body.push_str("recipe \"");
    body.push_str(name);
    body.push_str("\"\n");
    body.push_str("engine http\n");
    if let Some(t) = output_type {
        body.push_str("emits ");
        body.push_str(t);
        body.push('\n');
    }
    body.push('\n');
    if !stages.is_empty() {
        body.push_str("compose ");
        for (i, stage) in stages.iter().enumerate() {
            if i > 0 {
                body.push_str(" | ");
            }
            body.push('"');
            body.push_str(stage);
            body.push('"');
        }
        body.push('\n');
    }
    body
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

    use forage_daemon::{Cadence, Daemon, Outcome, RunConfig, RunFlags, Trigger};
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
        let signatures = workspace.recipe_signatures();
        daemon
            .deploy(name, source, wire, &signatures)
            .expect("deploy");
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
            output_format: forage_daemon::OutputFormat::default(),
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
            output_format: forage_daemon::OutputFormat::default(),
        };
        let run = daemon.configure_run(slug, cfg).expect("configure_run");
        deploy_from_disk(&daemon, &ws_root, slug);

        let sr = daemon
            .trigger_run(&run.id, RunFlags::prod())
            .await
            .expect("trigger_run");
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
            output_format: forage_daemon::OutputFormat::default(),
        };
        daemon.configure_run(slug, cfg).expect("configure_run");

        // The same lookup `run_recipe` performs before calling the
        // engine. The configured row is authoritative for inputs;
        // there's no file-system fallback.
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
    /// The join uses the recipe header name on both sides, so the
    /// file basename is irrelevant to the pairing.
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
        let signatures = workspace.recipe_signatures();
        daemon
            .deploy(
                "bar",
                "recipe \"bar\"\nengine http\n".to_string(),
                wire,
                &signatures,
            )
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
    /// recipe-name-keyed wire shape. The resolver, the source read,
    /// the deploy, the scheduled-run trigger, and the recipe-name
    /// stamp on the resulting record all key on `"bar"`, never on
    /// `"foo"`.
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
        // helper has to consult `Workspace::recipe_by_name`; falling
        // back to the file basename would resolve `"bar"` to nothing
        // when the file is named `foo.forage`. `Workspace::load`
        // canonicalizes, so compare canonical paths — the test
        // fixture's path may carry a `/private` prefix on macOS.
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
        let signatures = ws.recipe_signatures();
        daemon
            .deploy("bar", source, wire, &signatures)
            .expect("deploy");

        let cfg = RunConfig {
            cadence: Cadence::Manual,
            output: ws_root.join(".forage").join("data").join("bar.sqlite"),
            enabled: true,
            inputs: indexmap::IndexMap::new(),
            output_format: forage_daemon::OutputFormat::default(),
        };
        let run = daemon.configure_run("bar", cfg).expect("configure_run");
        assert_eq!(run.recipe_name, "bar");

        let sr = daemon
            .trigger_run(&run.id, RunFlags::prod())
            .await
            .expect("trigger_run");
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
    /// silently treating it as a declarations file.
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

    // --- Notebook --------------------------------------------------------
    //
    // The notebook commands are thin delegations to `Daemon::run_composition`
    // and to the local `render_composition_source` helper. The unit tests
    // here pin the helper's output shape (`compose "a" | "b"`) and assert
    // that the rendered source round-trips: parse + validate + deploy + run
    // produces the same snapshot as `Daemon::run_composition` against the
    // same stages.

    const NOTEBOOK_UPSTREAM: &str = r#"recipe "scrape"
engine http

share type Item { id: String }

emits Item

step list {
    method "GET"
    url    "https://example.test/items"
}

for $i in $list.items[*] {
    emit Item { id ← $i.id }
}
"#;

    const NOTEBOOK_ENRICH: &str = r#"recipe "enrich"
engine http

share type Item { id: String }

input prior: [Item]

emits Item

for $p in $input.prior {
    emit Item { id ← $p.id }
}
"#;

    #[test]
    fn render_composition_source_emits_parseable_compose_recipe() {
        let src = super::render_composition_source(
            "notebook",
            &["scrape".to_string(), "enrich".to_string()],
            Some("Item"),
        );
        assert!(src.contains("recipe \"notebook\""));
        assert!(src.contains("engine http"));
        assert!(src.contains("emits Item"));
        assert!(src.contains("compose \"scrape\" | \"enrich\""));

        // The synthesized source must parse cleanly — that's the
        // contract publish relies on. The validator needs the peer
        // signatures (the workspace's other recipes), so we don't
        // run it here; the round-trip test below does.
        let parsed = forage_core::parse(&src).expect("parses");
        assert_eq!(parsed.recipe_name(), Some("notebook"));
        let comp = parsed.body.composition().expect("composition body");
        assert_eq!(comp.stages.len(), 2);
        assert_eq!(comp.stages[0].name, "scrape");
        assert!(comp.stages[0].author.is_none());
        assert_eq!(comp.stages[1].name, "enrich");
        let emits = parsed.emits.as_ref().expect("emits decl");
        assert_eq!(emits.types, vec!["Item".to_string()]);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn published_notebook_runs_identically_to_run_composition() {
        let tmp = tempfile::tempdir().unwrap();
        let ws_root = tmp.path().to_path_buf();

        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [{"id": "a"}, {"id": "b"}],
            })))
            .mount(&mock)
            .await;

        write_workspace(&ws_root, "scrape", NOTEBOOK_UPSTREAM);
        std::fs::write(ws_root.join("enrich.forage"), NOTEBOOK_ENRICH).unwrap();
        let scrape_path = ws_root.join("scrape.forage");
        rewrite_url(&scrape_path, &format!("{}/items", mock.uri()));

        let daemon = Daemon::open(ws_root.clone()).expect("open daemon");
        deploy_from_disk(&daemon, &ws_root, "scrape");
        deploy_from_disk(&daemon, &ws_root, "enrich");

        // Notebook preview: run via `run_composition` directly.
        let stages = vec!["scrape".to_string(), "enrich".to_string()];
        let preview = daemon
            .run_composition(
                "notebook",
                stages.clone(),
                indexmap::IndexMap::new(),
                RunFlags::prod(),
            )
            .await
            .expect("run_composition");

        // Publish: render the same chain, write it as a recipe,
        // deploy through the normal flow, trigger it. The published
        // run's snapshot must match the preview's record set.
        let published_src =
            super::render_composition_source("notebook", &stages, Some("Item"));
        std::fs::write(ws_root.join("notebook.forage"), &published_src).unwrap();
        deploy_from_disk(&daemon, &ws_root, "notebook");
        let cfg = RunConfig {
            cadence: Cadence::Manual,
            output: ws_root.join(".forage").join("data").join("notebook.sqlite"),
            enabled: true,
            inputs: indexmap::IndexMap::new(),
            output_format: forage_daemon::OutputFormat::default(),
        };
        let run = daemon.configure_run("notebook", cfg).expect("configure_run");
        let sr = daemon
            .trigger_run(&run.id, RunFlags::prod())
            .await
            .expect("trigger_run");
        assert_eq!(sr.outcome, Outcome::Ok, "stall: {:?}", sr.stall);

        // The trigger persisted records under the published recipe's
        // output store; load them back and compare against the
        // preview snapshot's record ids.
        let records = daemon
            .load_records(&sr.id, "Item", 100)
            .expect("load records");
        let published_ids: Vec<String> = records
            .iter()
            .map(|r| {
                r.get("id")
                    .and_then(|v| v.as_str())
                    .map(String::from)
                    .expect("id field")
            })
            .collect();
        let preview_ids: Vec<String> = preview
            .records
            .iter()
            .map(|r| match r.fields.get("id") {
                Some(forage_core::ast::JSONValue::String(s)) => s.clone(),
                other => panic!("expected String id, got {other:?}"),
            })
            .collect();
        assert_eq!(preview_ids, published_ids);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn run_composition_passes_replay_path_through_flags() {
        // Replay-path threading: feed `RunFlags::replay = Some(path)`
        // and assert the engine reaches the replay transport (no HTTP
        // request is made). A live mock would catch any request that
        // leaks through; the absence of a live mock plus a successful
        // run means the replay path won.
        let tmp = tempfile::tempdir().unwrap();
        let ws_root = tmp.path().to_path_buf();

        // Captures file: one GET to /items returning two items.
        let captures_dir = ws_root.join("_fixtures");
        std::fs::create_dir_all(&captures_dir).unwrap();
        let captures_path = captures_dir.join("scrape.jsonl");
        let captures_jsonl = format!(
            "{}\n",
            serde_json::json!({
                "kind": "http",
                "url": "https://example.test/items",
                "method": "GET",
                "status": 200,
                "body": "{\"items\":[{\"id\":\"x\"},{\"id\":\"y\"}]}"
            })
        );
        std::fs::write(&captures_path, captures_jsonl).unwrap();

        write_workspace(&ws_root, "scrape", NOTEBOOK_UPSTREAM);
        std::fs::write(ws_root.join("enrich.forage"), NOTEBOOK_ENRICH).unwrap();

        let daemon = Daemon::open(ws_root.clone()).expect("open daemon");
        deploy_from_disk(&daemon, &ws_root, "scrape");
        deploy_from_disk(&daemon, &ws_root, "enrich");

        let flags = RunFlags {
            sample_limit: None,
            replay: Some(captures_path),
            ephemeral: true,
        };
        let snapshot = daemon
            .run_composition(
                "notebook",
                vec!["scrape".into(), "enrich".into()],
                indexmap::IndexMap::new(),
                flags,
            )
            .await
            .expect("run_composition with replay");
        assert_eq!(snapshot.records.len(), 2);
    }
}
