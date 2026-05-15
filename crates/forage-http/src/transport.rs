//! Transport abstraction for the HTTP engine.
//!
//! `Transport` is the async trait the engine drives; `ReplayTransport`
//! replays an in-memory capture list. Callers load the list via
//! `forage_replay::read_jsonl` (disk) or `parse_jsonl` (in-memory
//! string) and pass it to [`ReplayTransport::new`]. A live
//! `reqwest`-backed transport lives in `client.rs`.
//!
//! `EngineTransportContext` adapts a `&dyn Transport` into a
//! [`forage_core::TransportContext`] so transport-aware transforms
//! (`wikidataEntity` and friends) issue their fetches through the same
//! transport as step-level requests — which is what makes replay
//! capture them in the same fixture file.

use async_trait::async_trait;
use forage_core::{EvalError, EvalValue, TransportContext};
use indexmap::IndexMap;

use crate::error::{HttpError, HttpResult};
use forage_replay::{Capture, HttpExchange};

#[derive(Debug, Clone)]
pub struct HttpRequest {
    pub method: String,
    pub url: String,
    pub headers: IndexMap<String, String>,
    pub body: Option<Vec<u8>>,
}

#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: IndexMap<String, String>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn body_str(&self) -> &str {
        std::str::from_utf8(&self.body).unwrap_or("")
    }
}

#[async_trait]
pub trait Transport: Send + Sync {
    async fn fetch(&self, req: HttpRequest) -> HttpResult<HttpResponse>;
}

/// In-memory replayer: matches by method + URL.
pub struct ReplayTransport {
    pub fixtures: Vec<HttpExchange>,
}

impl ReplayTransport {
    pub fn new(captures: Vec<Capture>) -> Self {
        let fixtures = captures
            .into_iter()
            .filter_map(|c| match c {
                Capture::Http(h) => Some(h),
                _ => None,
            })
            .collect();
        Self { fixtures }
    }
}

#[async_trait]
impl Transport for ReplayTransport {
    async fn fetch(&self, req: HttpRequest) -> HttpResult<HttpResponse> {
        for f in &self.fixtures {
            if f.method.eq_ignore_ascii_case(&req.method) && url_matches(&f.url, &req.url) {
                return Ok(HttpResponse {
                    status: f.status,
                    headers: f.response_headers.clone(),
                    body: f.body.clone().into_bytes(),
                });
            }
        }
        Err(HttpError::NoFixture {
            method: req.method,
            url: req.url,
        })
    }
}

/// URL match heuristic: exact match wins; otherwise compare paths + query
/// presence (order-insensitive) so fixtures with extra/different params
/// still match. Good enough for the test recipes.
fn url_matches(fixture_url: &str, req_url: &str) -> bool {
    if fixture_url == req_url {
        return true;
    }
    // Normalize query parameter order.
    let f = strip_origin(fixture_url);
    let r = strip_origin(req_url);
    f == r
}

fn strip_origin(u: &str) -> String {
    // Keep path + sorted query params.
    let (path, query) = match u.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (u, None),
    };
    let path = match path.rfind("//") {
        Some(i) => {
            let rest = &path[i + 2..];
            match rest.find('/') {
                Some(j) => &rest[j..],
                None => rest,
            }
        }
        None => path,
    };
    match query {
        Some(q) => {
            let mut parts: Vec<&str> = q.split('&').collect();
            parts.sort_unstable();
            format!("{}?{}", path, parts.join("&"))
        }
        None => path.to_string(),
    }
}

/// Bridge from a `&dyn Transport` to `forage_core::TransportContext`.
/// Built once per engine run and passed into the async evaluator;
/// every transport-aware transform's fetch lands on the same transport
/// the engine uses for step-level requests, which is what makes
/// `--replay <fixtures>` cover wikidata reconciliation traffic.
pub struct EngineTransportContext<'t> {
    transport: &'t dyn Transport,
    user_agent: String,
}

impl<'t> EngineTransportContext<'t> {
    pub fn new(transport: &'t dyn Transport, user_agent: String) -> Self {
        Self { transport, user_agent }
    }
}

#[async_trait]
impl TransportContext for EngineTransportContext<'_> {
    async fn fetch_json(&self, url: &str) -> Result<EvalValue, EvalError> {
        let mut headers: IndexMap<String, String> = IndexMap::new();
        headers.insert("User-Agent".into(), self.user_agent.clone());
        headers.insert("Accept".into(), "application/json".into());
        let req = HttpRequest {
            method: "GET".into(),
            url: url.into(),
            headers,
            body: None,
        };
        let resp = self
            .transport
            .fetch(req)
            .await
            .map_err(|e| EvalError::TransportError {
                name: "<transform>".into(),
                msg: e.to_string(),
            })?;
        if !(200..400).contains(&resp.status) {
            return Err(EvalError::TransportError {
                name: "<transform>".into(),
                msg: format!("status {} from {}", resp.status, url),
            });
        }
        let body = resp.body_str();
        let parsed: serde_json::Value =
            serde_json::from_str(body).map_err(|e| EvalError::TransportError {
                name: "<transform>".into(),
                msg: format!("response is not JSON: {e}"),
            })?;
        Ok((&parsed).into())
    }
}
