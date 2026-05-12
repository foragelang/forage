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
    #[error("path index {idx} out of bounds for array of length {len}")]
    IndexOutOfBounds { idx: i64, len: usize },
    #[error("wildcard [*] applied to non-array value of kind {kind}")]
    WildcardOnNonArray { kind: &'static str },
    #[error("transform '{name}' not found")]
    UnknownTransform { name: String },
    #[error("transform '{name}': {msg}")]
    TransformError { name: String, msg: String },
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
