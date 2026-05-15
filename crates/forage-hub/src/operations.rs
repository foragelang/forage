//! High-level Studio↔hub operations.
//!
//! These functions own the on-disk shape of a synced recipe. The wire
//! layer ([`crate::client`]) just speaks the REST API; everything that
//! materializes a `PackageVersion` into a workspace directory, walks an
//! on-disk recipe back into a `PublishRequest`, and tracks the source
//! version in a sidecar lives here so Studio's Tauri commands and the
//! CLI subcommands share one implementation.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::client::HubClient;
use crate::error::{HubError, HubResult};
use crate::types::{
    ForkedFrom, PackageFile, PackageFixture, PackageMetadata, PackageSnapshot, PackageVersion,
    PublishRequest, PublishResponse, VersionSpec,
};

/// File name of the hub-sync sidecar. Lives at
/// `<workspace>/<slug>/.forage-meta.json`. The leading dot keeps it
/// out of the visible file tree; consumers ignore it.
pub const META_SIDECAR_NAME: &str = ".forage-meta.json";

/// Sidecar tracking the hub origin of a synced recipe. `base_version`
/// drives the publish-back stale-base check; `forked_from` is the
/// upstream lineage when the recipe was created via fork (the value
/// hub-api stamps on the v1 metadata).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForageMeta {
    /// Pretty origin string: `"@author/slug@vN"`. Stored for display;
    /// the publish path reads `author` + `slug` + `base_version`
    /// individually.
    pub origin: String,
    pub author: String,
    pub slug: String,
    pub base_version: u32,
    pub forked_from: Option<ForkedFrom>,
}

impl ForageMeta {
    pub fn pretty_origin(author: &str, slug: &str, version: u32) -> String {
        format!("@{author}/{slug}@v{version}")
    }
}

/// Result of [`sync_from_hub`]: the directory the recipe was written
/// to (always `<workspace>/<slug>`), the version that landed, and the
/// sidecar shape so callers can echo "synced @author/slug@v4" back to
/// the user.
#[derive(Debug, Clone)]
pub struct SyncOutcome {
    pub recipe_dir: PathBuf,
    pub version: u32,
    pub meta: ForageMeta,
}

/// Pull `(author, slug, version)` from the hub, materialize it under
/// `workspace_root/<slug>/`, write the sidecar, and bump the download
/// counter. The version defaults to `latest` when `version` is `None`.
///
/// The on-disk layout matches what `forage-core`'s workspace loader
/// expects: `<slug>/recipe.forage`, `<slug>/fixtures/captures.jsonl`,
/// `<slug>/snapshot.json`, decls at their declared paths. The sidecar
/// hides under `.forage-meta.json`.
///
/// The destination must be empty (or contain only the sidecar's old
/// copy). Refusing to overwrite avoids clobbering an in-progress edit
/// when the user `forage sync`'s into a slug they already have.
pub async fn sync_from_hub(
    client: &HubClient,
    workspace_root: &Path,
    author: &str,
    slug: &str,
    version: Option<u32>,
) -> HubResult<SyncOutcome> {
    let spec = match version {
        Some(n) => VersionSpec::Numbered(n),
        None => VersionSpec::Latest,
    };
    let artifact = client.get_version(author, slug, spec).await?;
    let recipe_dir = workspace_root.join(slug);
    if let Some(existing) = read_meta(&recipe_dir)? {
        if existing.author == artifact.author
            && existing.slug == artifact.slug
            && existing.base_version >= artifact.version
        {
            return Err(HubError::Generic(format!(
                "{} already holds {} (version {}); refusing to overwrite",
                recipe_dir.display(),
                existing.origin,
                existing.base_version,
            )));
        }
    } else if recipe_dir.exists() && contains_recipe_files(&recipe_dir)? {
        return Err(HubError::Generic(format!(
            "{} already holds local recipe files; pick another destination",
            recipe_dir.display()
        )));
    }

    // Workspace sync: decls live at the workspace root so the
    // workspace loader's root-only declarations rule picks them up.
    materialize_version(&recipe_dir, workspace_root, &artifact)?;

    let forked_from = client
        .get_package(&artifact.author, &artifact.slug)
        .await
        .ok()
        .flatten()
        .and_then(|m: PackageMetadata| m.forked_from);

    let meta = ForageMeta {
        origin: ForageMeta::pretty_origin(&artifact.author, &artifact.slug, artifact.version),
        author: artifact.author.clone(),
        slug: artifact.slug.clone(),
        base_version: artifact.version,
        forked_from,
    };
    write_meta(&recipe_dir, &meta)?;

    // The counter is informational; if it fails we still consider the
    // sync successful. Log the bail-out so we notice systematic
    // failures (e.g. a worker outage that leaves counts behind).
    if let Err(e) = client.record_download(&artifact.author, &artifact.slug).await {
        tracing::warn!(
            error = %e,
            author = %artifact.author,
            slug = %artifact.slug,
            "download counter bump failed (sync continues)"
        );
    }

    Ok(SyncOutcome {
        recipe_dir,
        version: artifact.version,
        meta,
    })
}

