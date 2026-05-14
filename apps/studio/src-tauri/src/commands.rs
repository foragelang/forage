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
use forage_hub::{AuthStore, AuthTokens, HubClient, PackageFile, PackageMeta};

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
use crate::state::StudioState;
use crate::workspace;

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

#[tauri::command]
pub fn create_recipe() -> Result<String, String> {
    workspace::create_recipe(None).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_recipe(slug: String) -> Result<(), String> {
    workspace::delete_recipe(&slug).map_err(|e| e.to_string())
}

/// Pop up a native context menu (NSMenu on macOS, etc.) at the cursor
/// location with a "Delete recipe…" item. Selection flows back through
/// the global on_menu_event handler, which emits `menu:recipe_delete`
/// with the slug as payload.
#[tauri::command]
pub fn show_recipe_context_menu(
    app: AppHandle,
    window: tauri::WebviewWindow,
    slug: String,
) -> Result<(), String> {
    let id = format!("recipe_delete:{slug}");
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
struct EmitterSink {
    app: AppHandle,
}

impl ProgressSink for EmitterSink {
    fn emit(&self, event: RunEvent) {
        // Emit failures are non-fatal (e.g. window closed mid-run): the run
        // continues even if the UI can't hear it anymore.
        let _ = self.app.emit(RUN_EVENT, &event);
    }
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
    slug: String,
    replay: bool,
) -> Result<RunOutcome, String> {
    let source = workspace::read_source(&slug).map_err(|e| e.to_string())?;
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
    let catalog = match build_catalog_for_slug(&slug, &recipe) {
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
    let raw_inputs = workspace::read_inputs(&slug);
    let mut inputs: IndexMap<String, EvalValue> = IndexMap::new();
    for (k, v) in raw_inputs {
        inputs.insert(k, EvalValue::from(&v));
    }
    let secrets = workspace::read_secrets_from_env(&recipe);
    let captures = if replay {
        workspace::read_captures(&slug)
    } else {
        Vec::new()
    };

    let sink: Arc<dyn ProgressSink> = Arc::new(EmitterSink { app: app.clone() });

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
    let debugger: Option<Arc<dyn Debugger>> = if recipe.engine_kind == EngineKind::Http {
        let session = Arc::new(crate::state::DebugSession::default());
        state.debug_session.store(Some(session.clone()));
        Some(Arc::new(StudioDebugger {
            app: app.clone(),
            session,
        }))
    } else {
        None
    };

    let snapshot: Result<Snapshot, String> = match (recipe.engine_kind, replay) {
        (EngineKind::Http, true) => {
            let transport = ReplayTransport::new(captures);
            let mut engine = Engine::new(&transport).with_progress(sink);
            if let Some(d) = debugger.clone() {
                engine = engine.with_debugger(d);
            }
            tokio::select! {
                biased;
                _ = cancel.notified() => Err("cancelled".into()),
                r = engine.run(&recipe, inputs, secrets) => r.map_err(|e| format!("{e}")),
            }
        }
        (EngineKind::Http, false) => {
            let transport = LiveTransport::new().map_err(|e| format!("{e}"))?;
            let mut engine = Engine::new(&transport).with_progress(sink);
            if let Some(d) = debugger.clone() {
                engine = engine.with_debugger(d);
            }
            tokio::select! {
                biased;
                _ = cancel.notified() => Err("cancelled".into()),
                r = engine.run(&recipe, inputs, secrets) => r.map_err(|e| format!("{e}")),
            }
        }
        (EngineKind::Browser, true) => {
            run_browser_replay(&recipe, &captures, inputs, secrets).map_err(|e| format!("{e}"))
        }
        (EngineKind::Browser, false) => {
            // Open a Tauri WebviewWindow + inject the shim; collect
            // captures; route through the replay engine.
            run_browser_live(&app, &recipe, inputs, secrets, LiveRunOptions::default()).await
        }
    };

    // Clear the cancellation handle so a stale notify can't fire on the
    // next run, and tear down the debug session so the resume path
    // can't wake into a finished engine. Dropping the session's Arc
    // drops any unfilled oneshot sender along with it.
    state.run_cancel.store(None);
    state.debug_session.store(None);

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
            let daemon_warning = match state.daemon.ensure_run(&slug) {
                Ok(_) => None,
                Err(e) => {
                    tracing::warn!(slug = %slug, error = %e, "ensure_run after dev-run failed");
                    Some(format!("daemon bookkeeping failed: {e}"))
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
/// command on slug switch so the engine's hot-path read sees the new
/// recipe's set, then persists the user's edits through the recipe-
/// scoped commands.
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
/// the slug's entry so the sidecar doesn't grow stale.
#[tauri::command]
pub fn set_recipe_breakpoints(
    state: State<'_, crate::state::StudioState>,
    slug: String,
    steps: Vec<String>,
) -> Result<(), String> {
    let mut all = workspace::read_breakpoints().map_err(|e| e.to_string())?;
    if steps.is_empty() {
        all.remove(&slug);
    } else {
        all.insert(slug, steps.clone());
    }
    workspace::write_breakpoints(&all).map_err(|e| e.to_string())?;
    state
        .breakpoints
        .store(Arc::new(steps.into_iter().collect()));
    Ok(())
}

/// Load the persisted breakpoint set for one recipe. Returns an empty
/// vec when the slug has no entry — the absence of breakpoints is the
/// default. A malformed sidecar surfaces as an error so the user sees
/// the parse failure instead of silently losing every breakpoint.
#[tauri::command]
pub fn load_recipe_breakpoints(slug: String) -> Result<Vec<String>, String> {
    let mut map = workspace::read_breakpoints().map_err(|e| e.to_string())?;
    Ok(map.remove(&slug).unwrap_or_default())
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

#[tauri::command]
pub async fn publish_recipe(
    slug: String,
    hub_url: String,
    dry_run: bool,
) -> Result<RunOutcome, String> {
    let source = workspace::read_source(&slug).map_err(|e| e.to_string())?;
    let recipe = parse(&source).map_err(|e| format!("{e}"))?;
    let catalog = build_catalog_for_slug(&slug, &recipe).map_err(|e| format!("{e}"))?;
    if validate(&recipe, &catalog).has_errors() {
        return Err("recipe failed validation".into());
    }
    let store = AuthStore::new();
    let host = host_of(&hub_url);
    let token = store.read(&host).ok().flatten().map(|t| t.access_token);

    if dry_run || token.is_none() {
        return Ok(RunOutcome {
            ok: true,
            snapshot: None,
            error: Some(format!(
                "would publish {} bytes to {hub_url}/v1/packages/{}{}",
                source.len(),
                recipe.name,
                if token.is_none() {
                    " — not signed in"
                } else {
                    ""
                }
            )),
            daemon_warning: None,
        });
    }
    let mut client = HubClient::new(&hub_url);
    if let Some(t) = token {
        client = client.with_token(t);
    }
    let meta = PackageMeta {
        slug: recipe.name.clone(),
        version: 0,
        owner_login: None,
        display_name: Some(recipe.name.clone()),
        summary: None,
        tags: vec![],
        license: None,
        sha256: None,
        published_at: None,
    };
    // Studio publishes single-recipe packages: one `recipe.forage` file
    // in the payload. The full workspace publish path goes through
    // `forage publish` from the CLI.
    let files = vec![PackageFile {
        name: "recipe.forage".into(),
        body: source,
    }];
    match client.publish_package(&recipe.name, files, &meta).await {
        Ok(r) => Ok(RunOutcome {
            ok: true,
            snapshot: None,
            error: Some(format!("published {} v{}", r.slug, r.version)),
            daemon_warning: None,
        }),
        Err(e) => Err(format!("{e}")),
    }
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
pub fn recipe_hover(
    source: String,
    line: u32,
    col: u32,
) -> Option<forage_lsp::intel::HoverInfo> {
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

/// Slug-aware validation: parses the source, builds the workspace
/// catalog (via `build_catalog_for_slug`), and attaches workspace
/// errors as document-level diagnostics. Used by `save_recipe`, which
/// always has a slug context.
fn validate_source_with_slug(slug: &str, source: &str) -> ValidationOutcome {
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
            let catalog = match build_catalog_for_slug(slug, &r) {
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
            let catalog = forage_core::TypeCatalog::from_recipe(&r);
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
        PE::UnexpectedEof { expected } => (0..0, format!("unexpected end of input, expected {expected}")),
        PE::Generic { span, message } => (span.clone(), message.clone()),
        PE::Lex(le) => (0..0, format!("{le}")),
    }
}

/// Type catalog for a Studio recipe identified by slug. If the
/// recipe's directory sits inside a workspace (ancestor
/// `forage.toml`), the catalog folds in workspace declarations files
/// plus cached hub-dep declarations. Otherwise lonely-recipe mode —
/// the recipe's own types only.
///
/// Workspace errors (duplicate types across declarations files, parse
/// failures in a sibling declarations file, etc.) are surfaced to the
/// caller instead of being silently swallowed — the user has to see
/// them to fix them.
fn build_catalog_for_slug(
    slug: &str,
    recipe: &forage_core::Recipe,
) -> Result<forage_core::TypeCatalog, forage_core::workspace::WorkspaceError> {
    let path = workspace::recipe_path(slug);
    if let Some(ws) = forage_core::workspace::discover(&path) {
        return ws.catalog(recipe, |p| std::fs::read_to_string(p));
    }
    Ok(forage_core::TypeCatalog::from_recipe(recipe))
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
// These read the workspace through `state.daemon.workspace()` — the
// daemon owns the single cached `Workspace` and refreshes it on
// filesystem events. Studio doesn't duplicate that state.
// ---------------------------------------------------------------------

use std::path::{Path, PathBuf};

use forage_daemon::{DaemonStatus, Run, RunConfig, ScheduledRun, validate_cron};

use crate::workspace::{FileNode, WorkspaceInfo, build_file_tree};

/// Snapshot of the loaded workspace: root path, manifest's `name`,
/// and `[deps]`. The file list is fetched separately via
/// `list_workspace_files` so polling the tree doesn't redundantly
/// reship the manifest.
#[tauri::command]
pub fn current_workspace(state: State<'_, StudioState>) -> WorkspaceInfo {
    WorkspaceInfo::from_workspace(&state.daemon.workspace())
}

/// Recursive directory tree rooted at the workspace root, with each
/// file classified by [`crate::workspace::FileKind`]. Hidden entries
/// (`.forage/`, dotfiles) are skipped so the tree reflects the user's
/// authored content, not runtime state.
#[tauri::command]
pub fn list_workspace_files(state: State<'_, StudioState>) -> Result<FileNode, String> {
    let root = state.daemon.workspace().root.clone();
    build_file_tree(&root).map_err(|e| e.to_string())
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
    Ok(validate_path(&root, &target, &source))
}

/// Resolve a path that must already exist inside the workspace.
/// Used by `load_file`. `canonicalize` follows symlinks, so a
/// `<workspace>/evil -> /etc/passwd` symlink is caught here
/// rather than being silently dereferenced by `read_to_string`.
fn resolve_existing_in_workspace(
    state: &State<'_, StudioState>,
    path: &Path,
) -> Result<PathBuf, String> {
    let root = state.daemon.workspace().root.clone();
    resolve_existing(&root, path)
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
    let root = state.daemon.workspace().root.clone();
    resolve_new(&root, path)
}

fn workspace_root_canonical(state: &State<'_, StudioState>) -> Result<PathBuf, String> {
    let root = state.daemon.workspace().root.clone();
    canonicalize_root(&root)
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
///   * `<root>/<name>.forage` — declarations file, parse-only.
///   * `<root>/<slug>/recipe.forage` — full recipe validation.
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
    let Some(file_name) = path.file_name().and_then(|s| s.to_str()) else {
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

    // Depth relative to the workspace root: 1 = root-level file
    // (declarations), 2 = `<slug>/<file>.forage` (recipe slot).
    let rel = path.strip_prefix(root).unwrap_or(path);
    let depth = rel.components().count();

    if depth == 1 {
        return validate_declarations_source(source);
    }
    if depth == 2 && file_name == "recipe.forage" {
        // Use the slug-aware validator so workspace-level catalog
        // errors surface. The slug is the recipe's parent directory.
        let slug = path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|s| s.to_str())
            .map(|s| s.to_string())
            .unwrap_or_default();
        return validate_source_with_slug(&slug, source);
    }

    // Any other `.forage` location is a sidecar — neither a
    // declarations file nor a recipe. classify_file tags it
    // `Other`; validate_path must agree.
    let r = LineMap::new(source).range(0..0);
    ValidationOutcome {
        ok: false,
        diagnostics: vec![Diagnostic {
            severity: "error",
            code: "UnrecognizedForageFile".into(),
            message: format!(
                "unrecognized .forage file location: {} — .forage files belong at the workspace root (declarations) or as <slug>/recipe.forage",
                path.display()
            ),
            start_line: r.start.line,
            start_col: r.start.character,
            end_line: r.end.line,
            end_col: r.end.character,
        }],
    }
}

/// Parse-only validation for header-less declarations files. If
/// the source parses, it contributes its types via the workspace
/// catalog; semantic validation against sibling recipes lands
/// when the LSP runs cross-file validation on every edit.
fn validate_declarations_source(source: &str) -> ValidationOutcome {
    match forage_core::parse::parse_workspace_file(source) {
        Ok(_) => ValidationOutcome {
            ok: true,
            diagnostics: Vec::new(),
        },
        Err(e) => {
            let line_map = LineMap::new(source);
            let (span, msg) = parse_error_span(&e);
            let r = line_map.range(span);
            ValidationOutcome {
                ok: false,
                diagnostics: vec![Diagnostic {
                    severity: "error",
                    code: "ParseError".into(),
                    message: msg,
                    start_line: r.start.line,
                    start_col: r.start.character,
                    end_line: r.end.line,
                    end_col: r.end.character,
                }],
            }
        }
    }
}

// --- Daemon commands -------------------------------------------------

#[tauri::command]
pub fn daemon_status(state: State<'_, StudioState>) -> Result<DaemonStatus, String> {
    state.daemon.status().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_runs(state: State<'_, StudioState>) -> Result<Vec<Run>, String> {
    state.daemon.list_runs().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_run(state: State<'_, StudioState>, run_id: String) -> Result<Option<Run>, String> {
    state.daemon.get_run(&run_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn configure_run(
    state: State<'_, StudioState>,
    slug: String,
    cfg: RunConfig,
) -> Result<Run, String> {
    state
        .daemon
        .configure_run(&slug, cfg)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn remove_run(state: State<'_, StudioState>, run_id: String) -> Result<(), String> {
    state.daemon.remove_run(&run_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn trigger_run(
    state: State<'_, StudioState>,
    run_id: String,
) -> Result<ScheduledRun, String> {
    // Clone the Arc — `trigger_run` takes `&Arc<Self>` so we need an
    // owned handle to call across the await.
    let daemon = state.daemon.clone();
    daemon.trigger_run(&run_id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_scheduled_runs(
    state: State<'_, StudioState>,
    run_id: String,
    limit: u32,
    before: Option<i64>,
) -> Result<Vec<ScheduledRun>, String> {
    state
        .daemon
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
    state
        .daemon
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

    fn write_workspace(root: &Path, slug: &str, recipe_source: &str) {
        std::fs::create_dir_all(root.join(slug)).unwrap();
        std::fs::write(root.join("forage.toml"), "").unwrap();
        std::fs::write(root.join(slug).join("recipe.forage"), recipe_source).unwrap();
    }

    fn rewrite_url(path: &Path, url: &str) {
        let src = std::fs::read_to_string(path).unwrap();
        std::fs::write(path, src.replace("https://example.test/items", url)).unwrap();
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
        };
        let created = daemon.configure_run(slug, cfg.clone()).expect("configure_run");

        let listed = daemon.list_runs().expect("list_runs");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, created.id);
        assert_eq!(listed[0].recipe_slug, slug);
        assert!(listed[0].enabled);

        // Repeated configure on the same slug is an update, not an
        // insert — list_runs should still return one row, and the id
        // should be stable.
        let updated_cfg = RunConfig {
            enabled: false,
            ..cfg
        };
        let updated = daemon.configure_run(slug, updated_cfg).expect("configure_run update");
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
        let recipe_path = ws_root.join(slug).join("recipe.forage");
        rewrite_url(&recipe_path, &format!("{}/items", mock.uri()));

        let daemon = Daemon::open(ws_root.clone()).expect("open daemon");
        let cfg = RunConfig {
            cadence: Cadence::Manual,
            output: ws_root.join(".forage").join("data").join("items.sqlite"),
            enabled: true,
        };
        let run = daemon.configure_run(slug, cfg).expect("configure_run");

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
        let err =
            resolve_new(tmp.path(), Path::new("")).expect_err("empty path must be rejected");
        assert!(err.contains("empty path"), "unexpected error: {err}");
    }

    /// A sidecar `.forage` file two-deep that isn't named
    /// `recipe.forage` is unclassified — validate_path must surface
    /// that as a diagnostic instead of silently treating it as a
    /// declarations file (which is what would happen if it slipped
    /// through to `parse_workspace_file`).
    #[test]
    fn validate_path_rejects_sidecar_forage_in_recipe_folder() {
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

    /// Root-level header-less `.forage` files validate as
    /// declarations — only their parse errors are surfaced.
    #[test]
    fn validate_path_treats_root_forage_as_declarations() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let decl = root.join("cannabis.forage");
        std::fs::write(&decl, "type Dispensary { id: String }\n").unwrap();

        let outcome = validate_path(root, &decl, "type Dispensary { id: String }\n");
        assert!(outcome.ok, "declarations file should validate clean: {outcome:?}");
        assert!(outcome.diagnostics.is_empty());
    }
}
