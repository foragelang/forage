//! High-level Studio↔hub operations.
//!
//! These functions own the on-disk shape of a synced recipe. The wire
//! layer ([`crate::client`]) just speaks the REST API; everything that
//! materializes a `PackageVersion` into a workspace, walks an on-disk
//! recipe back into a `PublishRequest`, and tracks the source version
//! in a sidecar lives here so Studio's Tauri commands and the CLI
//! subcommands share one implementation.
//!
//! The hub-side "slug" is the recipe's header name; locally each
//! recipe is one flat file `<workspace>/<recipe>.forage`. Workspace
//! data (`_fixtures/<recipe>.jsonl`, `_snapshots/<recipe>.json`) and
//! the hub-sync sidecar (`.forage/sync/<recipe>.json`) hang off the
//! workspace root keyed on the same recipe-name string.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use forage_core::workspace::{Workspace, fixtures_path, snapshot_path};

use crate::client::HubClient;
use crate::error::{HubError, HubResult};
use crate::types::{
    ForkedFrom, PackageFile, PackageFixture, PackageMetadata, PackageSnapshot, PackageVersion,
    PublishRequest, PublishResponse, VersionSpec,
};

/// Per-workspace directory holding `forage publish` sidecars. Sits
/// inside `.forage/` so the source scan already skips it.
const META_DIR: &str = ".forage/sync";

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
    /// Hub-side recipe identifier. Equals the recipe's header name —
    /// the wire still calls it `slug` because the URL shape
    /// (`/v1/packages/:author/:slug`) is unchanged.
    pub slug: String,
    pub base_version: u32,
    pub forked_from: Option<ForkedFrom>,
}

impl ForageMeta {
    pub fn pretty_origin(author: &str, slug: &str, version: u32) -> String {
        format!("@{author}/{slug}@v{version}")
    }
}

/// Result of [`sync_from_hub`]: the file the recipe was written to
/// (always `<workspace>/<slug>.forage`), the version that landed, and
/// the sidecar shape so callers can echo "synced @author/slug@v4"
/// back to the user.
#[derive(Debug, Clone)]
pub struct SyncOutcome {
    pub recipe_path: PathBuf,
    pub version: u32,
    pub meta: ForageMeta,
}

