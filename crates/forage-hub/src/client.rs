//! `HubClient` — thin wrapper over the api.foragelang.com REST surface.

use reqwest::{Client, Method};
use serde::{Deserialize, Serialize};

use crate::error::{HubError, HubResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecipeMeta {
    pub slug: String,
    pub version: u32,
    #[serde(default)]
    pub owner_login: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub sha256: Option<String>,
    #[serde(default)]
    pub published_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecipeBlob {
    pub slug: String,
    pub version: u32,
    pub source: String,
    #[serde(default)]
    pub metadata: Option<RecipeMeta>,
}

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

    pub async fn list(&self, query: Option<&str>) -> HubResult<Vec<RecipeMeta>> {
        let mut url = format!("{}/v1/recipes", self.base_url);
        if let Some(q) = query {
            url.push_str(&format!("?q={}", urlencode(q)));
        }
        let resp = self.send(Method::GET, &url, None).await?;
        let recipes: Vec<RecipeMeta> = serde_json::from_str(&resp)?;
        Ok(recipes)
    }

    pub async fn get(&self, slug: &str, version: Option<u32>) -> HubResult<RecipeBlob> {
        let mut url = format!("{}/v1/recipes/{}", self.base_url, slug);
        if let Some(v) = version {
            url.push_str(&format!("?version={v}"));
        }
        let resp = self.send(Method::GET, &url, None).await?;
        let blob: RecipeBlob = serde_json::from_str(&resp)?;
        Ok(blob)
    }

    pub async fn publish(
        &self,
        slug: &str,
        source: &str,
        metadata: &RecipeMeta,
    ) -> HubResult<RecipeMeta> {
        let url = format!("{}/v1/recipes/{}", self.base_url, slug);
        let body = serde_json::json!({
            "source": source,
            "metadata": metadata,
        });
        let resp = self
            .send(Method::POST, &url, Some(serde_json::to_string(&body)?))
            .await?;
        let meta: RecipeMeta = serde_json::from_str(&resp)?;
        Ok(meta)
    }

    pub async fn delete(&self, slug: &str, version: Option<u32>) -> HubResult<()> {
        let mut url = format!("{}/v1/recipes/{}", self.base_url, slug);
        if let Some(v) = version {
            url.push_str(&format!("?version={v}"));
        }
        self.send(Method::DELETE, &url, None).await?;
        Ok(())
    }

    pub async fn whoami(&self) -> HubResult<Option<String>> {
        let url = format!("{}/v1/oauth/whoami", self.base_url);
        let resp = self.send(Method::GET, &url, None).await.ok();
        let Some(body) = resp else {
            return Ok(None);
        };
        let v: serde_json::Value = serde_json::from_str(&body)?;
        Ok(v.get("login").and_then(|x| x.as_str()).map(String::from))
    }

    async fn send(&self, method: Method, url: &str, body: Option<String>) -> HubResult<String> {
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
        if !status.is_success() {
            // Try to decode a structured error envelope.
            let (code, message) = match serde_json::from_str::<serde_json::Value>(&text) {
                Ok(v) => (
                    v.get("error")
                        .and_then(|e| e.get("code"))
                        .and_then(|c| c.as_str())
                        .unwrap_or("ERROR")
                        .to_string(),
                    v.get("error")
                        .and_then(|e| e.get("message"))
                        .and_then(|m| m.as_str())
                        .unwrap_or(text.as_str())
                        .to_string(),
                ),
                Err(_) => ("HTTP".into(), text.clone()),
            };
            return Err(HubError::Api {
                status: status.as_u16(),
                code,
                message,
            });
        }
        Ok(text)
    }
}

fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            other => out.push_str(&format!("%{:02X}", other)),
        }
    }
    out
}
