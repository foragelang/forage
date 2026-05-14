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
    /// Arithmetic operation reached a domain edge — division by zero,
    /// modulo by zero. Surfaced as a typed error rather than silently
    /// returning `Infinity` / `NaN`; recipes routinely run unattended
    /// and a silent inf would propagate through downstream computations
    /// undetected.
    #[error("arithmetic: {0}")]
    ArithmeticDomain(String),
    /// Bracket indexing went out of range. Distinct from path-level
    /// `[N]` on a `PathExpr`, which is null-tolerant by design
    /// (scraping records routinely access fields like `$x.range[0]`
    /// against possibly-empty arrays); the expression-level form is
    /// strict so authors who reach for `$captures[5]` after a regex
    /// `match` see a real diagnostic when the group doesn't exist.
    #[error("index {index} out of bounds (length {len})")]
    IndexOutOfBounds { index: i64, len: usize },
    /// Bracket indexing against a non-array value. The path-level form
    /// returns `Null` for null bases; the expression-level form errors
    /// for anything but `Array`.
    #[error("indexing not supported on value of kind {kind}")]
    InvalidIndexBase { kind: &'static str },
    /// Struct literal declared the same field twice — `{ x: 1, x: 2 }`
    /// would silently keep one and drop the other. Surfacing it at
    /// runtime guarantees a diagnostic even when the validator hasn't
    /// run (REPL, ad-hoc eval).
    #[error("duplicate field '{0}' in struct literal")]
    DuplicateStructField(String),
    #[error("{0}")]
    Generic(String),
}
