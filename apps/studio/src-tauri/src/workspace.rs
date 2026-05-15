//! Filesystem helpers anchored on a workspace root. The root is whatever
//! the user opened — there's no longer a single global workspace.
//!
//! Recipe-scoped reads (source, deletes) key on the recipe header name
//! and consult `Workspace::recipe_by_name` to find the underlying file.
//! Recipes live at `<root>/<name>.forage` (the flat shape every
//! post-Phase-10 surface expects). A workspace that still carries the
//! pre-Phase-10 `<root>/<slug>/recipe.forage` shape is rejected at the
//! command boundary with a `forage migrate` prompt.
//!
//! This module also owns the cross-workspace **recents sidecar**: a
//! JSON file in the OS data dir tracking which workspaces the user has
//! opened recently. It lives outside any workspace because Studio needs
//! to read it before any workspace is open.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use forage_core::workspace::{MANIFEST_NAME, Manifest, Workspace, serialize_manifest};

/// Drop an empty `forage.toml` at `<root>/forage.toml` if it doesn't
/// exist. Called from `new_workspace` after `mkdir -p` so a brand-new
/// directory becomes a workspace; refuses to overwrite existing
/// manifests so re-opening with the New action would surface a real
/// "already exists" error.
pub fn write_empty_manifest(root: &Path) -> io::Result<()> {
    fs::create_dir_all(root)?;
    let path = root.join(MANIFEST_NAME);
    if path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{} already has a forage.toml — use Open instead", root.display()),
        ));
    }
    let body = serialize_manifest(&Manifest::default())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
    fs::write(&path, body)
}

/// Resolve a recipe's on-disk file path by header name. Looks the
/// recipe up in the cached workspace listing.
pub fn resolve_recipe_path(ws: &Workspace, name: &str) -> Result<PathBuf, String> {
    ws.recipe_by_name(name)
        .map(|r| r.path.to_path_buf())
        .ok_or_else(|| format!("no recipe named {name:?} in workspace at {}", ws.root.display()))
}

/// User-facing rejection used when an action targets a recipe still
/// in the legacy `<slug>/recipe.forage` layout. Studio refuses to act
/// against unmigrated workspaces; the CLI's `forage migrate` is the
/// one-shot fix.
pub fn unmigrated_workspace_message(root: &Path) -> String {
    format!(
        "this workspace has not been migrated — run `forage migrate {}`",
        root.display()
    )
}

/// True when `recipe_path` is at the pre-Phase-10 legacy layout
/// (`<root>/<slug>/recipe.forage`): exactly one directory deep under
/// the root, with the file basename literally `recipe.forage`.
pub fn is_legacy_recipe_path(root: &Path, recipe_path: &Path) -> bool {
    let Some(parent) = recipe_path.parent() else {
        return false;
    };
    if parent == root {
        return false;
    }
    if parent.parent() != Some(root) {
        return false;
    }
    recipe_path
        .file_name()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s == "recipe.forage")
}

/// Scaffold a new recipe at `<root>/<name>.forage` with a `recipe
/// "<name>" engine http` header. When `name_override` is None, pick
/// the next-available `untitled-N` (so a fresh workspace starts at
/// `untitled-1.forage`). Returns the recipe header name (which is also
/// the file stem) so callers can wire it into the active-recipe
/// selection without re-parsing.
pub fn create_recipe(root: &Path, name_override: Option<&str>) -> io::Result<String> {
    fs::create_dir_all(root)?;
    let name = match name_override {
        Some(n) => n.to_string(),
        None => pick_untitled_name(root)?,
    };
    let target = root.join(format!("{name}.forage"));
    if target.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{} already exists; refusing to overwrite", target.display()),
        ));
    }
    let body = format!(
        "recipe \"{name}\" engine http\n\ntype Item {{\n    id: String\n}}\n\nstep list {{\n    method \"GET\"\n    url    \"https://example.com\"\n}}\n\nfor $i in $list.items[*] {{\n    emit Item {{\n        id ← $i.id\n    }}\n}}\n"
    );
    fs::write(&target, body)?;
    Ok(name)
}

/// Pick the first `untitled-N` whose `.forage` file does not yet exist
/// in `root`. Counter starts at 1 and caps at 1000 to guard against a
/// pathological filesystem state.
fn pick_untitled_name(root: &Path) -> io::Result<String> {
    for n in 1..=1000 {
        let candidate = format!("untitled-{n}");
        let path = root.join(format!("{candidate}.forage"));
        if !path.exists() {
            return Ok(candidate);
        }
    }
    Err(io::Error::other("too many untitled recipes"))
}

/// Read the source of the recipe named `name`. Resolves the on-disk
/// path through the workspace's recipe index, then reads the file.
pub fn read_source(ws: &Workspace, name: &str) -> Result<String, String> {
    let path = resolve_recipe_path(ws, name)?;
    fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))
}

