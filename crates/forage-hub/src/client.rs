//! `HubClient` — thin wrapper over the api.foragelang.com REST surface.
//!
//! The wire shape is package-oriented: publish takes a manifest plus a
//! list of `.forage` files; get returns the same. Single-file recipes
//! are 1-file packages. The cache layout mirrors the wire format
//! exactly: `<cache>/<author>/<slug>/<version>/<name>.forage`. Names may
//! include nested directories (`<dir>/recipe.forage`); the cache
//! mirrors that layout verbatim after sanitising against directory
//! traversal.

use std::path::{Path, PathBuf};

use reqwest::{Client, Method};
use serde::{Deserialize, Serialize};

use crate::error::{HubError, HubResult};
use crate::types::{Package, PackageFile, PackageMeta};

#[derive(Debug, Clone)]
pub struct HubClient {
    base_url: String,
    bearer_token: Option<String>,
    client: Client,
}

#[derive(Debug, Clone, Serialize)]
struct PublishPayload<'a> {
    slug: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    display_name: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary: Option<&'a str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    license: Option<&'a str>,
    files: Vec<PackageFile>,
}

/// Response from `POST /v1/packages/<slug>`.
#[derive(Debug, Clone, Deserialize)]
struct PublishResponse {
    slug: String,
    version: u32,
    #[serde(default)]
    sha256: Option<String>,
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

    /// List published packages. Returns minimal metadata records.
    pub async fn list(&self, query: Option<&str>) -> HubResult<Vec<PackageMeta>> {
        let mut url = format!("{}/v1/packages", self.base_url);
        if let Some(q) = query {
            url.push_str(&format!("?q={}", urlencode(q)));
        }
        let resp = self.send(Method::GET, &url, None).await?;
        let recipes: Vec<PackageMeta> = serde_json::from_str(&resp)?;
        Ok(recipes)
    }

    /// Fetch one package's full file list.
    pub async fn get_package(&self, slug: &str, version: Option<u32>) -> HubResult<Package> {
        let mut url = format!("{}/v1/packages/{}", self.base_url, slug);
        if let Some(v) = version {
            url.push_str(&format!("?version={v}"));
        }
        let resp = self.send(Method::GET, &url, None).await?;
        let pkg: Package = serde_json::from_str(&resp)?;
        Ok(pkg)
    }

    /// Push a package to the hub.
    pub async fn publish_package(
        &self,
        slug: &str,
        files: Vec<PackageFile>,
        metadata: &PackageMeta,
    ) -> HubResult<PackageMeta> {
        let url = format!("{}/v1/packages/{}", self.base_url, slug);
        let payload = PublishPayload {
            slug,
            display_name: metadata.display_name.as_deref(),
            summary: metadata.summary.as_deref(),
            tags: metadata.tags.clone(),
            license: metadata.license.as_deref(),
            files,
        };
        let body = serde_json::to_string(&payload)?;
        let resp = self.send(Method::POST, &url, Some(body)).await?;
        let r: PublishResponse = serde_json::from_str(&resp)?;
        Ok(PackageMeta {
            slug: r.slug,
            version: r.version,
            sha256: r.sha256,
            ..metadata.clone()
        })
    }

