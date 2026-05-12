//! Error types for the Forage core.
//!
//! Filled in during R1.8 (alongside ariadne diagnostic rendering).

use thiserror::Error;

pub type ForageResult<T> = Result<T, ForageError>;

#[derive(Debug, Error)]
pub enum ForageError {
    #[error("parse error: {0}")]
    Parse(String),
    #[error("validation error: {0}")]
    Validate(String),
    #[error("evaluation error: {0}")]
    Eval(String),
}