/// Delete the on-disk file for the recipe named `name`. Resolves the
/// path through the workspace index and removes `<root>/<name>.forage`.
/// A recipe still at the legacy `<root>/<slug>/recipe.forage` location
/// is rejected with the migration prompt — Studio doesn't act against
/// unmigrated workspaces.
///
/// Refuses anything resolving outside the workspace root — a symlinked
/// recipe file pointing at `/etc/passwd` would otherwise let us delete
/// unrelated content.
pub fn delete_recipe(ws: &Workspace, name: &str) -> io::Result<()> {
    let path = ws
        .recipe_by_name(name)
        .map(|r| r.path.to_path_buf())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("no recipe named {name:?} in workspace at {}", ws.root.display()),
            )
        })?;
    if is_legacy_recipe_path(&ws.root, &path) {
        return Err(io::Error::other(unmigrated_workspace_message(&ws.root)));
    }

    let canonical = path.canonicalize()?;
    let root_canonical = ws.root.canonicalize()?;
    if !canonical.starts_with(&root_canonical) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("recipe path {canonical:?} escapes workspace root {root_canonical:?}"),
        ));
    }
    fs::remove_file(&canonical)
}

/// Read `<workspace>/_fixtures/<recipe_name>.jsonl`. Returns an empty
/// list on a missing file (the workspace hasn't recorded captures for
/// this recipe yet) and logs other I/O / parse failures at `warn` —
/// the run pipeline still has something to replay against even when a
/// fixture is corrupt, but the user gets a structured log line they
/// can act on.
pub fn read_captures(root: &Path, recipe_name: &str) -> Vec<forage_replay::Capture> {
    let path = forage_core::workspace::fixtures_path(root, recipe_name);
    match forage_replay::read_jsonl(&path) {
        Ok(captures) => captures,
        Err(e) => {
            tracing::warn!(
                error = %e,
                recipe_name = %recipe_name,
                path = %path.display(),
                "read_captures failed; falling back to empty list",
            );
            Vec::new()
        }
    }
}

/// Per-recipe breakpoint persistence. One JSON sidecar at
/// `<workspace_root>/breakpoints.json` keyed by recipe header name. The
/// file is missing until the user sets a first breakpoint, so the
/// empty-map case is the steady state for fresh workspaces.
pub fn breakpoints_path(root: &Path) -> PathBuf {
    root.join("breakpoints.json")
}

pub fn read_breakpoints(root: &Path) -> io::Result<std::collections::HashMap<String, Vec<String>>> {
    let path = breakpoints_path(root);
    let raw = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            // Steady state for a fresh workspace — no breakpoints yet,
            // no sidecar yet. Distinct from a malformed file, which
            // surfaces below.
            return Ok(std::collections::HashMap::new());
        }
        Err(e) => return Err(e),
    };
    serde_json::from_str(&raw).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

pub fn write_breakpoints(
    root: &Path,
    map: &std::collections::HashMap<String, Vec<String>>,
) -> io::Result<()> {
    let path = breakpoints_path(root);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let body = serde_json::to_string_pretty(map)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    fs::write(path, body)
}

/// Convenience for `forage_keychain` env-style secret resolution.
pub fn read_secrets_from_env(recipe: &forage_core::ForageFile) -> indexmap::IndexMap<String, String> {
    let mut out = indexmap::IndexMap::new();
    for s in &recipe.secrets {
        let key = format!("FORAGE_SECRET_{}", s.to_uppercase());
        if let Ok(v) = std::env::var(&key) {
            out.insert(s.clone(), v);
        }
    }
    out
}

// ---------------------------------------------------------------------
// Workspace info + file tree wire types.
// ---------------------------------------------------------------------

/// Wire view of the loaded `Workspace`. Carries the manifest summary
/// and the root path — the file list is served separately by
/// `list_workspace_files` so the UI can refetch the tree without
/// re-shipping the manifest on every poll.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct WorkspaceInfo {
    #[ts(type = "string")]
    pub root: PathBuf,
    /// Publish slug from `forage.toml`, e.g. `"dima/cannabis"`.
    pub name: Option<String>,
    /// `"author/slug"` → integer hub version. BTreeMap preserves
    /// sorted iteration on the wire so consumers see deterministic
    /// ordering.
    pub deps: BTreeMap<String, u32>,
    /// User home directory, if discoverable. The UI uses this prefix
    /// to render `~`-shortened paths in the workspace header. `None`
    /// when no home directory exists (test sandboxes, some CI hosts).
    #[ts(type = "string | null")]
    pub home: Option<PathBuf>,
}

impl WorkspaceInfo {
    pub fn from_workspace(ws: &Workspace) -> Self {
        Self::from_manifest(ws.root.clone(), &ws.manifest)
    }

    pub fn from_manifest(root: PathBuf, manifest: &Manifest) -> Self {
        Self {
            root,
            name: manifest.name.clone(),
            deps: manifest.deps.clone(),
            home: dirs::home_dir(),
        }
    }
}

/// Recursive file-tree node returned by `list_workspace_files`. The
/// tree is rooted at the workspace root; each `File` carries a precise
/// `FileKind` classification so the UI can render the right affordance
/// (recipe -> open in editor, fixture -> open as JSON, etc.).
#[derive(Debug, Clone, Serialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum FileNode {
    File {
        name: String,
        #[ts(type = "string")]
        path: PathBuf,
        file_kind: FileKind,
    },
    Folder {
        name: String,
        #[ts(type = "string")]
        path: PathBuf,
        children: Vec<FileNode>,
    },
}

