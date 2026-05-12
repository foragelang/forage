//! Shared capture types — `HTTPCapture` and `BrowserCapture`.
//!
//! Both the HTTP engine and the browser engine produce captures during
//! live runs and consume them during replay. The format is JSONL: one
//! capture per line at `fixtures/captures.jsonl`.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

/// Discriminator: HTTP exchange (request/response pair) vs browser
/// document body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Capture {
    Http(HttpExchange),
    Browser(BrowserCapture),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HttpExchange {
    pub url: String,
    pub method: String,
    #[serde(default)]
    pub request_headers: IndexMap<String, String>,
    #[serde(default)]
    pub request_body: Option<String>,
    pub status: u16,
    #[serde(default)]
    pub response_headers: IndexMap<String, String>,
    /// Response body — raw text, even if it's JSON.
    pub body: String,
}

/// A browser capture: either a fetch/XHR exchange (`Match`) or the
/// post-settle rendered document (`Document`). M10 interactive captures
/// (session bootstrap acknowledgement) also flow through `Document`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "subkind", rename_all = "lowercase")]
pub enum BrowserCapture {
    Match {
        url: String,
        method: String,
        status: u16,
        body: String,
    },
    Document {
        url: String,
        html: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_capture_round_trips() {
        let c = Capture::Http(HttpExchange {
            url: "https://example.com".into(),
            method: "GET".into(),
            request_headers: IndexMap::new(),
            request_body: None,
            status: 200,
            response_headers: IndexMap::new(),
            body: "{\"ok\":true}".into(),
        });
        let j = serde_json::to_string(&c).unwrap();
        let back: Capture = serde_json::from_str(&j).unwrap();
        assert_eq!(c, back);
    }

    #[test]
    fn browser_document_capture_round_trips() {
        let c = Capture::Browser(BrowserCapture::Document {
            url: "https://example.com".into(),
            html: "<html></html>".into(),
        });
        let j = serde_json::to_string(&c).unwrap();
        let back: Capture = serde_json::from_str(&j).unwrap();
        assert_eq!(c, back);
    }
}
