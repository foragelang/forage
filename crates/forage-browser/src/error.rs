//! Browser-engine errors.

use thiserror::Error;

use forage_core::EvalError;

pub type BrowserResult<T> = Result<T, BrowserError>;

#[derive(Debug, Error)]
pub enum BrowserError {
    #[error("eval: {0}")]
    Eval(#[from] EvalError),
    #[error("missing browser config — browser-engine recipe requires `browser {{ … }}`")]
    MissingBrowserConfig,
    #[error("regex compile failed: {0}")]
    Regex(String),
    #[error("{0}")]
    Generic(String),
}