/// File classification rules:
///   `forage.toml`                  → `Manifest`
///   `*.forage` at workspace root   → `Declarations`
///   `_fixtures/<recipe>.jsonl`     → `Fixture`
///   `_snapshots/<recipe>.json`     → `Snapshot`
///   everything else                → `Other`
///
/// Classification is path-based — the kind doesn't read file contents.
/// A `.forage` file at the root may be a recipe or a shared-types
/// declaration; the UI joins it back against the parsed recipe index
/// to tell the two apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum FileKind {
    Declarations,
    Fixture,
    Snapshot,
    Manifest,
    Other,
}

/// Per-recipe status combining the Studio's on-disk view (drafts)
/// with the daemon's view (deployed versions). The frontend renders
/// these side by side so the user can see "edited but not deployed"
/// or "deployed but the draft is missing" without joining the two
/// views itself. Keyed on the recipe header name.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct RecipeStatus {
    pub name: String,
    pub draft: DraftState,
    pub deployed: DeployedState,
}

/// Whether a recipe has a draft on disk and whether that draft parses.
/// `Missing` covers the deployed-but-no-draft case: the daemon
/// remembers a recipe we no longer have a source file for.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DraftState {
    Valid {
        #[ts(type = "string")]
        path: PathBuf,
    },
    Broken {
        #[ts(type = "string")]
        path: PathBuf,
        error: String,
    },
    Missing,
}

/// Whether a slug has any deployed version in the daemon. Carries the
/// latest version's metadata when present.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DeployedState {
    None,
    Deployed {
        version: u32,
        #[ts(type = "number")]
        deployed_at: i64,
    },
}

/// Walk `root` recursively and return a single `Folder` node with
/// classified `File` children. Hidden entries (`.*`) — including the
/// daemon's working dir `.forage/` — are skipped so the file tree
/// reflects what the user authored, not what the runtime cached.
///
/// `FileNode.path` is workspace-relative. The frontend's
/// path-equality checks across the UI assume that shape. The root
/// folder's relative path is empty; the UI iterates its `children`
/// directly and never selects the root itself.
pub fn build_file_tree(root: &Path) -> io::Result<FileNode> {
    let name = root
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.display().to_string());
    let children = read_children(root, root)?;
    Ok(FileNode::Folder {
        name,
        path: PathBuf::new(),
        children,
    })
}

fn read_children(root: &Path, dir: &Path) -> io::Result<Vec<FileNode>> {
    let raw = match fs::read_dir(dir) {
        Ok(it) => it,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };

    // Capture `(is_dir, name, path)` once per entry. Without this,
    // the comparator below ends up calling `entry.file_type()` —
    // an `lstat` syscall — twice per pairwise compare, so a sort
    // of N entries is ~2N log N syscalls. With the upcoming 4s
    // refetch loop in the sidebar, that's needless work.
    struct Item {
        is_dir: bool,
        name: std::ffi::OsString,
        path: PathBuf,
        is_regular_file: bool,
    }

    let mut items: Vec<Item> = Vec::new();
    for entry in raw.flatten() {
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        let ft = entry.file_type()?;
        items.push(Item {
            is_dir: ft.is_dir(),
            is_regular_file: ft.is_file(),
            name,
            path: entry.path(),
        });
    }

    // Stable, predictable ordering: directories first, then files,
    // each sorted by name (case-sensitive). The UI doesn't need to
    // sort, and the tree diffs cleanly across refetches.
    items.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.cmp(&b.name),
    });

    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let name = item.name.to_string_lossy().into_owned();
        let rel = item
            .path
            .strip_prefix(root)
            .map(Path::to_path_buf)
            .unwrap_or_else(|_| item.path.clone());
        if item.is_dir {
            let children = read_children(root, &item.path)?;
            out.push(FileNode::Folder {
                name,
                path: rel,
                children,
            });
        } else if item.is_regular_file {
            let file_kind = classify_file(root, &item.path);
            out.push(FileNode::File {
                name,
                path: rel,
                file_kind,
            });
        }
        // Symlinks / other entries are skipped — a workspace is a
        // plain tree of files and folders.
    }
    Ok(out)
}

fn classify_file(root: &Path, path: &Path) -> FileKind {
    let rel = path.strip_prefix(root).unwrap_or(path);
    let components: Vec<_> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    let last = match components.last() {
        Some(s) => s.as_str(),
        None => return FileKind::Other,
    };

    // Manifest sits at the workspace root.
    if components.len() == 1 && last == MANIFEST_NAME {
        return FileKind::Manifest;
    }

    let extension = path
        .extension()
        .map(|e| e.to_string_lossy().into_owned())
        .unwrap_or_default();

    // `.forage` files at the workspace root carry the recipe or
    // shared-types declarations the toolchain consumes. Anything
    // deeper (including the legacy `<slug>/recipe.forage` shape) is
    // unclassified — the UI shows it but action-bound commands
    // surface the `forage migrate` prompt instead of operating on it.
    if extension == "forage" {
        if components.len() == 1 {
            return FileKind::Declarations;
        }
        return FileKind::Other;
    }

    // Phase 5 data dirs: per-recipe JSONL captures and the published-run
    // snapshot live next to the workspace root keyed by recipe header
    // name.
    if components.len() == 2 {
        let parent = components[0].as_str();
        if parent == forage_core::workspace::FIXTURES_DIR && extension == "jsonl" {
            return FileKind::Fixture;
        }
        if parent == forage_core::workspace::SNAPSHOTS_DIR && extension == "json" {
            return FileKind::Snapshot;
        }
    }

    FileKind::Other
}

