//! `HubClient` — thin wrapper over the api.foragelang.com REST surface.
//!
//! The package model is per-version atomic: each version is one
//! indivisible artifact (recipe + decls + fixtures + snapshot +
//! base_version + optional forked_from). The client only knows the wire
//! — operations that materialize a version into a workspace, walk an
//! on-disk recipe back into a `PublishRequest`, etc. live in
//! [`crate::operations`].

use reqwest::{Client, Method};

use crate::error::{HubError, HubResult};
use crate::types::{
    ForkRequest, PackageMetadata, PackageVersion, PublishRequest, PublishResponse, VersionSpec,
};

#[derive(Debug, Clone)]
pub struct HubClient {
    base_url: String,
    bearer_token: Option<String>,
    client: Client,
}

impl HubClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            bearer_token: None,
            client: Client::new(),
        }
    }

    pub fn with_token(mut self, token: impl Into<String>) -> Self {
        self.bearer_token = Some(token.into());
        self
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// `GET /v1/packages/:author/:slug` — package metadata (no version
    /// artifact). Returns `None` on 404 so callers can branch on
    /// "package doesn't exist yet" without parsing the error.
    pub async fn get_package(
        &self,
        author: &str,
        slug: &str,
    ) -> HubResult<Option<PackageMetadata>> {
        let url = format!("{}/v1/packages/{author}/{slug}", self.base_url);
        match self.send(Method::GET, &url, None).await {
            Ok(body) => Ok(Some(serde_json::from_str(&body)?)),
            Err(HubError::Api { status: 404, .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// `GET /v1/packages/:author/:slug/versions/:n` — full version
    /// artifact. `version = VersionSpec::Latest` resolves the most
    /// recent version server-side.
    pub async fn get_version(
        &self,
        author: &str,
        slug: &str,
        version: VersionSpec,
    ) -> HubResult<PackageVersion> {
        let url = format!(
            "{}/v1/packages/{author}/{slug}/versions/{}",
            self.base_url,
            version.as_path_segment()
        );
        let body = self.send(Method::GET, &url, None).await?;
        Ok(serde_json::from_str(&body)?)
    }

    /// `POST /v1/packages/:author/:slug/versions` — publish the atomic
    /// artifact. The server enforces `base_version == latest_version`;
    /// mismatch surfaces as [`HubError::StaleBase`] so callers can
    /// render the rebase prompt without re-parsing the response body.
    pub async fn publish_version(
        &self,
        author: &str,
        slug: &str,
        payload: &PublishRequest,
    ) -> HubResult<PublishResponse> {
        let url = format!("{}/v1/packages/{author}/{slug}/versions", self.base_url);
        let body = serde_json::to_string(payload)?;
        let resp = self.send(Method::POST, &url, Some(body)).await?;
        Ok(serde_json::from_str(&resp)?)
    }

    /// `POST /v1/packages/:author/:slug/fork` — create `@me/<as>` (or
    /// `@me/<upstream-slug>` when `as` is `None`) from the upstream's
    /// latest. Returns the new fork's metadata.
    pub async fn fork(
        &self,
        upstream_author: &str,
        upstream_slug: &str,
        r#as: Option<String>,
    ) -> HubResult<PackageMetadata> {
        let url = format!(
            "{}/v1/packages/{upstream_author}/{upstream_slug}/fork",
            self.base_url
        );
        let req = ForkRequest { r#as };
        let body = serde_json::to_string(&req)?;
        let resp = self.send(Method::POST, &url, Some(body)).await?;
        Ok(serde_json::from_str(&resp)?)
    }

    /// `POST /v1/packages/:author/:slug/downloads` — bump the
    /// informational download counter. Best-effort; failures don't
    /// abort the sync.
    pub async fn record_download(&self, author: &str, slug: &str) -> HubResult<()> {
        let url = format!(
            "{}/v1/packages/{author}/{slug}/downloads",
            self.base_url
        );
        self.send(Method::POST, &url, Some(String::new())).await?;
        Ok(())
    }

    /// `GET /v1/oauth/whoami` — login of the signed-in user, or `None`
    /// when unauthenticated.
    pub async fn whoami(&self) -> HubResult<Option<String>> {
        let url = format!("{}/v1/oauth/whoami", self.base_url);
        let body = match self.send(Method::GET, &url, None).await {
            Ok(b) => b,
            Err(HubError::Api { status: 401, .. }) => return Ok(None),
            Err(e) => return Err(e),
        };
        let v: serde_json::Value = serde_json::from_str(&body)?;
        Ok(v.get("login").and_then(|x| x.as_str()).map(String::from))
    }

    async fn send(
        &self,
        method: Method,
        url: &str,
        body: Option<String>,
    ) -> HubResult<String> {
        let mut req = self.client.request(method, url);
        if let Some(token) = &self.bearer_token {
            req = req.bearer_auth(token);
        }
        if let Some(b) = body {
            req = req.header("Content-Type", "application/json").body(b);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| HubError::Transport(format!("{url}: {e}")))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| HubError::Transport(format!("read body: {e}")))?;
        if status.is_success() {
            return Ok(text);
        }
        // Decode the structured error envelope. The 409 `stale_base`
        // shape carries `latest_version` + `your_base` extras the UI
        // needs to render the rebase prompt — pluck those off here and
        // return the typed variant instead of folding them into the
        // generic Api error.
        let parsed: Option<serde_json::Value> = serde_json::from_str(&text).ok();
        let envelope = parsed.as_ref().and_then(|v| v.get("error"));
        let code = envelope
            .and_then(|e| e.get("code"))
            .and_then(|c| c.as_str())
            .unwrap_or("ERROR")
            .to_string();
        let message = envelope
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
            .unwrap_or(text.as_str())
            .to_string();
        if status.as_u16() == 409 && code == "stale_base" {
            let latest_version = envelope
                .and_then(|e| e.get("latest_version"))
                .and_then(|n| n.as_u64())
                .map(|n| n as u32)
                .unwrap_or(0);
            let your_base = envelope
                .and_then(|e| e.get("your_base"))
                .and_then(|n| n.as_u64())
                .map(|n| n as u32);
            return Err(HubError::StaleBase {
                latest_version,
                your_base,
                message,
            });
        }
        Err(HubError::Api {
            status: status.as_u16(),
            code,
            message,
        })
    }
}