/// Fetch a version artifact into the hub cache directory
/// (`<cache>/<author>/<slug>/<version>/`) so the workspace loader can
/// fold its decls into the type catalog. Returns the cache directory
/// and the SHA-256 of the raw JSON artifact (used to populate
/// `forage.lock`).
pub async fn fetch_to_cache(
    client: &HubClient,
    cache_root: &Path,
    author: &str,
    slug: &str,
    version: u32,
) -> HubResult<FetchedPackage> {
    let artifact = client
        .get_version(author, slug, VersionSpec::Numbered(version))
        .await?;
    let dir = cache_root.join(author).join(slug).join(version.to_string());
    // Dep cache: decls live inside the version-pinned subtree so
    // `scan_package_declarations` (which walks `cache/<author>/<slug>/
    // <version>/` recursively) finds them.
    materialize_version(&dir, &dir, &artifact)?;
    let sha = sha256_hex(&serde_json::to_string(&artifact)?);
    Ok(FetchedPackage { dir, sha256: sha })
}

/// Result of [`fetch_to_cache`]: on-disk path of the materialized
/// version plus the SHA-256 of its serialized wire artifact.
#[derive(Debug, Clone)]
pub struct FetchedPackage {
    pub dir: PathBuf,
    pub sha256: String,
}

fn sha256_hex(s: &str) -> String {
    use sha2::Digest;
    use std::fmt::Write;
    let mut h = sha2::Sha256::new();
    h.update(s.as_bytes());
    let out = h.finalize();
    let mut hex = String::with_capacity(out.len() * 2);
    for b in out {
        // Writing to `String` through `fmt::Write` is infallible —
        // expect surfaces the impossibility rather than silently
        // dropping the Result the way `let _ =` did.
        write!(hex, "{b:02x}").expect("String fmt::Write cannot fail");
    }
    hex
}

