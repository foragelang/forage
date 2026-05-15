//! Shared capture types — `HTTPCapture` and `BrowserCapture` — plus
//! the JSONL stream codec they ride on disk.
//!
//! Captures are persisted at `<workspace>/_fixtures/<recipe>.jsonl`
//! (one capture per line) and read back into the engine's
//! `ReplayTransport`. The codec lives here because the on-disk format
//! is part of the capture contract — every consumer (CLI, Studio,
//! hub-sync, daemon record path) reads and writes through these
//! helpers rather than reimplementing line-split logic at the call
//! site.

use std::fs;
use std::io::{self, Write};
use std::path::Path;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use thiserror::Error;

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

/// Codec failures for the JSONL stream.
#[derive(Debug, Error)]
pub enum CaptureError {
    #[error("read {path}: {source}")]
    Io {
        path: std::path::PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("parse capture in {path} (line {line}): {source}")]
    Parse {
        path: std::path::PathBuf,
        line: usize,
        #[source]
        source: serde_json::Error,
    },
    #[error("serialize capture: {0}")]
    Serialize(#[source] serde_json::Error),
}

/// Parse an in-memory JSONL stream into a list of captures. Each
/// non-empty line is decoded as one `Capture`; blank lines are
/// skipped. Used by hosts that already hold the bytes in memory
/// (forage-wasm, scaffold parser).
pub fn parse_jsonl(jsonl: &str) -> Result<Vec<Capture>, serde_json::Error> {
    let mut out = Vec::new();
    for line in jsonl.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let c: Capture = serde_json::from_str(line)?;
        out.push(c);
    }
    Ok(out)
}

/// Read `path` as a JSONL stream of captures. A missing file resolves
/// to an empty list — the workspace has no recorded captures for this
/// recipe yet. Other I/O failures and JSON failures surface so the
/// caller can flag a corrupt fixture instead of silently running
/// against zero captures.
pub fn read_jsonl(path: &Path) -> Result<Vec<Capture>, CaptureError> {
    let raw = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(CaptureError::Io {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    let mut out = Vec::new();
    for (i, line) in raw.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let c: Capture = serde_json::from_str(line).map_err(|source| CaptureError::Parse {
            path: path.to_path_buf(),
            line: i + 1,
            source,
        })?;
        out.push(c);
    }
    Ok(out)
}

/// Write `captures` to `path` as a newline-delimited JSON stream,
/// creating the parent directory if missing. Each capture serializes
/// to a single line followed by `\n` — `read_jsonl` round-trips the
/// result identically.
pub fn write_jsonl(path: &Path, captures: &[Capture]) -> Result<(), CaptureError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| CaptureError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let mut file = fs::File::create(path).map_err(|source| CaptureError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    for c in captures {
        let line = serde_json::to_string(c).map_err(CaptureError::Serialize)?;
        file.write_all(line.as_bytes()).map_err(|source| CaptureError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        file.write_all(b"\n").map_err(|source| CaptureError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_http() -> Capture {
        Capture::Http(HttpExchange {
            url: "https://example.com".into(),
            method: "GET".into(),
            request_headers: IndexMap::new(),
            request_body: None,
            status: 200,
            response_headers: IndexMap::new(),
            body: "{\"ok\":true}".into(),
        })
    }

    fn sample_doc() -> Capture {
        Capture::Browser(BrowserCapture::Document {
            url: "https://example.com".into(),
            html: "<html></html>".into(),
        })
    }

    #[test]
    fn http_capture_round_trips() {
        let c = sample_http();
        let j = serde_json::to_string(&c).unwrap();
        let back: Capture = serde_json::from_str(&j).unwrap();
        assert_eq!(c, back);
    }

    #[test]
    fn browser_document_capture_round_trips() {
        let c = sample_doc();
        let j = serde_json::to_string(&c).unwrap();
        let back: Capture = serde_json::from_str(&j).unwrap();
        assert_eq!(c, back);
    }

    /// Writing then reading must produce the original capture list —
    /// the JSONL encoding is the on-disk contract every consumer
    /// depends on.
    #[test]
    fn jsonl_round_trips_through_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("captures.jsonl");
        let captures = vec![sample_http(), sample_doc()];
        write_jsonl(&path, &captures).unwrap();
        let back = read_jsonl(&path).unwrap();
        assert_eq!(captures, back);
    }

    /// A missing file resolves to an empty list (the workspace just
    /// hasn't recorded any captures yet); other I/O errors propagate.
    #[test]
    fn read_jsonl_returns_empty_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("missing.jsonl");
        assert!(read_jsonl(&path).unwrap().is_empty());
    }

    /// Blank lines between captures are skipped. JSON writers often
    /// emit a trailing newline, which `lines()` reports as one extra
    /// empty entry; the parser must absorb it.
    #[test]
    fn parse_jsonl_skips_blank_lines() {
        let body = "{\"kind\":\"http\",\"url\":\"x\",\"method\":\"GET\",\"status\":200,\"body\":\"\"}\n\n{\"kind\":\"http\",\"url\":\"y\",\"method\":\"GET\",\"status\":200,\"body\":\"\"}\n";
        let captures = parse_jsonl(body).unwrap();
        assert_eq!(captures.len(), 2);
    }

    /// A malformed line surfaces its line number so the caller can
    /// point the user at the broken record instead of failing
    /// vaguely.
    #[test]
    fn read_jsonl_reports_offending_line() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("bad.jsonl");
        fs::write(
            &path,
            "{\"kind\":\"http\",\"url\":\"x\",\"method\":\"GET\",\"status\":200,\"body\":\"\"}\nthis is not json\n",
        )
        .unwrap();
        let err = read_jsonl(&path).unwrap_err();
        match err {
            CaptureError::Parse { line, .. } => assert_eq!(line, 2),
            other => panic!("expected Parse, got {other:?}"),
        }
    }
}
