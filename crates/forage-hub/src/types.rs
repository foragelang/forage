//! Wire types matching `hub-api/src/types.ts`.
//!
//! Every JSON shape on the wire is `snake_case`. The structs here
//! serialize to the exact same field names that the worker emits and
//! validates, so a server-side rename forces a Rust-side rename in the
//! same PR (greenfield: no `#[serde(default)]` to smuggle drift through
//! the deserializer).

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

/// One `.forage` file shipped inside a version artifact. `name` is the
/// in-package path (slash-separated, ending in `.forage`); `source` is
/// the UTF-8 file body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageFile {
    pub name: String,
    pub source: String,
}

/// One named fixture inside a version artifact. `content` is the
/// fixture's UTF-8 body (typically JSONL capture data).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageFixture {
    pub name: String,
    pub content: String,
}

/// Captured run output stamped at publish time. Per-type record arrays
/// plus a counts summary. Records are intentionally untyped on the wire
/// because the type catalog is per-package — consumers reuse the
/// catalog from the artifact's recipe + decls.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackageSnapshot {
    pub records: IndexMap<String, Vec<Json>>,
    pub counts: IndexMap<String, u64>,
}

/// One-shot lineage pointer stamped on a v1 fork.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForkedFrom {
    pub author: String,
    pub slug: String,
    pub version: u32,
}

/// The atomic package version artifact: recipe + decls + fixtures +
/// snapshot ride together. There is no sub-resource that returns one
/// piece without the others.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackageVersion {
    pub author: String,
    pub slug: String,
    pub version: u32,
    pub recipe: String,
    pub decls: Vec<PackageFile>,
    pub fixtures: Vec<PackageFixture>,
    pub snapshot: Option<PackageSnapshot>,
    pub base_version: Option<u32>,
    pub published_at: i64,
    pub published_by: String,
}

/// Package metadata returned by `GET /v1/packages/:author/:slug`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageMetadata {
    pub author: String,
    pub slug: String,
    pub description: String,
    pub category: String,
    pub tags: Vec<String>,
    pub forked_from: Option<ForkedFrom>,
    pub created_at: i64,
    pub latest_version: u32,
    pub stars: u32,
    pub downloads: u32,
    pub fork_count: u32,
    pub owner_login: String,
}

/// `POST /v1/packages/:author/:slug/versions` body. The server validates
/// `base_version == latest_version` (or `None` for first publish) and
/// returns 409 otherwise.
///
/// `forked_from` is intentionally absent — lineage is server-owned.
/// The fork endpoint stamps `forked_from` on the v1 metadata and the
/// server preserves it across subsequent publishes against the fork.
/// Callers that want to know the lineage of a fork they're publishing
/// against can read it from the `PackageMetadata` response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PublishRequest {
    pub description: String,
    pub category: String,
    pub tags: Vec<String>,
    pub recipe: String,
    pub decls: Vec<PackageFile>,
    pub fixtures: Vec<PackageFixture>,
    pub snapshot: Option<PackageSnapshot>,
    pub base_version: Option<u32>,
}

/// Server's response to a successful publish. Fields mirror what the
/// worker returns in the `201` body; callers display `version`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishResponse {
    pub author: String,
    pub slug: String,
    pub version: u32,
    pub latest_version: u32,
}

/// `POST /v1/packages/:author/:slug/fork` body. `as` is the requested
/// slug for the new fork; `None` keeps the upstream's slug.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForkRequest {
    pub r#as: Option<String>,
}

/// Specifier for `GET /v1/packages/:author/:slug/versions/:n`. The
/// server accepts an integer or the literal `latest`.
#[derive(Debug, Clone, Copy)]
pub enum VersionSpec {
    Latest,
    Numbered(u32),
}

impl VersionSpec {
    pub fn as_path_segment(&self) -> String {
        match self {
            VersionSpec::Latest => "latest".into(),
            VersionSpec::Numbered(n) => n.to_string(),
        }
    }
}
