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

use forage_core::workspace::{Workspace, fixtures_path, snapshot_path};

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
/// expects: `<slug>/recipe.forage`, decls at their declared paths
/// under `workspace_root`, captures at `_fixtures/<recipe>.jsonl`,
/// snapshot at `_snapshots/<recipe>.json`. The sidecar hides under
/// `.forage-meta.json`.
///
/// The destination must be empty (or contain only the sidecar's old
/// copy). Refusing to overwrite avoids clobbering an in-progress edit
/// when the user `forage sync`'s into a slug they already have.
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
    // workspace loader's root-only declarations rule picks them up;
    // captures and snapshot land in the workspace-level data dirs
    // keyed by the recipe's header name.
    let recipe_name = recipe_name_from_source(&artifact.recipe, &recipe_dir)?;
    write_recipe_and_decls(&recipe_dir, workspace_root, &artifact)?;
    write_fixtures_and_snapshot(workspace_root, &recipe_name, &artifact)?;

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
    // <version>/` recursively) finds them. Fixtures and snapshots are
    // run-time concerns the dep-cache reader never touches, so skip
    // them here — the cache stays a pure source-files-only mirror.
    write_recipe_and_decls(&dir, &dir, &artifact)?;
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
    let recipe_name = recipe_name_from_source(&recipe, &recipe_dir)?;

    let decls = read_workspace_decls(workspace_root)?;
    let fixtures = read_fixtures(workspace_root, &recipe_name)?;
    let snapshot = read_snapshot(workspace_root, &recipe_name)?;
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

// --- Recipe-name-keyed publish (Phase 6+) ---------------------------

/// Per-workspace directory holding `forage publish` sidecars in the
/// recipe-name-keyed layout. Sits inside `.forage/` so the source scan
/// already skips it.
const RECIPE_META_DIR: &str = ".forage/sync";

/// Sidecar path for a recipe-name-keyed `forage publish`.
/// `<workspace>/.forage/sync/<recipe-name>.json`.
pub fn recipe_meta_path(workspace_root: &Path, recipe_name: &str) -> PathBuf {
    workspace_root
        .join(RECIPE_META_DIR)
        .join(format!("{recipe_name}.json"))
}

