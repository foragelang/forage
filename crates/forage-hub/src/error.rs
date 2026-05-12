//! Hub client errors.

use thiserror::Error;

pub type HubResult<T> = Result<T, HubError>;

#[derive(Debug, Error)]
pub enum HubError {
    #[error("hub HTTP error {status}: {code} — {message}")]
    Api {
        status: u16,
        code: String,
        message: String,
    },
    #[error("hub transport: {0}")]
    Transport(String),
    #[error("hub auth: {0}")]
    Auth(String),
    #[error("hub I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("hub JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("device-code flow: {0}")]
    Device(String),
    #[error("{0}")]
    Generic(String),
}
