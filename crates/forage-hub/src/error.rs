//! Hub client errors.

use thiserror::Error;

pub type HubResult<T> = Result<T, HubError>;

/// Errors from the hub HTTP surface. `StaleBase` is the typed shape of
/// the server's 409 on publish — Studio displays it differently from
/// a generic API error (it surfaces "you're behind v{latest}; refresh
/// and retry" with a diff link), so the discriminant is part of the
/// public API.
#[derive(Debug, Error)]
pub enum HubError {
    #[error("hub HTTP error {status}: {code} — {message}")]
    Api {
        status: u16,
        code: String,
        message: String,
    },
    /// Server rejected a publish because the caller's `base_version`
    /// doesn't match the current `latest_version`. The caller needs to
    /// rebase against the new latest before retrying.
    #[error(
        "publish rejected: base v{your_base:?} is stale; hub is at v{latest_version}. {message}"
    )]
    StaleBase {
        latest_version: u32,
        your_base: Option<u32>,
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