// ---------------------------------------------------------------------
// Recents sidecar.
// ---------------------------------------------------------------------

/// One row in the recents list. Captured at workspace-open time;
/// `recipe_count` is a cached integer so the Welcome view doesn't have
/// to scan every recent workspace's disk.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RecentWorkspace {
    #[ts(type = "string")]
    pub path: PathBuf,
    pub name: String,
    /// Milliseconds since the Unix epoch.
    #[ts(type = "number")]
    pub opened_at: i64,
    pub recipe_count: u32,
}

/// File layout of the recents sidecar. Wrapped in a single-field struct
/// so future top-level fields (e.g. a schema version) can land without
/// breaking shape.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct RecentsFile {
    #[serde(default)]
    workspaces: Vec<RecentWorkspace>,
}

/// Resolve the cross-workspace recents sidecar path. Honors
/// `FORAGE_DATA_DIR` first — tests sandbox into a tempdir; users
/// shouldn't need this in practice.
pub fn recents_path() -> PathBuf {
    if let Ok(override_dir) = std::env::var("FORAGE_DATA_DIR")
        && !override_dir.is_empty()
    {
        return PathBuf::from(override_dir).join("recents.json");
    }
    dirs::data_dir()
        .expect("dirs::data_dir() returned None on a supported platform")
        .join("Forage")
        .join("recents.json")
}

/// Load the recents list, filtering out entries whose path no longer
/// exists on disk. Missing or corrupt sidecars degrade to an empty
/// list — recents are nice-to-have, not load-bearing.
///
/// Corrupt files log at `warn` and return empty rather than panicking;
/// the user can keep using the app and the next successful write
/// replaces the bad file. The pre-filter doesn't log per dropped
/// entry — that would spam the console every UI poll; the one-time
/// startup prune (`prune_recents`) handles the loud-once side.
pub fn read_recents() -> Vec<RecentWorkspace> {
    let path = recents_path();
    let raw = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Vec::new(),
        Err(e) => {
            tracing::warn!(error = %e, path = %path.display(), "failed to read recents sidecar");
            return Vec::new();
        }
    };
    let file: RecentsFile = match serde_json::from_str(&raw) {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!(error = %e, path = %path.display(), "recents sidecar is not valid JSON");
            return Vec::new();
        }
    };
    file.workspaces
        .into_iter()
        .filter(|w| w.path.exists())
        .collect()
}

/// One-shot startup prune: read the sidecar, drop entries whose path
/// is gone, write back if anything changed. Logs at info so the user
/// sees that stale data was cleared (e.g. tempdirs from old test runs
/// that leaked into the real recents file). Called from `lib.rs::setup`
/// during boot; safe to call at any other time too — idempotent.
pub fn prune_recents() {
    let raw = read_recents_raw();
    if raw.is_empty() {
        return;
    }
    let kept: Vec<RecentWorkspace> =
        raw.iter().filter(|w| w.path.exists()).cloned().collect();
    let dropped = raw.len() - kept.len();
    if dropped == 0 {
        return;
    }
    for w in &raw {
        if !w.path.exists() {
            tracing::info!(path = %w.path.display(), "recents: dropping stale entry whose path is gone");
        }
    }
    if let Err(e) = write_recents(&kept) {
        tracing::warn!(error = %e, "recents: prune failed to write back; will retry on next mutation");
    } else {
        tracing::info!(dropped, kept = kept.len(), "recents: pruned stale entries at startup");
    }
}

/// Atomic write of the recents sidecar. Uses tempfile + rename to mirror
/// the daemon-fortress deployments pattern: the rename is atomic on the
/// same filesystem, so readers either see the old file or the new file
/// — never a half-written truncation.
fn write_recents(list: &[RecentWorkspace]) -> io::Result<()> {
    let path = recents_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let body = serde_json::to_vec_pretty(&RecentsFile {
        workspaces: list.to_vec(),
    })
    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    // Tempfile in the same dir so the rename stays on one filesystem.
    let parent = path.parent().expect("recents path has parent");
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    tmp.write_all(&body)?;
    tmp.as_file().sync_all()?;
    tmp.persist(&path).map_err(|e| e.error)?;
    Ok(())
}

/// Read the sidecar without the path-still-exists filter — the recorder
/// needs to dedupe against entries that may no longer exist on disk so
/// stale rows don't keep accumulating.
fn read_recents_raw() -> Vec<RecentWorkspace> {
    let path = recents_path();
    let raw = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let file: RecentsFile = match serde_json::from_str(&raw) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };
    file.workspaces
}

/// Push a freshly-opened workspace to the front of the recents list.
/// Dedupes by canonical path; truncates to 10 entries. The recents
/// sidecar lives outside any workspace, so this is safe to call from
/// any path-resolution state.
pub fn record_recent(path: &Path, name: String, recipe_count: u32) -> io::Result<()> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let entry = RecentWorkspace {
        path: canonical.clone(),
        name,
        opened_at: now,
        recipe_count,
    };
    let mut list = read_recents_raw();
    list.retain(|w| w.path != canonical);
    list.insert(0, entry);
    list.truncate(10);
    write_recents(&list)
}