pub fn read_recipe_meta(workspace_root: &Path, recipe_name: &str) -> HubResult<Option<ForageMeta>> {
    let path = recipe_meta_path(workspace_root, recipe_name);
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

pub fn write_recipe_meta(
    workspace_root: &Path,
    recipe_name: &str,
    meta: &ForageMeta,
) -> HubResult<()> {
    let path = recipe_meta_path(workspace_root, recipe_name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let body = serde_json::to_string_pretty(meta)?;
    fs::write(path, body)?;
    Ok(())
}

/// Assemble a publish artifact keyed on the recipe's header name. The
/// recipe file is the one `Workspace::recipe_by_name(recipe_name)`
/// returns; the `decls` bundle every other workspace `.forage` file
/// that contains at least one `share`d declaration (so file-scoped
/// helpers stay file-scoped and aren't shipped to the hub); fixtures
/// and snapshot come off the workspace's `_fixtures/<recipe>.jsonl` /
/// `_snapshots/<recipe>.json`; `base_version` comes from the per-recipe
/// sidecar in `.forage/sync/`.
pub fn assemble_recipe_publish(
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
    let meta = read_recipe_meta(&workspace.root, recipe_name)?;
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

/// Recipe-name-keyed counterpart to [`publish_from_workspace`]: looks
/// up the recipe in the workspace, POSTs to `<author>/<recipe_name>`,
/// and updates the per-recipe sidecar with the freshly-stamped version.
pub async fn publish_recipe_from_workspace(
    client: &HubClient,
    workspace: &Workspace,
    recipe_name: &str,
    author: &str,
    description: String,
    category: String,
    tags: Vec<String>,
) -> HubResult<PublishResponse> {
    let payload = assemble_recipe_publish(workspace, recipe_name, description, category, tags)?;
    let resp = client
        .publish_version(author, recipe_name, &payload)
        .await?;
    let existing = read_recipe_meta(&workspace.root, recipe_name)?;
    let meta = ForageMeta {
        origin: ForageMeta::pretty_origin(author, recipe_name, resp.version),
        author: author.to_string(),
        slug: recipe_name.to_string(),
        base_version: resp.version,
        forked_from: existing.and_then(|m| m.forked_from),
    };
    write_recipe_meta(&workspace.root, recipe_name, &meta)?;
    Ok(resp)
}

/// Walk the workspace and bundle every `.forage` file other than the
/// focal recipe that contains at least one `share`d type/enum/fn.
/// Files with only file-local declarations stay home — they're invisible
/// to the catalog anyway and have no business in the publish artifact.
fn collect_shared_decls(
    workspace: &Workspace,
    focal_path: &Path,
) -> HubResult<Vec<PackageFile>> {
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

/// Lay the source half of an atomic `PackageVersion` artifact on disk:
/// the recipe file at `<recipe_dir>/recipe.forage` plus every decl
/// rooted at `decls_root`. Used by both `sync_from_hub` (workspace
/// destination) and `fetch_to_cache` (hub cache subtree); only the
/// former goes on to write data-dir files via
/// [`write_fixtures_and_snapshot`].
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
fn write_recipe_and_decls(
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

/// Parse `source` and pull the recipe's header name out for keying the
/// workspace's data dirs. A failing parse or a header-less artifact is
/// a publish-side bug — the hub-api rejects publishes without a
/// `recipe "..."` header — but we surface a structured error here so
/// the sync path doesn't silently land captures in `_fixtures/.jsonl`.
fn recipe_name_from_source(source: &str, recipe_dir: &Path) -> HubResult<String> {
    let parsed = forage_core::parse(source).map_err(|e| {
        HubError::Generic(format!(
            "parse synced recipe at {}: {e}",
            recipe_dir.display()
        ))
    })?;
    parsed.recipe_name().map(str::to_string).ok_or_else(|| {
        HubError::Generic(format!(
            "synced recipe at {} has no `recipe \"<name>\"` header",
            recipe_dir.display()
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

    /// A workspace-side sync lays the recipe + decls in the legacy
    /// nested shape and the fixtures + snapshot in the workspace-level
    /// data dirs keyed by the recipe's header name.
    #[test]
    fn sync_writes_workspace_files_in_phase5_layout() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path();
        let recipe_dir = ws.join("zen-leaf");
        let art = artifact("alice", "zen-leaf", 4);
        write_recipe_and_decls(&recipe_dir, ws, &art).unwrap();
        let recipe_name = recipe_name_from_source(&art.recipe, &recipe_dir).unwrap();
        write_fixtures_and_snapshot(ws, &recipe_name, &art).unwrap();

        assert!(recipe_dir.join("recipe.forage").is_file());
        assert!(ws.join("shared.forage").is_file());
        assert!(ws.join("_fixtures").join("zen-leaf.jsonl").is_file());
        assert!(ws.join("_snapshots").join("zen-leaf.json").is_file());
        // No legacy data dirs under the recipe folder.
        assert!(!recipe_dir.join("fixtures").exists());
        assert!(!recipe_dir.join("snapshot.json").exists());
    }

    /// A dep-cache fetch writes the source half only — fixtures and
    /// snapshots are run-time concerns the cache never reads.
    #[test]
    fn fetch_to_cache_writes_source_only() {
        let tmp = tempfile::tempdir().unwrap();
        let version_dir = tmp.path().join("alice").join("zen-leaf").join("4");
        let art = artifact("alice", "zen-leaf", 4);
        write_recipe_and_decls(&version_dir, &version_dir, &art).unwrap();

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
        // Cache never holds data dirs.
        assert!(!version_dir.join("_fixtures").exists());
        assert!(!version_dir.join("_snapshots").exists());
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
