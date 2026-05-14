//! Wire types shared between `HubClient` and the hub API.
//!
//! The unit of distribution is a **package** — a directory of `.forage`
//! files with an optional manifest. Single-file recipes ship as 1-file
//! packages. Metadata records the same publish info as before
//! (display name, summary, tags, license, owner) plus the file list.

use serde::{Deserialize, Serialize};

/// Metadata for one published package. Mirrors the server's
/// `RecipeMetadata` (minus internal storage keys).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PackageMeta {
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

/// One `.forage` file in a package. `name` is the file's path relative
/// to the package root (matches the server's
/// `FILE_NAME_RE`: one or more `/`-joined segments, each starting with
/// `[a-z0-9]`, joined by single `/` separators, terminating in
/// `.forage`). Workspace recipes typically publish as
/// `<dir>/recipe.forage`; shared declarations are usually
/// `<name>.forage` at the workspace root.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageFile {
    pub name: String,
    pub body: String,
}

/// A fetched package: the publish-time metadata that lived on the
/// listing entry, plus every `.forage` file's body in declared order.
///
/// Wire shape matches the server's detail response 1:1 — top-level
/// fields rather than a nested `metadata` blob so the wire is one
/// canonical view of a package version.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Package {
    pub slug: String,
    pub version: u32,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub platform: Option<String>,
    #[serde(default)]
    pub sha256: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
    pub files: Vec<PackageFile>,
}