/// Pull `(author, slug, version)` from the hub, materialize it under
/// `workspace_root`, write the sidecar, and bump the download counter.
/// The version defaults to `latest` when `version` is `None`.
///
/// The on-disk layout matches the flat workspace shape:
///
/// - `<workspace_root>/<slug>.forage` — the recipe source.
/// - `<workspace_root>/<decl-name>` — every decl at the relative path
///   the publisher used.
/// - `<workspace_root>/_fixtures/<slug>.jsonl` — captured replay data.
/// - `<workspace_root>/_snapshots/<slug>.json` — captured snapshot.
/// - `<workspace_root>/.forage/sync/<slug>.json` — hub-sync sidecar.
///
/// The destination file must be empty (or already a hub-synced copy
/// at an older version). Refusing to overwrite avoids clobbering an
/// in-progress edit when the user `forage sync`'s a recipe whose
/// header name collides with a local file.
///
/// Counter semantics: every successful sync bumps the upstream
/// package's `downloads` counter, including re-syncs that pull a
/// higher version into the same workspace. "Downloads" therefore
/// counts artifact-pulls, not unique users — the hub stays stateless
/// per user (no idempotency key, no per-caller dedup), and the
/// counter remains a useful "how lively is this package" signal.
/// Unique-user counting would require server-side caller identity,
/// which we don't want to grow for an informational stat.
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
    let recipe_name = recipe_name_from_source(&artifact.recipe, slug)?;
    if recipe_name != artifact.slug {
        return Err(HubError::Generic(format!(
            "hub-side slug {:?} does not match the recipe header name {recipe_name:?} \
             in the artifact; refusing to sync",
            artifact.slug,
        )));
    }
    let recipe_path = workspace_root.join(format!("{}.forage", artifact.slug));

    if let Some(existing) = read_meta(workspace_root, &artifact.slug)? {
        if existing.author == artifact.author
            && existing.slug == artifact.slug
            && existing.base_version >= artifact.version
        {
            return Err(HubError::Generic(format!(
                "{} already holds {} (version {}); refusing to overwrite",
                recipe_path.display(),
                existing.origin,
                existing.base_version,
            )));
        }
    } else if recipe_path.exists() {
        return Err(HubError::Generic(format!(
            "{} already exists locally and has no hub-sync sidecar; \
             pick another destination or remove the file first",
            recipe_path.display()
        )));
    }

    write_recipe_and_decls(workspace_root, &recipe_path, &artifact)?;
    write_fixtures_and_snapshot(workspace_root, &artifact.slug, &artifact)?;

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
    write_meta(workspace_root, &artifact.slug, &meta)?;

    // The counter is informational; if it fails we still consider
    // the sync successful. Log the bail-out at debug — a worker
    // outage would spam warn on every sync, and the user has nothing
    // to act on for an informational counter. The signal is still
    // captured in the structured log for anyone investigating
    // counter drift.
    if let Err(e) = client.record_download(&artifact.author, &artifact.slug).await {
        tracing::debug!(
            error = %e,
            author = %artifact.author,
            slug = %artifact.slug,
            "download counter bump failed (sync continues)"
        );
    }

    Ok(SyncOutcome {
        recipe_path,
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
    // <version>/` recursively) finds them. Fixtures and snapshots are
    // run-time concerns the dep-cache reader never touches, so skip
    // them here — the cache stays a pure source-files-only mirror.
    let recipe_path = dir.join(format!("{slug}.forage"));
    write_recipe_and_decls(&dir, &recipe_path, &artifact)?;
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
///
/// The hub's fork endpoint bumps the upstream's download counter
/// server-side; the inner `sync_from_hub` then runs against the new
/// fork (not the upstream), so a successful fork records exactly one
/// "download" against the *fork itself* on its first sync. Intentional
/// — the user did pull the artifact into their workspace — but worth
/// naming because it looks like two downloads got counted at first
/// glance.
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

/// Assemble a publish artifact keyed on the recipe's header name. The
/// recipe file is the one `Workspace::recipe_by_name(recipe_name)`
/// returns; the `decls` bundle every other workspace `.forage` file
/// that contains at least one `share`d declaration (so file-scoped
/// helpers stay file-scoped and aren't shipped to the hub); fixtures
/// and snapshot come off the workspace's `_fixtures/<recipe>.jsonl` /
/// `_snapshots/<recipe>.json`; `base_version` comes from the per-recipe
/// sidecar in `.forage/sync/`.
pub fn assemble_publish_request(
    workspace: &Workspace,
    recipe_name: &str,
    description: String,
    category: String,
    tags: Vec<String>,
) -> HubResult<PublishRequest> {
    let recipe_ref = workspace.recipe_by_name(recipe_name).ok_or_else(|| {
        HubError::Generic(format!(
            "no recipe named {recipe_name:?} in workspace {}",
            workspace.root.display()
        ))
    })?;
    let recipe = fs::read_to_string(recipe_ref.path).map_err(|e| {
        HubError::Io(io::Error::new(
            e.kind(),
            format!("read {}: {e}", recipe_ref.path.display()),
        ))
    })?;

    let decls = collect_shared_decls(workspace, recipe_ref.path)?;
    let fixtures = read_fixtures(&workspace.root, recipe_name)?;
    let snapshot = read_snapshot(&workspace.root, recipe_name)?;
    let meta = read_meta(&workspace.root, recipe_name)?;
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
    workspace: &Workspace,
    recipe_name: &str,
    author: &str,
    description: String,
    category: String,
    tags: Vec<String>,
) -> HubResult<PublishResponse> {
    let payload = assemble_publish_request(workspace, recipe_name, description, category, tags)?;
    let resp = client
        .publish_version(author, recipe_name, &payload)
        .await?;
    let existing = read_meta(&workspace.root, recipe_name)?;
    let meta = ForageMeta {
        origin: ForageMeta::pretty_origin(author, recipe_name, resp.version),
        author: author.to_string(),
        slug: recipe_name.to_string(),
        base_version: resp.version,
        forked_from: existing.and_then(|m| m.forked_from),
    };
    write_meta(&workspace.root, recipe_name, &meta)?;
    Ok(resp)
}

/// Walk the workspace and bundle every `.forage` file other than the
/// focal recipe that contains at least one `share`d type/enum/fn.
/// Files with only file-local declarations stay home — they're invisible
/// to the catalog anyway and have no business in the publish artifact.
fn collect_shared_decls(workspace: &Workspace, focal_path: &Path) -> HubResult<Vec<PackageFile>> {
    let mut out = Vec::new();
    for entry in &workspace.files {
        if entry.path == focal_path {
            continue;
        }
        let Ok(parsed) = entry.parsed.as_ref() else {
            // A broken sibling file is surfaced via `Workspace::broken()`;
            // skipping it here lets the publish proceed against the
            // healthy files of the workspace.
            continue;
        };
        let any_shared = parsed.types.iter().any(|t| t.shared)
            || parsed.enums.iter().any(|e| e.shared)
            || parsed.functions.iter().any(|f| f.shared);
        if !any_shared {
            continue;
        }
        let rel = entry
            .path
            .strip_prefix(&workspace.root)
            .unwrap_or(&entry.path);
        let name = rel.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/");
        let source = fs::read_to_string(&entry.path).map_err(|e| {
            HubError::Io(io::Error::new(
                e.kind(),
                format!("read {}: {e}", entry.path.display()),
            ))
        })?;
        out.push(PackageFile { name, source });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

// --- Sidecar I/O ----------------------------------------------------

/// Sidecar path for a recipe-name-keyed `forage publish`.
/// `<workspace>/.forage/sync/<recipe-name>.json`.
pub fn meta_path(workspace_root: &Path, recipe_name: &str) -> PathBuf {
    workspace_root
        .join(META_DIR)
        .join(format!("{recipe_name}.json"))
}

pub fn read_meta(workspace_root: &Path, recipe_name: &str) -> HubResult<Option<ForageMeta>> {
    let path = meta_path(workspace_root, recipe_name);
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

pub fn write_meta(
    workspace_root: &Path,
    recipe_name: &str,
    meta: &ForageMeta,
) -> HubResult<()> {
    let path = meta_path(workspace_root, recipe_name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let body = serde_json::to_string_pretty(meta)?;
    fs::write(path, body)?;
    Ok(())
}

// --- Materialization -----------------------------------------------

/// Lay the source half of an atomic `PackageVersion` artifact on disk:
/// the recipe file at `recipe_path` plus every decl rooted at
/// `decls_root`. Used by both `sync_from_hub` (workspace destination)
/// and `fetch_to_cache` (hub cache subtree); only the former goes on
/// to write data-dir files via [`write_fixtures_and_snapshot`].
///
/// `decls_root` differs by caller because the two consumers walk decls
/// from different roots:
///
/// - The workspace loader (`Workspace::catalog`) scans the workspace
///   root recursively, so `sync_from_hub` passes the workspace root.
/// - The dep-cache loader (`scan_package_declarations`) walks the
///   version-pinned subtree `<cache>/<author>/<slug>/<version>/`
///   recursively, so `fetch_to_cache` passes the version directory.
///
/// Decls keep their authored relative paths (so a publish that
/// declared `nested/shared.forage` lands at `<decls_root>/nested/...`).
/// `sanitize_member` rejects absolute names and `..` segments so a
/// hostile artifact can't escape the root.
fn write_recipe_and_decls(
    decls_root: &Path,
    recipe_path: &Path,
    artifact: &PackageVersion,
) -> HubResult<()> {
    fs::create_dir_all(decls_root)?;
    if let Some(parent) = recipe_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(recipe_path, &artifact.recipe)?;

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
    Ok(())
}

/// Lay the data half of the artifact under the workspace's recipe-name-
/// keyed data dirs:
///
/// - `<workspace_root>/_fixtures/<recipe>.jsonl` ← merged JSONL from
///   `artifact.fixtures[*].content`
/// - `<workspace_root>/_snapshots/<recipe>.json` ← `artifact.snapshot`
///   (omitted when null)
///
/// The fixtures merge concatenates every `PackageFixture.content` blob
/// with a separating newline; the hub wire format historically allows
/// multiple fixture entries per package, but the workspace stores one
/// JSONL stream per recipe.
fn write_fixtures_and_snapshot(
    workspace_root: &Path,
    recipe_name: &str,
    artifact: &PackageVersion,
) -> HubResult<()> {
    if !artifact.fixtures.is_empty() {
        let path = fixtures_path(workspace_root, recipe_name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut merged = String::new();
        for f in &artifact.fixtures {
            // Each fixture's content is already JSONL; concatenate
            // with a separating newline if one isn't already there.
            merged.push_str(&f.content);
            if !f.content.ends_with('\n') {
                merged.push('\n');
            }
        }
        fs::write(&path, merged)?;
    }

    if let Some(s) = &artifact.snapshot {
        let path = snapshot_path(workspace_root, recipe_name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let body = serde_json::to_string_pretty(s)?;
        fs::write(&path, body)?;
    }
    Ok(())
}

/// Parse `source` and pull the recipe's header name out. The publish
/// pipeline keys the workspace's data dirs and sidecar on the header
/// name, so a header-less artifact is a structured error — silently
/// landing captures in `_fixtures/.jsonl` would be a real bug.
fn recipe_name_from_source(source: &str, slug: &str) -> HubResult<String> {
    let parsed = forage_core::parse(source).map_err(|e| {
        HubError::Generic(format!("parse synced recipe @{slug}: {e}"))
    })?;
    parsed.recipe_name().map(str::to_string).ok_or_else(|| {
        HubError::Generic(format!(
            "synced recipe @{slug} has no `recipe \"<name>\"` header",
        ))
    })
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

// --- Publish-side I/O ------------------------------------------------

/// Read the workspace's per-recipe JSONL captures file and wrap its
/// raw bytes as a single `PackageFixture` for the publish wire. The
/// wire format historically allows multiple fixture entries per
/// package; today every consumer reads one JSONL stream per recipe,
/// so we ship a single entry called `captures.jsonl` to keep the
/// hub-side validation regex stable.
fn read_fixtures(workspace_root: &Path, recipe_name: &str) -> HubResult<Vec<PackageFixture>> {
    let path = fixtures_path(workspace_root, recipe_name);
    let mut out = Vec::new();
    match fs::read_to_string(&path) {
        Ok(content) => {
            out.push(PackageFixture {
                name: "captures.jsonl".into(),
                content,
            });
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => {
            return Err(HubError::Io(io::Error::new(
                e.kind(),
                format!("read {}: {e}", path.display()),
            )));
        }
    }
    Ok(out)
}

fn read_snapshot(workspace_root: &Path, recipe_name: &str) -> HubResult<Option<PackageSnapshot>> {
    let path = snapshot_path(workspace_root, recipe_name);
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
        records.entry(r.type_name.clone()).or_default().push(v);
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
                source: "share type Shared { id: String }\n".into(),
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

    /// `write_recipe_and_decls` lays the recipe file at the
    /// caller-supplied path and every decl alongside the decls root.
    /// `write_fixtures_and_snapshot` lays workspace data keyed on the
    /// recipe-name.
    #[test]
    fn writers_lay_flat_workspace_shape() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path();
        let art = artifact("alice", "zen-leaf", 4);
        let recipe_path = ws.join("zen-leaf.forage");
        write_recipe_and_decls(ws, &recipe_path, &art).unwrap();
        write_fixtures_and_snapshot(ws, &art.slug, &art).unwrap();

        assert!(recipe_path.is_file());
        assert!(ws.join("shared.forage").is_file());
        assert!(ws.join("_fixtures").join("zen-leaf.jsonl").is_file());
        assert!(ws.join("_snapshots").join("zen-leaf.json").is_file());
        // No legacy nested layout.
        assert!(!ws.join("zen-leaf").join("recipe.forage").exists());
        assert!(!ws.join("zen-leaf").join("fixtures").exists());
    }

    /// A dep-cache fetch writes the source half only — fixtures and
    /// snapshots are run-time concerns the cache never reads.
    #[test]
    fn fetch_to_cache_writes_source_only() {
        let tmp = tempfile::tempdir().unwrap();
        let version_dir = tmp.path().join("alice").join("zen-leaf").join("4");
        let art = artifact("alice", "zen-leaf", 4);
        let recipe_path = version_dir.join("zen-leaf.forage");
        write_recipe_and_decls(&version_dir, &recipe_path, &art).unwrap();

        assert!(recipe_path.is_file());
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
        // Cache never holds data dirs.
        assert!(!version_dir.join("_fixtures").exists());
        assert!(!version_dir.join("_snapshots").exists());
    }

    #[test]
    fn meta_round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path();
        let meta = ForageMeta {
            origin: ForageMeta::pretty_origin("alice", "zen-leaf", 4),
            author: "alice".into(),
            slug: "zen-leaf".into(),
            base_version: 4,
            forked_from: None,
        };
        write_meta(ws, "zen-leaf", &meta).unwrap();
        let back = read_meta(ws, "zen-leaf").unwrap().unwrap();
        assert_eq!(back, meta);
    }

    /// `assemble_publish_request` resolves the recipe via
    /// `Workspace::recipe_by_name`, bundles every sibling file that
    /// declares at least one `share`d type, and folds in the workspace
    /// data dirs.
    #[test]
    fn assemble_request_walks_flat_workspace() {
        let tmp = tempfile::tempdir().unwrap();
        let ws_root = tmp.path();
        fs::write(
            ws_root.join("forage.toml"),
            "name = \"alice/bar\"\ndescription = \"\"\ncategory = \"x\"\ntags = []\n",
        )
        .unwrap();
        fs::write(
            ws_root.join("foo.forage"),
            "recipe \"bar\"\nengine http\nstep s { method \"GET\" url \"https://example.test\" }\n",
        )
        .unwrap();
        fs::write(
            ws_root.join("shared.forage"),
            "share type Shared { id: String }\n",
        )
        .unwrap();
        // A sibling with only file-local decls must stay home.
        fs::write(
            ws_root.join("local-only.forage"),
            "type LocalOnly { id: String }\n",
        )
        .unwrap();
        fs::create_dir_all(ws_root.join("_fixtures")).unwrap();
        fs::write(ws_root.join("_fixtures").join("bar.jsonl"), "{\"k\":1}\n").unwrap();

        let meta = ForageMeta {
            origin: ForageMeta::pretty_origin("alice", "bar", 3),
            author: "alice".into(),
            slug: "bar".into(),
            base_version: 3,
            forked_from: None,
        };
        write_meta(ws_root, "bar", &meta).unwrap();

        let ws = forage_core::workspace::load(ws_root).unwrap();
        let req = assemble_publish_request(
            &ws,
            "bar",
            "desc".into(),
            "scrape".into(),
            vec!["t".into()],
        )
        .unwrap();
        assert!(req.recipe.contains("recipe \"bar\""));
        assert_eq!(req.base_version, Some(3));
        assert!(req.decls.iter().any(|d| d.name == "shared.forage"));
        assert!(
            req.decls.iter().all(|d| d.name != "local-only.forage"),
            "file-local-only siblings must not appear on the wire",
        );
        assert!(req.fixtures.iter().any(|f| f.name == "captures.jsonl"));
    }

    #[test]
    fn assemble_without_sidecar_means_first_publish() {
        let tmp = tempfile::tempdir().unwrap();
        let ws_root = tmp.path();
        fs::write(
            ws_root.join("forage.toml"),
            "name = \"alice/fresh\"\ndescription = \"\"\ncategory = \"x\"\ntags = []\n",
        )
        .unwrap();
        fs::write(
            ws_root.join("fresh.forage"),
            "recipe \"fresh\"\nengine http\nstep s { method \"GET\" url \"x\" }\n",
        )
        .unwrap();
        let ws = forage_core::workspace::load(ws_root).unwrap();
        let req = assemble_publish_request(
            &ws,
            "fresh",
            "desc".into(),
            "scrape".into(),
            vec![],
        )
        .unwrap();
        assert_eq!(req.base_version, None);
    }

    #[test]
    fn assemble_errors_on_unknown_recipe_name() {
        let tmp = tempfile::tempdir().unwrap();
        let ws_root = tmp.path();
        fs::write(
            ws_root.join("forage.toml"),
            "name = \"alice/x\"\ndescription = \"\"\ncategory = \"x\"\ntags = []\n",
        )
        .unwrap();
        fs::write(
            ws_root.join("a.forage"),
            "recipe \"a\"\nengine http\nstep s { method \"GET\" url \"x\" }\n",
        )
        .unwrap();
        let ws = forage_core::workspace::load(ws_root).unwrap();
        let err = assemble_publish_request(
            &ws,
            "missing",
            "".into(),
            "x".into(),
            vec![],
        )
        .unwrap_err();
        assert!(format!("{err}").contains("missing"), "unexpected: {err}");
    }
}