/// Create `@me/<as>` (or `@me/<upstream-slug>` when `as` is `None`)
/// from `(upstream_author, upstream_slug)`, then sync the new fork
/// into `workspace_root`. Returns the same shape as
/// [`sync_from_hub`].
pub async fn fork_from_hub(
    client: &HubClient,
    workspace_root: &Path,
    upstream_author: &str,
    upstream_slug: &str,
    r#as: Option<String>,
) -> HubResult<SyncOutcome> {
    let fork = client.fork(upstream_author, upstream_slug, r#as).await?;
    // The hub stamps the v1 artifact at fork time; we sync that.
    sync_from_hub(client, workspace_root, &fork.author, &fork.slug, Some(1)).await
}

/// Walk the on-disk workspace for `slug` and assemble the atomic
/// publish artifact. Reads the sidecar for `base_version`; absent
/// sidecar means "first publish". Lineage (`forked_from`) is
/// server-owned and is not part of the publish request.
pub fn assemble_publish_request(
    workspace_root: &Path,
    slug: &str,
    description: String,
    category: String,
    tags: Vec<String>,
) -> HubResult<PublishRequest> {
    let recipe_dir = workspace_root.join(slug);
    let recipe_path = recipe_dir.join("recipe.forage");
    let recipe = fs::read_to_string(&recipe_path).map_err(|e| {
        HubError::Io(io::Error::new(
            e.kind(),
            format!("read {}: {e}", recipe_path.display()),
        ))
    })?;

    let decls = read_workspace_decls(workspace_root)?;
    let fixtures = read_fixtures(&recipe_dir)?;
    let snapshot = read_snapshot(&recipe_dir)?;
    let meta = read_meta(&recipe_dir)?;
    let base_version = meta.map(|m| m.base_version);

    Ok(PublishRequest {
        description,
        category,
        tags,
        recipe,
        decls,
        fixtures,
        snapshot,
        base_version,
    })
}

/// Assemble the publish artifact, POST it, and on success update the
/// sidecar so subsequent publishes carry the new `base_version`.
pub async fn publish_from_workspace(
    client: &HubClient,
    workspace_root: &Path,
    author: &str,
    slug: &str,
    description: String,
    category: String,
    tags: Vec<String>,
) -> HubResult<PublishResponse> {
    let payload = assemble_publish_request(workspace_root, slug, description, category, tags)?;
    let resp = client.publish_version(author, slug, &payload).await?;
    let recipe_dir = workspace_root.join(slug);
    let existing = read_meta(&recipe_dir)?;
    let meta = ForageMeta {
        origin: ForageMeta::pretty_origin(author, slug, resp.version),
        author: author.to_string(),
        slug: slug.to_string(),
        base_version: resp.version,
        forked_from: existing.and_then(|m| m.forked_from),
    };
    write_meta(&recipe_dir, &meta)?;
    Ok(resp)
}

// --- Sidecar I/O ----------------------------------------------------

/// Path of the sidecar for `<workspace_root>/<slug>`.
pub fn meta_path(recipe_dir: &Path) -> PathBuf {
    recipe_dir.join(META_SIDECAR_NAME)
}

pub fn read_meta(recipe_dir: &Path) -> HubResult<Option<ForageMeta>> {
    let path = meta_path(recipe_dir);
    let raw = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(HubError::Io(io::Error::new(
                e.kind(),
                format!("read {}: {e}", path.display()),
            )));
        }
    };
    let meta: ForageMeta = serde_json::from_str(&raw)?;
    Ok(Some(meta))
}

pub fn write_meta(recipe_dir: &Path, meta: &ForageMeta) -> HubResult<()> {
    fs::create_dir_all(recipe_dir)?;
    let body = serde_json::to_string_pretty(meta)?;
    fs::write(meta_path(recipe_dir), body)?;
    Ok(())
}

// --- Materialization -----------------------------------------------

