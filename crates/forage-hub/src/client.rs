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
        //
        // A response whose body doesn't decode as the documented
        // envelope (missing `error.code`, missing `error.message`, or
        // — for a 409 stale_base — missing `latest_version`) surfaces
        // as `ServerMalformed` rather than guessing a default; a
        // silent `"ERROR"` / `0` here would lie to the UI and hide a
        // real server bug.
        let parsed: Option<serde_json::Value> = serde_json::from_str(&text).ok();
        let envelope = parsed.as_ref().and_then(|v| v.get("error"));
        let Some(envelope) = envelope else {
            return Err(HubError::ServerMalformed {
                detail: format!(
                    "{url} returned {} but body lacks a top-level `error` object: {text:?}",
                    status.as_u16()
                ),
            });
        };
        let code = match envelope.get("code").and_then(|c| c.as_str()) {
            Some(s) => s.to_string(),
            None => {
                return Err(HubError::ServerMalformed {
                    detail: format!(
                        "{url} returned {} but error envelope lacks `code`: {text:?}",
                        status.as_u16()
                    ),
                });
            }
        };
        let message = match envelope.get("message").and_then(|m| m.as_str()) {
            Some(s) => s.to_string(),
            None => {
                return Err(HubError::ServerMalformed {
                    detail: format!(
                        "{url} returned {} but error envelope lacks `message`: {text:?}",
                        status.as_u16()
                    ),
                });
            }
        };
        if status.as_u16() == 409 && code == "stale_base" {
            let latest_version = match envelope
                .get("latest_version")
                .and_then(|n| n.as_u64())
                .map(|n| n as u32)
            {
                Some(v) => v,
                None => {
                    return Err(HubError::ServerMalformed {
                        detail: format!(
                            "{url} returned 409 stale_base without `latest_version`: {text:?}"
                        ),
                    });
                }
            };
            // `your_base` is genuinely optional — the server returns
            // it when the caller sent a base_version, and omits it
            // for first-publish attempts. Stay with Option<u32> here.
            let your_base = envelope
                .get("your_base")
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// A response body that decodes as JSON but lacks the top-level
    /// `error` object surfaces as `ServerMalformed`. The old code
    /// invented a `code = "ERROR"` and a generic message, which
    /// hid the fact that the server was returning a non-conforming
    /// envelope.
    #[tokio::test]
    async fn malformed_envelope_surfaces_server_malformed() {
        let server = MockServer::start().await;
        // Body is JSON but doesn't match the envelope shape.
        Mock::given(method("GET"))
            .and(path("/v1/packages/x/y"))
            .respond_with(ResponseTemplate::new(500).set_body_json(json!({ "oops": true })))
            .mount(&server)
            .await;

        let client = HubClient::new(server.uri());
        let err = client.get_package("x", "y").await.unwrap_err();
        match err {
            HubError::ServerMalformed { detail } => {
                assert!(detail.contains("error"), "detail must name the missing field: {detail}");
            }
            other => panic!("expected ServerMalformed, got {other:?}"),
        }
    }

    /// A 409 stale_base whose envelope is missing `latest_version`
    /// is a hub bug — surface it as `ServerMalformed`, not as
    /// `StaleBase { latest_version: 0 }` (which would tell the UI
    /// the hub is at v0 and let the regression go silent).
    #[tokio::test]
    async fn stale_base_without_latest_version_surfaces_malformed() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/packages/x/y/versions"))
            .respond_with(ResponseTemplate::new(409).set_body_json(json!({
                "error": {
                    "code": "stale_base",
                    "message": "behind",
                    // latest_version intentionally missing
                }
            })))
            .mount(&server)
            .await;

        let client = HubClient::new(server.uri()).with_token("t");
        let payload = PublishRequest {
            description: "d".into(),
            category: "c".into(),
            tags: vec![],
            recipe: "r".into(),
            decls: vec![],
            fixtures: vec![],
            snapshot: None,
            base_version: Some(1),
        };
        let err = client.publish_version("x", "y", &payload).await.unwrap_err();
        match err {
            HubError::ServerMalformed { detail } => {
                assert!(
                    detail.contains("latest_version"),
                    "detail must name the missing field: {detail}",
                );
            }
            other => panic!("expected ServerMalformed, got {other:?}"),
        }
    }

    /// A well-formed envelope still produces a clean Api / StaleBase
    /// error — the strict decoder doesn't regress the happy path.
    #[tokio::test]
    async fn well_formed_stale_base_still_parses() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/packages/x/y/versions"))
            .respond_with(ResponseTemplate::new(409).set_body_json(json!({
                "error": {
                    "code": "stale_base",
                    "message": "behind",
                    "latest_version": 7,
                    "your_base": 3,
                }
            })))
            .mount(&server)
            .await;

        let client = HubClient::new(server.uri()).with_token("t");
        let payload = PublishRequest {
            description: "d".into(),
            category: "c".into(),
            tags: vec![],
            recipe: "r".into(),
            decls: vec![],
            fixtures: vec![],
            snapshot: None,
            base_version: Some(3),
        };
        let err = client.publish_version("x", "y", &payload).await.unwrap_err();
        match err {
            HubError::StaleBase {
                latest_version,
                your_base,
                ..
            } => {
                assert_eq!(latest_version, 7);
                assert_eq!(your_base, Some(3));
            }
            other => panic!("expected StaleBase, got {other:?}"),
        }
    }
}
