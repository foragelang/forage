//! Step debugger hook.
//!
//! Studio's interactive debugger plugs into the engine via this trait. The
//! engine calls `before_step` immediately before each `step <name>` block —
//! after `$page` is bound but before the first request goes out. The host
//! decides whether to actually pause (await user input) or resume right
//! away; the engine doesn't track stepping state itself.
//!
//! Symmetric with `ProgressSink`: progress is fire-and-forget, debug is
//! request/response (engine awaits a `ResumeAction`).

use async_trait::async_trait;
use indexmap::IndexMap;
use serde::Serialize;
use ts_rs::TS;

use forage_core::ast::ParseFormat;
use forage_core::{EvalValue, Scope};

/// What the engine does after a debug pause.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ResumeAction {
    /// Run uninterrupted to the next breakpoint or end-of-recipe. Equivalent
    /// to no debugger from the engine's perspective.
    Continue,
    /// Pause again at the next pause-able statement at the same scope
    /// level. The host decides what "next" means — the engine just
    /// resumes; the host's pause hook re-pauses on the next site.
    StepOver,
    /// Pause on the first body statement of the next scope-introducing
    /// construct (currently for-loops). At a non-scope-introducing
    /// pause site this is equivalent to `StepOver`. As with
    /// `StepOver`, the engine just resumes; the host's pause hook
    /// decides where to re-pause.
    StepIn,
    /// Abort the run with a "stopped by debugger" error.
    Stop,
}

/// Snapshot of evaluation state at a pause point, JSON-friendly.
///
/// Secrets never leave the host process: we send the names only so the UI
/// can list them without leaking values into the renderer or any later
/// devtools capture.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct DebugScope {
    /// Named bindings, outer-most frame first, inner frames last (same
    /// order they were pushed). Inner-frame bindings shadow outer ones at
    /// lookup time — the UI is responsible for honoring that visually if
    /// it wants to.
    pub bindings: Vec<DebugFrame>,
    /// Recipe inputs.
    #[ts(type = "Record<string, unknown>")]
    pub inputs: IndexMap<String, serde_json::Value>,
    /// Secret *names* declared by the recipe. Values are never serialized.
    pub secrets: Vec<String>,
    /// Bare `$` current value, if any.
    #[ts(type = "unknown | null")]
    pub current: Option<serde_json::Value>,
    /// Per-type emit counts so far (cumulative).
    #[ts(type = "Record<string, number>")]
    pub emit_counts: IndexMap<String, usize>,
    /// One `StepResponse` per executed step (by step name). The
    /// recipe-side `$<stepname>` binding stays in `bindings` as the
    /// parsed value; this map carries the raw bytes + headers +
    /// resolved format the debugger UI needs to render the response
    /// inspector. Populated incrementally as steps run.
    pub step_responses: IndexMap<String, StepResponse>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct DebugFrame {
    #[ts(type = "Record<string, unknown>")]
    pub bindings: IndexMap<String, serde_json::Value>,
}

impl DebugScope {
    /// Capture the scope as JSON. Large values are passed through verbatim;
    /// truncation/clipping is the UI's responsibility — the host already
    /// has the values in memory and we want lossless inspection.
    pub fn from_scope(
        scope: &Scope,
        secret_names: &[String],
        emit_counts: &IndexMap<String, usize>,
        step_responses: &IndexMap<String, StepResponse>,
    ) -> Self {
        let bindings = scope
            .frames()
            .iter()
            .map(|frame| DebugFrame {
                bindings: frame
                    .iter()
                    .map(|(k, v)| (k.clone(), eval_to_json(v)))
                    .collect(),
            })
            .collect();
        let inputs = scope
            .inputs()
            .iter()
            .map(|(k, v)| (k.clone(), eval_to_json(v)))
            .collect();
        let current = scope.current.as_ref().map(eval_to_json);
        Self {
            bindings,
            inputs,
            secrets: secret_names.to_vec(),
            current,
            emit_counts: emit_counts.clone(),
            step_responses: step_responses.clone(),
        }
    }
}

/// Maximum bytes of response body the debugger carries in `body_raw`.
/// Larger responses get truncated to this length with `body_truncated`
/// set. The recipe-side `$<stepname>` binding still sees the full
/// parsed value — only the debugger's raw-bytes view is capped.
pub const BODY_CAPTURE_MAX: usize = 1024 * 1024;