/// Lay the atomic `PackageVersion` artifact out on disk under
/// `recipe_dir`, with decls written under `decls_root`:
///
/// - `recipe.forage` ← `artifact.recipe`
/// - `<decls.name>` files relative to `decls_root` ← `artifact.decls`
/// - `fixtures/captures.jsonl` ← merged JSONL from `artifact.fixtures[*].content`
/// - `snapshot.json` ← `artifact.snapshot` (omitted when null)
///
/// `decls_root` differs by caller because the two consumers walk decls
/// from different roots:
///
/// - The workspace loader (`Workspace::catalog`) scans root-level
///   `.forage` files of the workspace itself, so `sync_from_hub` passes
///   the workspace root (the parent of `<workspace>/<slug>/`).
/// - The dep-cache loader (`scan_package_declarations`) walks the
///   version-pinned subtree `<cache>/<author>/<slug>/<version>/`
///   recursively, so `fetch_to_cache` passes the version directory
///   itself.
///
/// Decls keep their authored relative paths (so a publish that
/// declared `nested/shared.forage` lands at `<decls_root>/nested/...`).
/// `sanitize_member` rejects absolute names and `..` segments so a
/// hostile artifact can't escape the root.
fn materialize_version(
    recipe_dir: &Path,
    decls_root: &Path,
    artifact: &PackageVersion,
) -> HubResult<()> {
    fs::create_dir_all(recipe_dir)?;
    fs::create_dir_all(decls_root)?;

    fs::write(recipe_dir.join("recipe.forage"), &artifact.recipe)?;

    for f in &artifact.decls {
        let target = if f.name.contains('/') {
            sanitize_member(decls_root, &f.name)?
        } else {
            decls_root.join(&f.name)
        };
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&target, &f.source)?;
    }

    // Fixtures get folded into a single captures.jsonl — the studio +
    // CLI replay path reads one merged file per recipe.
    let fixtures_dir = recipe_dir.join("fixtures");
    fs::create_dir_all(&fixtures_dir)?;
    if !artifact.fixtures.is_empty() {
        let mut merged = String::new();
        for f in &artifact.fixtures {
            // Each fixture's content is already JSONL; concatenate
            // with a separating newline if one isn't already there.
            merged.push_str(&f.content);
            if !f.content.ends_with('\n') {
                merged.push('\n');
            }
        }
        fs::write(fixtures_dir.join("captures.jsonl"), merged)?;
    }

    if let Some(s) = &artifact.snapshot {
        let body = serde_json::to_string_pretty(s)?;
        fs::write(recipe_dir.join("snapshot.json"), body)?;
    }
    Ok(())
}

/// Validate a decl `name` and join it onto `root`. Rejects absolute
/// paths, traversal segments, and any post-resolve location that
/// escapes the root. The decl file does not exist yet, so we
/// canonicalize the parent and re-attach the leaf.
fn sanitize_member(root: &Path, name: &str) -> HubResult<PathBuf> {
    if name.is_empty()
        || name.starts_with('/')
        || name.starts_with('\\')
        || name.contains('\\')
        || name.contains("..")
        || name.contains("//")
        || name.contains("/./")
        || name.starts_with("./")
    {
        return Err(HubError::Generic(format!(
            "invalid decl file name: {name}"
        )));
    }
    for segment in name.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." {
            return Err(HubError::Generic(format!(
                "invalid decl file name: {name}"
            )));
        }
    }
    let target = root.join(name);
    let parent = target.parent().ok_or_else(|| {
        HubError::Generic(format!("decl file name has no parent: {name}"))
    })?;
    fs::create_dir_all(parent)?;
    let canonical_parent = parent.canonicalize()?;
    let canonical_root = root.canonicalize()?;
    if !canonical_parent.starts_with(&canonical_root) {
        return Err(HubError::Generic(format!(
            "decl file {name} escapes the workspace root",
        )));
    }
    let leaf = target.file_name().ok_or_else(|| {
        HubError::Generic(format!("decl file name has no leaf: {name}"))
    })?;
    Ok(canonical_parent.join(leaf))
}

fn contains_recipe_files(recipe_dir: &Path) -> HubResult<bool> {
    if !recipe_dir.exists() {
        return Ok(false);
    }
    let entries = fs::read_dir(recipe_dir)?;
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        // The sidecar itself doesn't count — we just overwrote one
        // when the user's last sync put it there. Hidden files in
        // general (.DS_Store, etc.) don't block a sync either.
        if name_str.starts_with('.') {
            continue;
        }
        return Ok(true);
    }
    Ok(false)
}

// --- Publish-side assembly --------------------------------------------

