//! Forage recipe language core: AST, parser, validator, evaluator, snapshot.
//!
//! This crate has no I/O. It defines what a `.forage` recipe *is* and how
//! it's checked for soundness. Concrete engines (HTTP, browser) and hosts
//! (CLI, Studio, web IDE) build on top.

pub mod ast;
pub mod error;
pub mod eval;
pub mod parse;
pub mod progress;
pub mod snapshot;
pub mod source;
pub mod validate;
pub mod workspace;

pub use ast::{ForageFile, RecipeHeader};
pub use error::{ForageError, ForageResult};
pub use eval::{EvalError, EvalValue, Evaluator, Scope, TransformRegistry, default_registry};
pub use parse::parse;
pub use progress::{ProgressUnit, infer_progress_unit};
pub use snapshot::{
    DiagnosticReport, Record, RecordType, RecordTypeField, RuntimeDiagnostic, Snapshot,
};
pub use source::{LineMap, Position, Range};
pub use validate::{Severity, ValidationCode, ValidationIssue, ValidationReport, validate};
pub use workspace::{
    SerializableCatalog, TypeCatalog, Workspace, WorkspaceError, discover, fixtures_path, load,
    snapshot_path,
};
