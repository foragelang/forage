//! OAuth 2.0 device-code flow against `api.foragelang.com`.
//!
//! - `start_device(hub)` → `{ device_code, user_code, verification_url, interval, expires_in }`
//! - poll `poll_device(...)` every `interval` seconds until either:
//!     - 200 + tokens (success),
//!     - 202 + status="pending" (keep polling),
//!     - 410 + status="expired" (start over).

use std::time::Duration;

use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::error::{HubError, HubResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceStartResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_url: String,
    pub interval: u64,
    pub expires_in: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DevicePollResponse {
    pub status: String,
    #[serde(default)]
    pub access_token: Option<String>,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub user: Option<DeviceUser>,
    #[serde(default)]
    pub expires_in: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceUser {
    pub login: String,
}

pub async fn start_device(hub_url: &str) -> HubResult<DeviceStartResponse> {
    let client = Client::new();
    let url = format!("{}/v1/oauth/device", hub_url.trim_end_matches('/'));
    let resp = client
        .post(&url)
        .json(&serde_json::json!({}))
        .send()
        .await
        .map_err(|e| HubError::Transport(format!("POST {url}: {e}")))?;
    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| HubError::Transport(format!("read body: {e}")))?;
    if !status.is_success() {
        return Err(HubError::Device(format!(
            "device/start returned {}: {body}",
            status.as_u16()
        )));
    }
    let resp: DeviceStartResponse = serde_json::from_str(&body)?;
    Ok(resp)
}

pub async fn poll_device(hub_url: &str, device_code: &str) -> HubResult<DevicePollResponse> {
    let client = Client::new();
    let url = format!("{}/v1/oauth/device/poll", hub_url.trim_end_matches('/'));
    let resp = client
        .post(&url)
        .json(&serde_json::json!({ "device_code": device_code }))
        .send()
        .await
        .map_err(|e| HubError::Transport(format!("POST {url}: {e}")))?;
    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| HubError::Transport(format!("read body: {e}")))?;
    // 200 = ok, 202 = pending, 410 = expired.
    let resp: DevicePollResponse =
        serde_json::from_str(&body).unwrap_or_else(|_| DevicePollResponse {
            status: format!("http-{}", status.as_u16()),
            access_token: None,
            refresh_token: None,
            user: None,
            expires_in: None,
        });
    Ok(resp)
}

/// Driver loop: start a flow, print the user code + URL, poll until done
/// or expired. Returns the access + refresh tokens on success.
pub async fn run_device_flow(
    hub_url: &str,
    on_user_code: impl FnOnce(&DeviceStartResponse),
) -> HubResult<DevicePollResponse> {
    let start = start_device(hub_url).await?;
    on_user_code(&start);
    let deadline = std::time::Instant::now() + Duration::from_secs(start.expires_in.max(60));
    let interval = Duration::from_secs(start.interval.max(2));
    loop {
        if std::time::Instant::now() > deadline {
            return Err(HubError::Device("device-code flow expired".into()));
        }
        tokio::time::sleep(interval).await;
        let resp = poll_device(hub_url, &start.device_code).await?;
        if resp.status == "ok" && resp.access_token.is_some() {
            return Ok(resp);
        }
        if resp.status == "expired" {
            return Err(HubError::Device("device code expired".into()));
        }
        // pending — keep polling.
    }
}