fn read_workspace_decls(workspace_root: &Path) -> HubResult<Vec<PackageFile>> {
    let mut out = Vec::new();
    if !workspace_root.exists() {
        return Ok(out);
    }
    // Root-level `.forage` files are header-less declarations — same
    // rule the workspace loader uses. Subdirectories hold recipes
    // (under `<slug>/recipe.forage`); we only publish the active
    // slug's recipe, so other slugs' folders are skipped here.
    let entries = fs::read_dir(workspace_root)?;
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy().into_owned();
        if name_str.starts_with('.') {
            continue;
        }
        let path = entry.path();
        if !entry.file_type()?.is_file() {
            continue;
        }
        if !name_str.ends_with(".forage") {
            continue;
        }
        let source = fs::read_to_string(&path)?;
        out.push(PackageFile {
            name: name_str,
            source,
        });
    }
    // Stable order on the wire — easier to diff between publishes.
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

fn read_fixtures(recipe_dir: &Path) -> HubResult<Vec<PackageFixture>> {
    let captures = recipe_dir.join("fixtures").join("captures.jsonl");
    let mut out = Vec::new();
    if captures.exists() {
        let content = fs::read_to_string(&captures)?;
        out.push(PackageFixture {
            name: "captures.jsonl".into(),
            content,
        });
    }
    Ok(out)
}

fn read_snapshot(recipe_dir: &Path) -> HubResult<Option<PackageSnapshot>> {
    let path = recipe_dir.join("snapshot.json");
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&path)?;
    // The on-disk snapshot is `forage_core::Snapshot` (records as a Vec
    // with `_id` + `typeName`); the hub stores per-type record arrays
    // + counts. Convert.
    let core_snapshot: forage_core::Snapshot = serde_json::from_str(&raw)?;
    Ok(Some(core_snapshot_to_wire(&core_snapshot)?))
}

