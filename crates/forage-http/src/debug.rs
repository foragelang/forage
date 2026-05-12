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

use forage_core::{EvalValue, Scope};

/// What the engine does after a debug pause.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ResumeAction {
    /// Run uninterrupted to the next breakpoint or end-of-recipe. Equivalent
    /// to no debugger from the engine's perspective.
    Continue,
    /// Pause again at the next step.
    StepOver,
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
        }
    }
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
    pub scope: DebugScope,
}

/// Hook the engine calls at each pause site. A trivial impl that returns
/// `Continue` immediately is equivalent to running without a debugger.
///
/// `before_step` is called before each `step <name>` block.
/// `before_iteration` is called at the top of each `for` loop iteration
/// after the loop variable is bound. Hosts that don't care about
/// iteration-level pausing can rely on the default impl, which
/// short-circuits to Continue.
#[async_trait]
pub trait Debugger: Send + Sync {
    async fn before_step(&self, pause: StepPause) -> ResumeAction;

    async fn before_iteration(&self, _pause: IterationPause) -> ResumeAction {
        ResumeAction::Continue
    }
}

/// No-op debugger — always `Continue`. Equivalent to no debugger at all
/// but useful as a default placeholder.
pub struct NoopDebugger;

#[async_trait]
impl Debugger for NoopDebugger {
    async fn before_step(&self, _: StepPause) -> ResumeAction {
        ResumeAction::Continue
    }
}

/// Records each pause and replays a scripted `ResumeAction` sequence. Used
/// by engine tests to assert the debugger fires for every step + iteration
/// with the expected scope shape, and that `Stop` cleanly aborts. Cross-
/// module so the engine tests can drive it directly.
#[cfg(test)]
pub struct RecordingDebugger {
    pub script: std::sync::Mutex<Vec<ResumeAction>>,
    pub seen_steps: std::sync::Mutex<Vec<StepPause>>,
    pub seen_iterations: std::sync::Mutex<Vec<IterationPause>>,
    pub pause_iterations: bool,
}

#[cfg(test)]
impl RecordingDebugger {
    pub fn new(script: Vec<ResumeAction>) -> Self {
        Self {
            script: std::sync::Mutex::new(script),
            seen_steps: std::sync::Mutex::new(Vec::new()),
            seen_iterations: std::sync::Mutex::new(Vec::new()),
            pause_iterations: false,
        }
    }

    pub fn with_iterations(mut self) -> Self {
        self.pause_iterations = true;
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
    async fn before_step(&self, pause: StepPause) -> ResumeAction {
        self.seen_steps.lock().unwrap().push(pause);
        self.next()
    }

    async fn before_iteration(&self, pause: IterationPause) -> ResumeAction {
        self.seen_iterations.lock().unwrap().push(pause);
        if !self.pause_iterations {
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

        let snap = DebugScope::from_scope(&scope, &["password".into()], &IndexMap::new());
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