/// Captured response metadata for one executed step. Lives on
/// `DebugScope::step_responses` keyed by step name so the debug panel's
/// Response column can render Tree / Raw / Headers views.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct StepResponse {
    pub status: u16,
    #[ts(type = "Record<string, string>")]
    pub headers: IndexMap<String, String>,
    /// UTF-8 (lossy) body bytes, truncated to `BODY_CAPTURE_MAX`.
    pub body_raw: String,
    /// True when `body_raw` is shorter than the actual response body
    /// because of the size cap. The UI surfaces this with a "load
    /// full" affordance.
    pub body_truncated: bool,
    /// Resolved parse format (override > content-type > default).
    pub format: ParseFormat,
    /// Raw `Content-Type` header value, lower-cased, with `; charset=`
    /// stripped. `None` when the response carried no `Content-Type`.
    pub content_type_header: Option<String>,
}

fn eval_to_json(v: &EvalValue) -> serde_json::Value {
    serde_json::to_value(v.clone().into_json()).unwrap_or(serde_json::Value::Null)
}

/// Payload the engine sends to the debugger at a step pause.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct StepPause {
    /// Step name (`recipe.body[i].name`).
    pub step: String,
    /// 0-based index of this step in the recipe's flat statement order.
    pub step_index: usize,
    /// 0-based source line of the `step` keyword. Hosts key
    /// breakpoints on this so a click in the editor gutter maps
    /// directly to the engine's pause check.
    pub start_line: u32,
    pub scope: DebugScope,
}

/// Payload the engine sends to the debugger at an `emit` statement.
/// Fired immediately before the record is appended to the snapshot.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct EmitPause {
    /// Record type being emitted (`emit <Type> { … }`).
    pub type_name: String,
    /// 0-based index of this emit in the recipe's flat statement order.
    pub emit_index: usize,
    /// 0-based source line of the `emit` keyword.
    pub start_line: u32,
    pub scope: DebugScope,
}

/// Payload the engine sends to the debugger at a `for`-loop iteration
/// boundary. Fired immediately after `$<variable>` is bound to the
/// current item but before the loop body executes.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct IterationPause {
    /// Loop variable name from `for $<variable> in …`.
    pub variable: String,
    /// 0-based index within the current `for` collection.
    pub iteration: usize,
    /// Total items in the current iteration's collection.
    pub total: usize,
    /// 0-based source line of the `for` keyword.
    pub start_line: u32,
    pub scope: DebugScope,
}

/// Payload the engine sends to the debugger once on `for`-loop entry,
/// after the collection has been evaluated and `total` computed, but
/// before the first iteration body executes. Fires for zero-item
/// loops too so the user can inspect why the collection is empty.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct ForLoopPause {
    /// Loop variable name from `for $<variable> in …`.
    pub variable: String,
    /// Number of items the collection resolved to. Zero is legal and
    /// is precisely the case this pause site exists for.
    pub total: usize,
    /// 0-based source line of the `for` keyword.
    pub start_line: u32,
    pub scope: DebugScope,
}

/// Hook the engine calls at each pause site. A trivial impl that returns
/// `Continue` immediately is equivalent to running without a debugger.
///
/// `before_step` fires before each `step <name>` block; `before_emit`
/// fires before each `emit <Type> { … }` record append; `before_iteration`
/// fires at the top of each `for` iteration after the loop variable is
/// bound; `before_for_loop` fires once on loop entry (after the
/// collection has been evaluated, before iteration starts). Hosts that
/// don't care about a particular pause site can rely on the default
/// impls, which short-circuit to Continue.
///
/// Each method receives the live `&Scope` alongside the pause payload.
/// The payload carries a redacted `DebugScope` snapshot (suitable for
/// the wire); the live scope lets a host that stays in-process —
/// Studio's `StudioDebugger` — stash a clone for later use by features
/// like watch-expression evaluation. Hosts that only forward the
/// payload over a transport (no in-process evaluator) can ignore the
/// scope parameter.
#[async_trait]
pub trait Debugger: Send + Sync {
    async fn before_step(&self, pause: StepPause, scope: &Scope) -> ResumeAction;

    async fn before_emit(&self, _pause: EmitPause, _scope: &Scope) -> ResumeAction {
        ResumeAction::Continue
    }

    async fn before_iteration(&self, _pause: IterationPause, _scope: &Scope) -> ResumeAction {
        ResumeAction::Continue
    }

    /// Fires once when control enters a `for` block, after the
    /// collection is resolved but before any iteration body. Default
    /// impl returns `Continue` so hosts that don't care about the
    /// loop-entry pause site can ignore it.
    async fn before_for_loop(&self, _pause: ForLoopPause, _scope: &Scope) -> ResumeAction {
        ResumeAction::Continue
    }
}