/// Convert a `forage_core::Snapshot` into the hub's compact
/// per-type-arrays shape. Records carry the full JSON body
/// (`_id`, `typeName`, every field) so the hub can round-trip them
/// back without losing the synthetic id. A serialization failure on
/// any record propagates rather than landing as `null` in the wire
/// payload — `[null]` on the hub would survive replay as a phantom
/// record and the original failure would be lost.
pub fn core_snapshot_to_wire(snapshot: &forage_core::Snapshot) -> HubResult<PackageSnapshot> {
    let mut records: indexmap::IndexMap<String, Vec<serde_json::Value>> = indexmap::IndexMap::new();
    let mut counts: indexmap::IndexMap<String, u64> = indexmap::IndexMap::new();
    for r in &snapshot.records {
        let v = serde_json::to_value(r)?;
        records
            .entry(r.type_name.clone())
            .or_default()
            .push(v);
        *counts.entry(r.type_name.clone()).or_default() += 1;
    }
    Ok(PackageSnapshot { records, counts })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::PackageSnapshot;

    fn artifact(author: &str, slug: &str, v: u32) -> PackageVersion {
        PackageVersion {
            author: author.into(),
            slug: slug.into(),
            version: v,
            recipe: format!(
                "recipe \"{slug}\"\nengine http\n\nstep s {{ method \"GET\" url \"https://example.test\" }}\n"
            ),
            decls: vec![PackageFile {
                name: "shared.forage".into(),
                source: "type Shared { id: String }\n".into(),
            }],
            fixtures: vec![PackageFixture {
                name: "captures.jsonl".into(),
                content: "{\"kind\":\"http\",\"url\":\"https://example.test\",\"method\":\"GET\",\"status\":200,\"body\":\"{}\"}\n".into(),
            }],
            snapshot: Some(PackageSnapshot {
                records: indexmap::IndexMap::new(),
                counts: indexmap::IndexMap::new(),
            }),
            base_version: None,
            published_at: 0,
            published_by: author.into(),
        }
    }

    #[test]
    fn materialize_lays_out_workspace_files() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path();
        let recipe_dir = ws.join("zen-leaf");
        let art = artifact("alice", "zen-leaf", 4);
        // Workspace-style call: decls go to the workspace root.
        materialize_version(&recipe_dir, ws, &art).unwrap();

        assert!(recipe_dir.join("recipe.forage").is_file());
        assert!(ws.join("shared.forage").is_file());
        assert!(recipe_dir.join("fixtures").join("captures.jsonl").is_file());
        assert!(recipe_dir.join("snapshot.json").is_file());
    }

    #[test]
    fn materialize_dep_cache_keeps_decls_in_version_subtree() {
        let tmp = tempfile::tempdir().unwrap();
        let version_dir = tmp.path().join("alice").join("zen-leaf").join("4");
        let art = artifact("alice", "zen-leaf", 4);
        // Dep-cache-style call: recipe_dir and decls_root are the same
        // version-pinned directory, which is what
        // `scan_package_declarations` walks recursively.
        materialize_version(&version_dir, &version_dir, &art).unwrap();

        assert!(version_dir.join("recipe.forage").is_file());
        // shared.forage lands INSIDE the version dir, not in the
        // parent slug dir.
        assert!(version_dir.join("shared.forage").is_file());
        assert!(
            !version_dir
                .parent()
                .unwrap()
                .join("shared.forage")
                .exists(),
            "decls must not leak into the slug-level directory",
        );
    }

    #[test]
    fn meta_round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        let recipe_dir = tmp.path().join("rec");
        let meta = ForageMeta {
            origin: ForageMeta::pretty_origin("alice", "zen-leaf", 4),
            author: "alice".into(),
            slug: "zen-leaf".into(),
            base_version: 4,
            forked_from: None,
        };
        write_meta(&recipe_dir, &meta).unwrap();
        let back = read_meta(&recipe_dir).unwrap().unwrap();
        assert_eq!(back, meta);
    }

    #[test]
    fn assemble_uses_sidecar_base_version() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path();
        std::fs::create_dir_all(ws.join("zen-leaf").join("fixtures")).unwrap();
        std::fs::write(ws.join("zen-leaf").join("recipe.forage"), "recipe \"zen-leaf\"\nengine http\nstep s { method \"GET\" url \"x\" }\n").unwrap();
        let meta = ForageMeta {
            origin: ForageMeta::pretty_origin("alice", "zen-leaf", 4),
            author: "alice".into(),
            slug: "zen-leaf".into(),
            base_version: 4,
            forked_from: None,
        };
        write_meta(&ws.join("zen-leaf"), &meta).unwrap();
        let req = assemble_publish_request(
            ws,
            "zen-leaf",
            "desc".into(),
            "scrape".into(),
            vec![],
        )
        .unwrap();
        assert_eq!(req.base_version, Some(4));
        assert!(req.recipe.contains("zen-leaf"));
    }

    #[test]
    fn assemble_without_sidecar_means_first_publish() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path();
        std::fs::create_dir_all(ws.join("fresh")).unwrap();
        std::fs::write(ws.join("fresh").join("recipe.forage"), "recipe \"fresh\"\nengine http\nstep s { method \"GET\" url \"x\" }\n").unwrap();
        let req = assemble_publish_request(
            ws,
            "fresh",
            "desc".into(),
            "scrape".into(),
            vec![],
        )
        .unwrap();
        assert_eq!(req.base_version, None);
    }

    #[test]
    fn refuses_to_overwrite_local_recipe() {
        // sync_from_hub guards against clobbering local edits, even
        // without a sidecar.
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path();
        std::fs::create_dir_all(ws.join("zen-leaf")).unwrap();
        std::fs::write(ws.join("zen-leaf").join("recipe.forage"), "recipe \"zen-leaf\"\nengine http\n").unwrap();
        assert!(contains_recipe_files(&ws.join("zen-leaf")).unwrap());
    }
}