/// The workspace's display name for the recents sidecar and switcher
/// header. Prefers the manifest `name` (publish slug) — split after the
/// last `/` so `dima/zen-leaf` shows as `zen-leaf` in the UI. Falls
/// back to the directory basename when the manifest has no name.
pub fn derive_workspace_name(ws: &Workspace) -> String {
    if let Some(name) = ws.manifest.name.as_deref() {
        let trimmed = name.trim();
        if !trimmed.is_empty() {
            return trimmed.rsplit('/').next().unwrap_or(trimmed).to_string();
        }
    }
    ws.root
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| ws.root.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_manifest(root: &Path) {
        fs::write(
            root.join("forage.toml"),
            "description = \"\"\ncategory = \"\"\ntags = []\n",
        )
        .unwrap();
    }

    fn make_legacy_recipe(root: &Path, slug: &str, header_name: &str) {
        let dir = root.join(slug);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("recipe.forage"),
            format!("recipe \"{header_name}\"\nengine http\n"),
        )
        .unwrap();
    }

    fn make_flat_recipe(root: &Path, file_stem: &str, header_name: &str) {
        fs::write(
            root.join(format!("{file_stem}.forage")),
            format!("recipe \"{header_name}\"\nengine http\n"),
        )
        .unwrap();
    }

    /// A recipe whose source file is still at the pre-Phase-10
    /// `<root>/<slug>/recipe.forage` location is an unmigrated
    /// workspace. Recipe-scoped commands refuse to act and surface a
    /// `forage migrate` prompt — Studio doesn't treat a half-shape
    /// workspace as workable.
    #[test]
    fn delete_unmigrated_recipe_surfaces_migration_prompt() {
        let tmp = tempfile::tempdir().unwrap();
        write_manifest(tmp.path());
        make_legacy_recipe(tmp.path(), "to-delete", "to-delete");

        let ws = forage_core::workspace::load(tmp.path()).unwrap();
        let err = delete_recipe(&ws, "to-delete").unwrap_err();
        assert!(
            err.to_string().contains("forage migrate"),
            "expected migrate prompt; got {err}"
        );
        // The file is left in place — Studio doesn't mutate
        // unmigrated state.
        assert!(tmp.path().join("to-delete/recipe.forage").exists());
    }

    /// A recipe named "remedy" in the flat `<root>/remedy.forage`
    /// layout takes only the `.forage` file with it; the workspace
    /// root and unrelated files stay intact.
    #[test]
    fn delete_removes_flat_recipe_file() {
        let tmp = tempfile::tempdir().unwrap();
        write_manifest(tmp.path());
        make_flat_recipe(tmp.path(), "remedy", "remedy");
        fs::write(tmp.path().join("cannabis.forage"), "share type X { id: String }\n").unwrap();

        let ws = forage_core::workspace::load(tmp.path()).unwrap();
        delete_recipe(&ws, "remedy").unwrap();

        assert!(!tmp.path().join("remedy.forage").exists());
        assert!(tmp.path().join("cannabis.forage").exists());
    }

    /// A flat-shape recipe whose header name differs from its file
    /// basename — `foo.forage` containing `recipe "bar"` — resolves
    /// off the header name, not the basename.
    #[test]
    fn delete_resolves_by_header_name_not_basename() {
        let tmp = tempfile::tempdir().unwrap();
        write_manifest(tmp.path());
        make_flat_recipe(tmp.path(), "foo", "bar");

        let ws = forage_core::workspace::load(tmp.path()).unwrap();
        delete_recipe(&ws, "bar").unwrap();

        assert!(!tmp.path().join("foo.forage").exists());
    }

    /// A request to delete a recipe whose header doesn't appear in
    /// the workspace surfaces NotFound rather than a generic IO
    /// error, so the UI can render a precise "no such recipe" toast.
    #[test]
    fn delete_unknown_recipe_is_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        write_manifest(tmp.path());
        let ws = forage_core::workspace::load(tmp.path()).unwrap();
        let err = delete_recipe(&ws, "ghost").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    /// The workspace scanner refuses to follow symlinked dirs and
    /// file symlinks, so any recipe content reachable only through a
    /// symlink is invisible to the recipe index in the first place —
    /// `delete_recipe` then returns NotFound and the file outside
    /// the workspace remains untouched.
    #[cfg(unix)]
    #[test]
    fn delete_symlinked_recipe_is_not_found_and_preserves_outside() {
        let tmp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        write_manifest(tmp.path());
        fs::write(outside.path().join("important.txt"), "DO NOT DELETE").unwrap();
        fs::write(
            outside.path().join("escapee.forage"),
            "recipe \"escapee\"\nengine http\n",
        )
        .unwrap();
        std::os::unix::fs::symlink(outside.path(), tmp.path().join("evil")).unwrap();

        let ws = forage_core::workspace::load(tmp.path()).unwrap();
        let err = delete_recipe(&ws, "escapee").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        assert!(outside.path().join("important.txt").exists());
        assert!(outside.path().join("escapee.forage").exists());
    }

    /// A fresh workspace's first scaffold lands at
    /// `<root>/untitled-1.forage` with a recipe header keyed off the
    /// same name, and `create_recipe` hands the name back to the
    /// caller.
    #[test]
    fn create_recipe_scaffolds_flat_untitled_one() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write_manifest(root);

        let name = create_recipe(root, None).unwrap();
        assert_eq!(name, "untitled-1");
        let path = root.join("untitled-1.forage");
        assert!(path.is_file(), "scaffolded file must exist: {path:?}");
        // No legacy directory is created alongside the flat file.
        assert!(!root.join("untitled-1").exists());

        let body = fs::read_to_string(&path).unwrap();
        assert!(
            body.starts_with("recipe \"untitled-1\" engine http"),
            "scaffolded body must declare the matching recipe header: {body:?}"
        );

        // The workspace scanner finds the new recipe by its header
        // name; the file stem doubles as the addressable identity.
        let ws = forage_core::workspace::load(root).unwrap();
        assert!(ws.recipe_by_name("untitled-1").is_some());
    }

    /// Successive scaffolds advance the `untitled-N` counter past the
    /// next-available number, regardless of which earlier names were
    /// created or removed.
    #[test]
    fn create_recipe_picks_next_available_untitled() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write_manifest(root);

        assert_eq!(create_recipe(root, None).unwrap(), "untitled-1");
        assert_eq!(create_recipe(root, None).unwrap(), "untitled-2");
        assert_eq!(create_recipe(root, None).unwrap(), "untitled-3");

        // Removing untitled-2 from the middle of the sequence frees
        // its slot — the next scaffold reuses it before bumping past
        // untitled-3.
        fs::remove_file(root.join("untitled-2.forage")).unwrap();
        assert_eq!(create_recipe(root, None).unwrap(), "untitled-2");
        assert_eq!(create_recipe(root, None).unwrap(), "untitled-4");
    }

    /// A name override scaffolds at `<root>/<name>.forage` directly.
    /// Refusing to overwrite an existing file means an authored
    /// recipe can't be silently clobbered by a "New" action.
    #[test]
    fn create_recipe_with_name_override_refuses_to_overwrite() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write_manifest(root);

        let name = create_recipe(root, Some("cannabis")).unwrap();
        assert_eq!(name, "cannabis");
        assert!(root.join("cannabis.forage").is_file());

        let err = create_recipe(root, Some("cannabis")).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
    }

    /// Lay out a synthetic workspace and verify the tree shape and
    /// per-file classifications. Captures and snapshots live at
    /// `_fixtures/<recipe>.jsonl` / `_snapshots/<recipe>.json` — the
    /// data dirs the workspace loader keeps out of the source scan.
    #[test]
    fn build_file_tree_classifies_and_shapes_workspace() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        // Files at the root.
        fs::write(
            root.join("forage.toml"),
            "description = \"\"\ncategory = \"\"\ntags = []\n",
        )
        .unwrap();
        fs::write(
            root.join("cannabis.forage"),
            "type Dispensary { id: String }\n",
        )
        .unwrap();
        // Hidden entries — the daemon's working dir and a dotfile —
        // must be skipped.
        fs::create_dir_all(root.join(".forage").join("data")).unwrap();
        fs::write(root.join(".forage").join("daemon.sqlite"), "").unwrap();
        fs::write(root.join(".DS_Store"), "").unwrap();

        // Recipe at the root + its captures and snapshot in the
        // shared data dirs.
        fs::write(
            root.join("trilogy-rec.forage"),
            "recipe \"trilogy-rec\"\nengine http\n",
        )
        .unwrap();
        fs::create_dir_all(root.join("_fixtures")).unwrap();
        fs::write(root.join("_fixtures").join("trilogy-rec.jsonl"), "").unwrap();
        fs::create_dir_all(root.join("_snapshots")).unwrap();
        fs::write(root.join("_snapshots").join("trilogy-rec.json"), "{}").unwrap();
        fs::write(root.join("README.md"), "").unwrap();

        let tree = build_file_tree(root).unwrap();
        let FileNode::Folder { children, .. } = tree else {
            panic!("expected Folder at root");
        };

        for child in &children {
            match child {
                FileNode::File { name, .. } | FileNode::Folder { name, .. } => {
                    assert!(
                        !name.starts_with('.'),
                        "expected hidden entry to be skipped, got {name}"
                    );
                }
            }
        }

        let mut by_name: BTreeMap<String, FileKind> = BTreeMap::new();
        let mut folders: BTreeMap<String, Vec<FileNode>> = BTreeMap::new();
        for child in children {
            match child {
                FileNode::File {
                    name, file_kind, ..
                } => {
                    by_name.insert(name, file_kind);
                }
                FileNode::Folder { name, children, .. } => {
                    folders.insert(name, children);
                }
            }
        }

        assert_eq!(
            by_name.get("forage.toml").copied(),
            Some(FileKind::Manifest)
        );
        assert_eq!(
            by_name.get("cannabis.forage").copied(),
            Some(FileKind::Declarations)
        );
        assert_eq!(
            by_name.get("trilogy-rec.forage").copied(),
            Some(FileKind::Declarations),
            "a flat .forage at the root with a header is still classified as Declarations \
             by the path-based classifier; FileKind doesn't inspect file contents",
        );
        assert_eq!(by_name.get("README.md").copied(), Some(FileKind::Other));

        let fixtures = folders.get("_fixtures").expect("_fixtures folder");
        let fixture_kind = fixtures
            .iter()
            .find_map(|n| match n {
                FileNode::File {
                    name, file_kind, ..
                } if name == "trilogy-rec.jsonl" => Some(*file_kind),
                _ => None,
            })
            .expect("trilogy-rec.jsonl");
        assert_eq!(fixture_kind, FileKind::Fixture);

        let snapshots = folders.get("_snapshots").expect("_snapshots folder");
        let snapshot_kind = snapshots
            .iter()
            .find_map(|n| match n {
                FileNode::File {
                    name, file_kind, ..
                } if name == "trilogy-rec.json" => Some(*file_kind),
                _ => None,
            })
            .expect("trilogy-rec.json");
        assert_eq!(snapshot_kind, FileKind::Snapshot);
    }

    /// FileNode.path is workspace-relative. The frontend derives a
    /// recipe slug from path shape; absolute paths break every
    /// path-aware action (Run, Replay, context menu) in the UI.
    #[test]
    fn build_file_tree_paths_are_workspace_relative() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::write(
            root.join("forage.toml"),
            "description = \"\"\ncategory = \"\"\ntags = []\n",
        )
        .unwrap();
        fs::write(root.join("cannabis.forage"), "").unwrap();
        fs::write(root.join("trilogy-rec.forage"), "").unwrap();
        fs::create_dir_all(root.join("_fixtures")).unwrap();
        fs::write(root.join("_fixtures").join("trilogy-rec.jsonl"), "").unwrap();

        let tree = build_file_tree(root).unwrap();
        let FileNode::Folder {
            path: root_path,
            children,
            ..
        } = &tree
        else {
            panic!("expected Folder at root");
        };
        assert_eq!(root_path.as_os_str(), "");

        fn walk(node: &FileNode, out: &mut Vec<(String, PathBuf)>) {
            match node {
                FileNode::File { name, path, .. } => {
                    out.push((name.clone(), path.clone()));
                }
                FileNode::Folder {
                    name,
                    path,
                    children,
                } => {
                    out.push((name.clone(), path.clone()));
                    for c in children {
                        walk(c, out);
                    }
                }
            }
        }
        let mut all: Vec<(String, PathBuf)> = Vec::new();
        for c in children {
            walk(c, &mut all);
        }
        let by_name: BTreeMap<String, PathBuf> = all.into_iter().collect();

        assert_eq!(by_name["forage.toml"], PathBuf::from("forage.toml"));
        assert_eq!(by_name["cannabis.forage"], PathBuf::from("cannabis.forage"));
        assert_eq!(
            by_name["trilogy-rec.forage"],
            PathBuf::from("trilogy-rec.forage")
        );
        assert_eq!(by_name["_fixtures"], PathBuf::from("_fixtures"));
        assert_eq!(
            by_name["trilogy-rec.jsonl"],
            PathBuf::from("_fixtures/trilogy-rec.jsonl")
        );
    }

    /// Folders come before files in a deterministic order. Stable
    /// ordering matters for diffing the tree across refetches in the
    /// UI — a churn-y order would force needless re-renders.
    #[test]
    fn build_file_tree_orders_folders_first() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::write(
            root.join("forage.toml"),
            "description = \"\"\ncategory = \"\"\ntags = []\n",
        )
        .unwrap();
        fs::write(root.join("z.forage"), "").unwrap();
        fs::create_dir_all(root.join("a-folder")).unwrap();
        fs::write(root.join("a-folder").join("inside.txt"), "").unwrap();

        let tree = build_file_tree(root).unwrap();
        let FileNode::Folder { children, .. } = tree else {
            panic!("expected Folder at root");
        };
        let names: Vec<_> = children
            .iter()
            .map(|c| match c {
                FileNode::File { name, .. } | FileNode::Folder { name, .. } => name.clone(),
            })
            .collect();
        let folder_idx = names.iter().position(|n| n == "a-folder").unwrap();
        let toml_idx = names.iter().position(|n| n == "forage.toml").unwrap();
        let z_idx = names.iter().position(|n| n == "z.forage").unwrap();
        assert!(folder_idx < toml_idx);
        assert!(folder_idx < z_idx);
    }

    #[test]
    fn workspace_info_carries_manifest_summary() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::write(
            root.join("forage.toml"),
            r#"name = "dima/cannabis"
            description = "Cannabis-domain shared types"
            category = "shared-types"
            tags = ["cannabis"]
            [deps]
            "dima/shared-types" = 3
            "#,
        )
        .unwrap();
        let ws = forage_core::workspace::load(root).unwrap();
        let info = WorkspaceInfo::from_workspace(&ws);
        assert_eq!(info.name.as_deref(), Some("dima/cannabis"));
        assert_eq!(info.deps.get("dima/shared-types"), Some(&3));
    }

    // -----------------------------------------------------------------
    // Recents sidecar tests.
    //
    // Each test stamps a unique `FORAGE_DATA_DIR` via tempdir so they
    // don't share state with each other or with the user's real
    // recents file. The env var is read on every `recents_path()` call,
    // so as long as we set it before invoking the helpers the override
    // wins.
    // -----------------------------------------------------------------

    /// `cargo test` runs tests on a thread pool; setting a process-wide
    /// env var from multiple threads concurrently produces interleaved
    /// reads. Serialize through a static mutex so each recents test
    /// gets exclusive use of `FORAGE_DATA_DIR` for its duration.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn with_data_dir<F: FnOnce()>(f: F) {
        let _guard = ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let prev = std::env::var("FORAGE_DATA_DIR").ok();
        // Safety: the mutex guarantees we're the only thread touching
        // this env var during the call.
        unsafe {
            std::env::set_var("FORAGE_DATA_DIR", tmp.path());
        }
        f();
        unsafe {
            match prev {
                Some(v) => std::env::set_var("FORAGE_DATA_DIR", v),
                None => std::env::remove_var("FORAGE_DATA_DIR"),
            }
        }
    }

    #[test]
    fn recents_path_resolves_under_data_dir() {
        with_data_dir(|| {
            let p = recents_path();
            assert!(p.ends_with("recents.json"), "got {p:?}");
        });
    }

    #[test]
    fn read_recents_on_missing_file_returns_empty() {
        with_data_dir(|| {
            assert!(read_recents().is_empty());
        });
    }

    #[tracing_test::traced_test]
    #[test]
    fn read_recents_on_corrupt_json_logs_and_returns_empty() {
        with_data_dir(|| {
            let path = recents_path();
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, "not json {{{").unwrap();
            assert!(read_recents().is_empty());
            assert!(
                logs_contain("recents sidecar is not valid JSON"),
                "expected warn log for corrupt JSON; subscriber saw nothing"
            );
        });
    }

    #[test]
    fn record_recent_deduplicates_and_truncates() {
        with_data_dir(|| {
            // Twelve distinct workspaces; the 7th + 9th get re-recorded
            // later so we can check dedup and ordering.
            let dirs: Vec<_> = (0..12)
                .map(|_| {
                    let d = tempfile::tempdir().unwrap();
                    // Write a forage.toml so `exists()` returns true.
                    fs::write(
                        d.path().join("forage.toml"),
                        "description = \"\"\ncategory = \"\"\ntags = []\n",
                    )
                    .unwrap();
                    d
                })
                .collect();

            for (i, d) in dirs.iter().enumerate() {
                record_recent(d.path(), format!("w{i}"), i as u32).unwrap();
            }
            // Re-record one of the older ones — it should jump to the
            // front and the list should still be 10 entries.
            record_recent(dirs[3].path(), "w3-again".into(), 99).unwrap();

            let list = read_recents();
            assert_eq!(list.len(), 10);
            // Most-recent is the re-recorded entry.
            assert_eq!(list[0].name, "w3-again");
            assert_eq!(list[0].recipe_count, 99);
            // No duplicate of w3 anywhere else.
            let dupes = list.iter().filter(|w| w.path == list[0].path).count();
            assert_eq!(dupes, 1);
        });
    }

    #[test]
    fn read_recents_drops_entries_whose_path_is_missing() {
        with_data_dir(|| {
            let stable = tempfile::tempdir().unwrap();
            fs::write(
                stable.path().join("forage.toml"),
                "description = \"\"\ncategory = \"\"\ntags = []\n",
            )
            .unwrap();
            record_recent(stable.path(), "stable".into(), 1).unwrap();
            // Record a second entry, then delete the directory so the
            // recents file points at a now-missing path.
            let throwaway = tempfile::tempdir().unwrap();
            fs::write(
                throwaway.path().join("forage.toml"),
                "description = \"\"\ncategory = \"\"\ntags = []\n",
            )
            .unwrap();
            let throwaway_path = throwaway.path().to_path_buf();
            record_recent(&throwaway_path, "gone".into(), 0).unwrap();
            drop(throwaway);

            let list = read_recents();
            assert_eq!(list.len(), 1);
            assert_eq!(list[0].name, "stable");
        });
    }

    #[test]
    fn derive_workspace_name_prefers_manifest_publish_slug() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join("forage.toml"),
            "name = \"dima/zen-leaf\"\ndescription = \"\"\ncategory = \"\"\ntags = []\n",
        )
        .unwrap();
        let ws = forage_core::workspace::load(tmp.path()).unwrap();
        assert_eq!(derive_workspace_name(&ws), "zen-leaf");
    }

    #[test]
    fn derive_workspace_name_falls_back_to_basename() {
        let tmp = tempfile::tempdir().unwrap();
        // Create a child dir with a known name so the tempdir's
        // randomized suffix doesn't make this test flaky.
        let inner = tmp.path().join("my-recipes");
        fs::create_dir_all(&inner).unwrap();
        fs::write(
            inner.join("forage.toml"),
            "description = \"\"\ncategory = \"\"\ntags = []\n",
        )
        .unwrap();
        let ws = forage_core::workspace::load(&inner).unwrap();
        assert_eq!(derive_workspace_name(&ws), "my-recipes");
    }

}
