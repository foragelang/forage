//! Evaluation errors.

use thiserror::Error;

#[derive(Debug, Clone, Error, PartialEq)]
pub enum EvalError {
    #[error("undefined variable: {0}")]
    UndefinedVariable(String),
    #[error("undefined input: {0}")]
    UndefinedInput(String),
    #[error("undefined secret: {0}")]
    UndefinedSecret(String),
    #[error("path field '{field}' missing on value of kind {kind}")]
    MissingField { field: String, kind: &'static str },
    #[error("wildcard [*] applied to non-array value of kind {kind}")]
    WildcardOnNonArray { kind: &'static str },
    #[error("transform '{name}' not found")]
    UnknownTransform { name: String },
    #[error("transform '{name}': {msg}")]
    TransformError { name: String, msg: String },
    #[error("function '{name}' expects {expected} argument{s_e}, got {got}", s_e = if *expected == 1 { "" } else { "s" })]
    FnArityMismatch {
        name: String,
        expected: usize,
        got: usize,
    },
    #[error("case-of: no branch matched label '{label}'")]
    CaseNoMatch { label: String },
    #[error("type error: expected {expected}, got {actual}")]
    TypeMismatch {
        expected: &'static str,
        actual: &'static str,
    },
    #[error("{0}")]
    Generic(String),
}
