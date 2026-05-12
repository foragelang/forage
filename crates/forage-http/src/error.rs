//! Errors from the HTTP engine.

use thiserror::Error;

use forage_core::EvalError;

pub type HttpResult<T> = Result<T, HttpError>;

#[derive(Debug, Error)]
pub enum HttpError {
    #[error("eval error: {0}")]
    Eval(#[from] EvalError),
    #[error("HTTP {status} from {url}")]
    Status { status: u16, url: String },
    #[error("transport error: {0}")]
    Transport(String),
    #[error("invalid JSON in response from {url}: {error}")]
    InvalidJson { url: String, error: String },
    #[error("no fixture matches {method} {url}")]
    NoFixture { method: String, url: String },
    #[error("{0}")]
    Generic(String),
}
