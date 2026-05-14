//! Evaluation scope — stack of bindings for nested `for` loops + the
//! recipe's `$input`/`$secret` resolution.

use indexmap::IndexMap;

use crate::eval::value::EvalValue;

#[derive(Debug, Clone)]
pub struct Scope {
    /// Stack of named bindings — most recent at the back. `$current` and
    /// the loop variable live here.
    frames: Vec<IndexMap<String, EvalValue>>,
    inputs: IndexMap<String, EvalValue>,
    secrets: IndexMap<String, String>,
    /// Bare `$` value at the current binding site (e.g. inside a `for`).
    pub current: Option<EvalValue>,
}

impl Default for Scope {
    fn default() -> Self {
        Self {
            frames: vec![IndexMap::new()],
            inputs: IndexMap::new(),
            secrets: IndexMap::new(),
            current: None,
        }
    }
}

impl Scope {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_inputs(mut self, inputs: IndexMap<String, EvalValue>) -> Self {
        self.inputs = inputs;
        self
    }

    pub fn with_secrets(mut self, secrets: IndexMap<String, String>) -> Self {
        self.secrets = secrets;
        self
    }

    pub fn push_frame(&mut self) {
        self.frames.push(IndexMap::new());
    }

    pub fn pop_frame(&mut self) {
        if self.frames.len() > 1 {
            self.frames.pop();
        }
    }

    pub fn bind(&mut self, name: &str, value: EvalValue) {
        self.frames
            .last_mut()
            .expect("at least one frame")
            .insert(name.into(), value);
    }

    pub fn lookup(&self, name: &str) -> Option<&EvalValue> {
        for frame in self.frames.iter().rev() {
            if let Some(v) = frame.get(name) {
                return Some(v);
            }
        }
        None
    }

    pub fn input(&self, name: &str) -> Option<&EvalValue> {
        self.inputs.get(name)
    }

    pub fn inputs(&self) -> &IndexMap<String, EvalValue> {
        &self.inputs
    }

    pub fn secret(&self, name: &str) -> Option<&str> {
        self.secrets.get(name).map(String::as_str)
    }

    /// Full secrets map — used when spawning a child scope for a
    /// user-fn call (the child needs the recipe's secrets but not the
    /// parent's loop bindings).
    pub fn secrets_map(&self) -> &IndexMap<String, String> {
        &self.secrets
    }

    /// All active frames, outer-most first. Used by the debugger to render
    /// the binding stack at a pause point — same iteration order as `bind`
    /// would have produced.
    pub fn frames(&self) -> &[IndexMap<String, EvalValue>] {
        &self.frames
    }
}