    pub async fn delete(&self, slug: &str, version: Option<u32>) -> HubResult<()> {
        let mut url = format!("{}/v1/packages/{}", self.base_url, slug);
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

/// High-level: fetch a package from the hub and write its files into
/// the local cache at `<cache>/<author>/<slug>/<version>/`. Returns the
/// resulting directory path so callers can pass it through to the
/// workspace catalog scan, along with the package digest (used to
/// populate `forage.lock`).
pub async fn fetch_package(
    client: &HubClient,
    slug: &str,
    version: u32,
) -> std::io::Result<FetchedPackage> {
    let pkg = client.get_package(slug, Some(version)).await.map_err(|e| {
        std::io::Error::other(format!("hub fetch failed for {slug}@{version}: {e}"))
    })?;
    let cache_dir = package_cache_dir(slug, version)
        .ok_or_else(|| std::io::Error::other(format!("malformed slug: {slug}")))?;
    std::fs::create_dir_all(&cache_dir)?;
    let canonical_root = cache_dir.canonicalize()?;
    for f in &pkg.files {
        let target = sanitize_package_member(&canonical_root, &f.name)?;
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&target, &f.body)?;
    }
    Ok(FetchedPackage {
        dir: cache_dir,
        sha256: pkg.sha256,
    })
}

/// Result of a successful `fetch_package`: the on-disk cache directory
/// plus the server-supplied package digest (when present). The digest
/// is what `forage update` records in `forage.lock`.
#[derive(Debug, Clone)]
pub struct FetchedPackage {
    pub dir: PathBuf,
    pub sha256: Option<String>,
}

/// Validate a package-supplied file name and join it onto the cache
/// directory. Rejects absolute paths, traversal segments, double slashes
/// (the wire format expects single-segment separators), and any path
/// that, once resolved, would escape the package cache root.
fn sanitize_package_member(canonical_root: &Path, name: &str) -> std::io::Result<PathBuf> {
    if name.is_empty()
        || name.starts_with('/')
        || name.starts_with('\\')
        || name.contains('\\')
        || name.contains("//")
        || name.contains("/./")
        || name.starts_with("./")
        || name.ends_with("/.")
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("invalid file name in package: {name}"),
        ));
    }
    for segment in name.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("invalid file name in package: {name}"),
            ));
        }
    }
    let target = canonical_root.join(name);
    // The file does not exist yet, so we canonicalize the parent path
    // and re-attach the leaf to confirm the resolved location stays
    // inside the cache root. This catches symlinks under the cache
    // pointing outside; an unconditional canonicalize would fail
    // because the leaf doesn't exist on disk.
    let parent = target.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("invalid file name in package: {name}"),
        )
    })?;
    std::fs::create_dir_all(parent)?;
    let canonical_parent = parent.canonicalize()?;
    if !canonical_parent.starts_with(canonical_root) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("package member {name} escapes the cache root"),
        ));
    }
    let leaf = target.file_name().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("invalid file name in package: {name}"),
        )
    })?;
    Ok(canonical_parent.join(leaf))
}

/// High-level: publish a workspace as a package. Caller hands in the
/// manifest plus the (name, body) pairs for every `.forage` file. The
/// metadata is filled in with the manifest's display fields when
/// available; the server stamps version + sha256.
pub async fn publish_package(
    client: &HubClient,
    slug: &str,
    files: Vec<PackageFile>,
    display_name: Option<&str>,
    summary: Option<&str>,
    tags: Vec<String>,
    license: Option<&str>,
) -> std::io::Result<PackageMeta> {
    let meta = PackageMeta {
        slug: slug.into(),
        version: 0,
        owner_login: None,
        display_name: display_name.map(str::to_string),
        summary: summary.map(str::to_string),
        tags,
        license: license.map(str::to_string),
        sha256: None,
        published_at: None,
    };
    client
        .publish_package(slug, files, &meta)
        .await
        .map_err(|e| std::io::Error::other(format!("hub publish failed for {slug}: {e}")))
}

/// On-disk location of a cached package, regardless of whether it's
/// been fetched yet. Returns `None` when the slug isn't a
/// `<author>/<name>` composite (single-segment slugs aren't routable
/// through the cache hierarchy).
pub fn package_cache_dir(slug: &str, version: u32) -> Option<PathBuf> {
    let (author, name) = slug.split_once('/')?;
    Some(
        hub_cache_root()
            .join(author)
            .join(name)
            .join(version.to_string()),
    )
}

/// Same convention as `forage_core::workspace::hub_cache_root`. Kept
/// here so hub-only crates don't need to depend on forage-core.
pub fn hub_cache_root() -> PathBuf {
    if let Ok(p) = std::env::var("FORAGE_HUB_CACHE") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    if cfg!(target_os = "macos") {
        if let Some(home) = dirs::home_dir() {
            return home.join("Library").join("Forage").join("Cache").join("hub");
        }
    }
    if let Some(data) = dirs::data_dir() {
        return data.join("Forage").join("Cache").join("hub");
    }
    PathBuf::from(".forage-cache").join("hub")
}

/// Look up a package in the local cache; returns the directory if
/// present. Used by the workspace loader to fold cached deps into the
/// type catalog without hitting the network.
pub fn resolve_dep(slug: &str, version: u32) -> Option<PathBuf> {
    let dir = package_cache_dir(slug, version)?;
    if dir.is_dir() { Some(dir) } else { None }
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
