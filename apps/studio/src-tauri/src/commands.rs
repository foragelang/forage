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
use forage_hub::{AuthStore, AuthTokens, HubClient, RecipeMeta};

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
use crate::library::{self, RecipeEntry};
use crate::state::StudioState;

#[derive(Serialize, TS)]
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
#[derive(Serialize, TS)]
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

#[tauri::command]
pub fn list_recipes() -> Vec<RecipeEntry> {
    library::list_entries()
}

#[tauri::command]
pub fn load_recipe(slug: String) -> Result<String, String> {
    library::read_source(&slug).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_recipe(slug: String, source: String) -> Result<ValidationOutcome, String> {
    library::write_source(&slug, &source).map_err(|e| e.to_string())?;
    Ok(validate_source(&source))
}

/// Validate a source buffer without touching disk. Studio fires this on
/// every keystroke (debounced) to keep diagnostics live. `save_recipe`
/// still re-validates after writing so editor markers remain accurate
/// even if the user only saves without typing.
#[tauri::command]
pub fn validate_recipe(source: String) -> ValidationOutcome {
    validate_source(&source)
}

#[tauri::command]
pub fn create_recipe() -> Result<String, String> {
    library::create_recipe(None).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_recipe(slug: String) -> Result<(), String> {
    library::delete_recipe(&slug).map_err(|e| e.to_string())
}

/// Pop up a native context menu (NSMenu on macOS, etc.) at the given
/// window-relative position with a "Delete recipe…" item. Selection
/// flows back through the global on_menu_event handler, which emits
/// `menu:recipe_delete` with the slug as payload.
#[tauri::command]
pub fn show_recipe_context_menu(
    app: AppHandle,
    window: tauri::WebviewWindow,
    slug: String,
    x: f64,
    y: f64,
) -> Result<(), String> {
    let id = format!("recipe_delete:{slug}");
    tracing::info!(slug = %slug, id = %id, x, y, "show_recipe_context_menu");
    let delete_item = tauri::menu::MenuItemBuilder::with_id(&id, "Delete Recipe…")
        .build(&app)
        .map_err(|e| e.to_string())?;
    let menu = tauri::menu::MenuBuilder::new(&app)
        .item(&delete_item)
        .build()
        .map_err(|e| e.to_string())?;
    let position = tauri::Position::Logical(tauri::LogicalPosition { x, y });
    window
        .popup_menu_at(&menu, position)
        .map_err(|e| e.to_string())?;
    // The menu must outlive popup_menu_at on macOS — muda's NSMenu wrapper
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
    let source = library::read_source(&slug).map_err(|e| e.to_string())?;
    let recipe = match parse(&source) {
        Ok(r) => r,
        Err(e) => {
            return Ok(RunOutcome {
                ok: false,
                snapshot: None,
                error: Some(format!("{e}")),
            });
        }
    };
    let report = validate(&recipe);
    if report.has_errors() {
        let msgs: Vec<String> = report.errors().map(|e| e.message.clone()).collect();
        return Ok(RunOutcome {
            ok: false,
            snapshot: None,
            error: Some(msgs.join("; ")),
        });
    }
    let raw_inputs = library::read_inputs(&slug);
    let mut inputs: IndexMap<String, EvalValue> = IndexMap::new();
    for (k, v) in raw_inputs {
        inputs.insert(k, EvalValue::from(&v));
    }
    let secrets = library::read_secrets_from_env(&recipe);
    let captures = if replay {
        library::read_captures(&slug)
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
        Ok(s) => Ok(RunOutcome {
            ok: true,
            snapshot: Some(s),
            error: None,
        }),
        Err(e) => Ok(RunOutcome {
            ok: false,
            snapshot: None,
            error: Some(e),
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

/// Persist a recipe's breakpoint set to the library sidecar and push it
/// to the in-memory cache the engine reads on pause. Empty set deletes
/// the slug's entry so the sidecar doesn't grow stale.
#[tauri::command]
pub fn set_recipe_breakpoints(
    state: State<'_, crate::state::StudioState>,
    slug: String,
    steps: Vec<String>,
) -> Result<(), String> {
    let mut all = library::read_breakpoints();
    if steps.is_empty() {
        all.remove(&slug);
    } else {
        all.insert(slug, steps.clone());
    }
    library::write_breakpoints(&all).map_err(|e| e.to_string())?;
    state
        .breakpoints
        .store(Arc::new(steps.into_iter().collect()));
    Ok(())
}

/// Load the persisted breakpoint set for one recipe. Returns an empty
/// vec when the slug has no entry — the absence of breakpoints is the
/// default.
#[tauri::command]
pub fn load_recipe_breakpoints(slug: String) -> Vec<String> {
    library::read_breakpoints()
        .remove(&slug)
        .unwrap_or_default()
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
    let source = library::read_source(&slug).map_err(|e| e.to_string())?;
    let recipe = parse(&source).map_err(|e| format!("{e}"))?;
    if validate(&recipe).has_errors() {
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
                "would publish {} bytes to {hub_url}/v1/recipes/{}{}",
                source.len(),
                recipe.name,
                if token.is_none() {
                    " — not signed in"
                } else {
                    ""
                }
            )),
        });
    }
    let mut client = HubClient::new(&hub_url);
    if let Some(t) = token {
        client = client.with_token(t);
    }
    let meta = RecipeMeta {
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
    match client.publish(&recipe.name, &source, &meta).await {
        Ok(r) => Ok(RunOutcome {
            ok: true,
            snapshot: None,
            error: Some(format!("published {} v{}", r.slug, r.version)),
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
            let report = validate(&r);
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

fn host_of(url: &str) -> String {
    let after_scheme = url.split("//").nth(1).unwrap_or(url);
    after_scheme
        .split('/')
        .next()
        .unwrap_or(after_scheme)
        .to_string()
}

// Quiet `dead_code` on the unused state for now — the dirty buffer cache
// is wired in when we add background autosave.
#[allow(dead_code)]
fn _state_typecheck(_s: State<'_, StudioState>) {}
