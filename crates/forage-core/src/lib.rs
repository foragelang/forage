//! Forage recipe language core: AST, parser, validator, evaluator, snapshot.
//!
//! This crate has no I/O. It defines what a `.forage` recipe *is* and how
//! it's checked for soundness. Concrete engines (HTTP, browser) and hosts
//! (CLI, Studio, web IDE) build on top.

pub mod ast;
pub mod error;
pub mod eval;
pub mod parse;
pub mod snapshot;
pub mod transforms;
pub mod validate;

pub use ast::Recipe;
pub use error::{ForageError, ForageResult};
pub use parse::parse;