/// No-op debugger — always `Continue`. Equivalent to no debugger at all
/// but useful as a default placeholder.
pub struct NoopDebugger;

#[async_trait]
impl Debugger for NoopDebugger {
    async fn before_step(&self, _: StepPause, _: &Scope) -> ResumeAction {
        ResumeAction::Continue
    }
}

/// Records each pause and replays a scripted `ResumeAction` sequence. Used
/// by engine tests to assert the debugger fires for every step / emit /
/// iteration with the expected scope shape, and that `Stop` cleanly
/// aborts. Cross-module so the engine tests can drive it directly.
///
/// `script_iterations`, `script_emits`, and `script_for_loops` are test-local
/// conveniences: when false, those pause sites short-circuit to Continue
/// without consuming from the script. Tests that only care about steps
/// can leave them off and let the script drive step-only behavior.
#[cfg(test)]
pub struct RecordingDebugger {
    pub script: std::sync::Mutex<Vec<ResumeAction>>,
    pub seen_steps: std::sync::Mutex<Vec<StepPause>>,
    pub seen_emits: std::sync::Mutex<Vec<EmitPause>>,
    pub seen_iterations: std::sync::Mutex<Vec<IterationPause>>,
    pub seen_for_loops: std::sync::Mutex<Vec<ForLoopPause>>,
    pub script_iterations: bool,
    pub script_emits: bool,
    pub script_for_loops: bool,
}

#[cfg(test)]
impl RecordingDebugger {
    pub fn new(script: Vec<ResumeAction>) -> Self {
        Self {
            script: std::sync::Mutex::new(script),
            seen_steps: std::sync::Mutex::new(Vec::new()),
            seen_emits: std::sync::Mutex::new(Vec::new()),
            seen_iterations: std::sync::Mutex::new(Vec::new()),
            seen_for_loops: std::sync::Mutex::new(Vec::new()),
            script_iterations: false,
            script_emits: false,
            script_for_loops: false,
        }
    }

    pub fn with_iterations(mut self) -> Self {
        self.script_iterations = true;
        self
    }

    pub fn with_emits(mut self) -> Self {
        self.script_emits = true;
        self
    }

    pub fn with_for_loops(mut self) -> Self {
        self.script_for_loops = true;
        self
    }

    fn next(&self) -> ResumeAction {
        let mut s = self.script.lock().unwrap();
        if s.is_empty() {
            ResumeAction::Continue
        } else {
            s.remove(0)
        }
    }
}

#[cfg(test)]
#[async_trait]
impl Debugger for RecordingDebugger {
    async fn before_step(&self, pause: StepPause, _scope: &Scope) -> ResumeAction {
        self.seen_steps.lock().unwrap().push(pause);
        self.next()
    }

    async fn before_emit(&self, pause: EmitPause, _scope: &Scope) -> ResumeAction {
        self.seen_emits.lock().unwrap().push(pause);
        if !self.script_emits {
            return ResumeAction::Continue;
        }
        self.next()
    }

    async fn before_iteration(&self, pause: IterationPause, _scope: &Scope) -> ResumeAction {
        self.seen_iterations.lock().unwrap().push(pause);
        if !self.script_iterations {
            return ResumeAction::Continue;
        }
        self.next()
    }

    async fn before_for_loop(&self, pause: ForLoopPause, _scope: &Scope) -> ResumeAction {
        self.seen_for_loops.lock().unwrap().push(pause);
        if !self.script_for_loops {
            return ResumeAction::Continue;
        }
        self.next()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_scope_redacts_secrets() {
        let mut scope = Scope::new();
        let mut inputs = IndexMap::new();
        inputs.insert("term".to_string(), EvalValue::String("OT22".into()));
        scope = scope.with_inputs(inputs);
        let mut secrets = IndexMap::new();
        secrets.insert("password".to_string(), "hunter2".to_string());
        scope = scope.with_secrets(secrets);
        scope.bind("page", EvalValue::Int(3));

        let snap = DebugScope::from_scope(
            &scope,
            &["password".into()],
            &IndexMap::new(),
            &IndexMap::new(),
        );
        assert_eq!(snap.secrets, vec!["password".to_string()]);
        // The redaction is by-omission: no value is ever produced for a
        // secret. The whole point is that "hunter2" must not appear in the
        // serialized form.
        let json = serde_json::to_string(&snap).unwrap();
        assert!(!json.contains("hunter2"), "secret leaked into scope dump");
        assert!(json.contains("\"password\""));
        // Inputs and bindings still serialize.
        assert!(json.contains("OT22"));
        assert!(json.contains("\"page\""));
    }
}
