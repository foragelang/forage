//! Filesystem helpers anchored on a workspace root. The root is whatever
//! the user opened — there's no longer a single global workspace.
//!
//! Each recipe lives in `<workspace>/<slug>/recipe.forage`; the
//! workspace itself is marked by a `forage.toml` at the root and may
//! include header-less declarations files shared across recipes.
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

pub fn recipe_path(root: &Path, slug: &str) -> PathBuf {
    root.join(slug).join("recipe.forage")
}

pub fn recipe_dir(root: &Path, slug: &str) -> PathBuf {
    root.join(slug)
}

pub fn create_recipe(root: &Path, template_slug: Option<&str>) -> io::Result<String> {
    fs::create_dir_all(root)?;
    let base = template_slug.unwrap_or("untitled");
    let mut n = 1;
    loop {
        let candidate = if n == 1 {
            format!("{base}-1")
        } else {
            format!("{base}-{n}")
        };
        let candidate_path = root.join(&candidate);
        if !candidate_path.exists() {
            fs::create_dir_all(candidate_path.join("fixtures"))?;
            let source = format!(
                "recipe \"{candidate}\"\nengine http\n\ntype Item {{\n    id: String\n}}\n\nstep list {{\n    method \"GET\"\n    url    \"https://example.com\"\n}}\n\nfor $i in $list.items[*] {{\n    emit Item {{\n        id ← $i.id\n    }}\n}}\n"
            );
            fs::write(candidate_path.join("recipe.forage"), source)?;
            return Ok(candidate);
        }
        n += 1;
        if n > 1000 {
            return Err(io::Error::other("too many untitled recipes"));
        }
    }
}

pub fn read_source(root: &Path, slug: &str) -> io::Result<String> {
    fs::read_to_string(recipe_path(root, slug))
}

/// Delete a recipe directory under the workspace root.
///
/// Refuses anything that isn't a single path segment (no slashes, no `..`),
/// so a malicious slug can't escape the workspace root with `../etc/passwd`.
/// The slug must already exist as a directory directly under the workspace.
pub fn delete_recipe(root: &Path, slug: &str) -> io::Result<()> {
    if slug.is_empty() || slug.contains('/') || slug.contains('\\') || slug == "." || slug == ".." {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid recipe slug: {slug:?}"),
        ));
    }
    let dir = root.join(slug);
    // Confirm the target sits inside the workspace root before deleting — a
    // hardlink or symlink would otherwise let us nuke unrelated content.
    let canonical = dir.canonicalize()?;
    let root_canonical = root.canonicalize()?;
    if !canonical.starts_with(&root_canonical) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("recipe path {canonical:?} escapes workspace root {root_canonical:?}"),
        ));
    }
    fs::remove_dir_all(&dir)
}

pub fn read_inputs(root: &Path, slug: &str) -> indexmap::IndexMap<String, serde_json::Value> {
    let path = recipe_dir(root, slug).join("fixtures").join("inputs.json");
    if !path.exists() {
        return indexmap::IndexMap::new();
    }
    let raw = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return indexmap::IndexMap::new(),
    };
    let v: serde_json::Value = serde_json::from_str(&raw).unwrap_or(serde_json::Value::Null);
    let mut out = indexmap::IndexMap::new();
    if let serde_json::Value::Object(o) = v {
        for (k, v) in o {
            out.insert(k, v);
        }
    }
    out
}

pub fn read_captures(root: &Path, slug: &str) -> Vec<forage_replay::Capture> {
    let path = recipe_dir(root, slug).join("fixtures").join("captures.jsonl");
    if !path.exists() {
        return Vec::new();
    }
    let raw = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(c) = serde_json::from_str::<forage_replay::Capture>(line) {
            out.push(c);
        }
    }
    out
}

/// Per-recipe breakpoint persistence. One JSON sidecar at
/// `<workspace_root>/breakpoints.json` keyed by recipe slug. The file is
/// missing until the user sets a first breakpoint, so the empty-map
/// case is the steady state for fresh workspaces.
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
pub fn read_secrets_from_env(recipe: &forage_core::Recipe) -> indexmap::IndexMap<String, String> {
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

/// File classification rules (DESIGN_HANDOFF.md):
///   `forage.toml`               → `Manifest`
///   `<slug>/recipe.forage`      → `Recipe`
///   `*.forage` at workspace root → `Declarations`
///   `<slug>/fixtures/*.json`    → `Fixture`
///   `<slug>/snapshot.json`      → `Snapshot`
///   everything else             → `Other`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum FileKind {
    Recipe,
    Declarations,
    Fixture,
    Snapshot,
    Manifest,
    Other,
}

/// Per-slug status combining the Studio's on-disk view (drafts) with
/// the daemon's view (deployed versions). The frontend renders these
/// side by side so the user can see "edited but not deployed" or
/// "deployed but the draft is missing" without joining the two views
/// itself.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct RecipeStatus {
    pub slug: String,
    pub draft: DraftState,
    pub deployed: DeployedState,
}

/// Whether a slug has a draft on disk and whether that draft parses.
/// `Missing` covers the deployed-but-no-draft case: the daemon
/// remembers a slug we no longer have a source file for.
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
/// `FileNode.path` is workspace-relative — the frontend's slug
/// derivation (`slugOf(path) == "<slug>"` when the path is exactly
/// `<slug>/recipe.forage`) and per-path equality checks across the
/// UI assume that shape. The root folder's relative path is empty;
/// the UI iterates its `children` directly and never selects the
/// root itself.
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

    // `.forage` files: recipes live under a folder, declarations sit
    // at the workspace root.
    if extension == "forage" {
        if components.len() == 1 {
            return FileKind::Declarations;
        }
        if components.len() == 2 && last == "recipe.forage" {
            return FileKind::Recipe;
        }
        // A `.forage` file two-deep that isn't `recipe.forage` is
        // unclassified — the workspace layout doesn't reserve that
        // slot. Better to surface it as Other than to silently
        // mis-tag it as a recipe.
        return FileKind::Other;
    }

    // `<slug>/fixtures/<name>.json` — captures / inputs / etc.
    if extension == "json" {
        if components.len() == 3 && components[1] == "fixtures" {
            return FileKind::Fixture;
        }
        if components.len() == 2 && last == "snapshot.json" {
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
        .map(|d| d.join("Forage"))
        .unwrap_or_else(|| PathBuf::from(".forage-data"))
        .join("recents.json")
}

/// Load the recents list, filtering out entries whose path no longer
/// exists on disk. Missing or corrupt sidecars degrade to an empty
/// list — recents are nice-to-have, not load-bearing.
///
/// Corrupt files log at `warn` and return empty rather than panicking;
/// the user can keep using the app and the next successful write
/// replaces the bad file.
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
        .filter(|w| {
            if w.path.exists() {
                true
            } else {
                tracing::debug!(path = %w.path.display(), "dropping recent workspace whose path no longer exists");
                false
            }
        })
        .collect()
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
    let parent = path.parent().unwrap_or(Path::new("."));
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

    fn make_recipe(root: &Path, slug: &str) {
        let dir = root.join(slug);
        fs::create_dir_all(dir.join("fixtures")).unwrap();
        fs::write(dir.join("recipe.forage"), "recipe \"x\"\nengine http\n").unwrap();
        fs::write(dir.join("fixtures").join("inputs.json"), "{}").unwrap();
    }

    #[test]
    fn delete_removes_directory_and_fixtures() {
        let tmp = tempfile::tempdir().unwrap();
        make_recipe(tmp.path(), "to-delete");
        assert!(tmp.path().join("to-delete/recipe.forage").exists());
        assert!(tmp.path().join("to-delete/fixtures/inputs.json").exists());

        delete_recipe(tmp.path(), "to-delete").unwrap();

        assert!(!tmp.path().join("to-delete").exists());
    }

    #[test]
    fn delete_rejects_path_traversal() {
        let tmp = tempfile::tempdir().unwrap();
        let siblings = tempfile::tempdir().unwrap();
        let victim = siblings.path().join("victim");
        fs::create_dir_all(&victim).unwrap();
        fs::write(victim.join("important.txt"), "DO NOT DELETE").unwrap();

        for bad in ["..", "../victim", "./x", "a/b", "a\\b", ""] {
            let err = delete_recipe(tmp.path(), bad).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::InvalidInput, "slug {bad:?}");
        }
        assert!(victim.join("important.txt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn delete_rejects_symlink_escape() {
        let tmp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("important.txt"), "DO NOT DELETE").unwrap();
        std::os::unix::fs::symlink(outside.path(), tmp.path().join("evil")).unwrap();

        let err = delete_recipe(tmp.path(), "evil").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(outside.path().join("important.txt").exists());
    }

    /// Lay out a synthetic workspace and verify the tree shape and
    /// per-file classifications match the rules in DESIGN_HANDOFF.md.
    #[test]
    fn build_file_tree_classifies_and_shapes_workspace() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        // Files at the root.
        fs::write(root.join("forage.toml"), "").unwrap();
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

        // One recipe with fixtures + snapshot.
        let recipe_dir = root.join("trilogy-rec");
        fs::create_dir_all(recipe_dir.join("fixtures")).unwrap();
        fs::write(
            recipe_dir.join("recipe.forage"),
            "recipe \"trilogy-rec\"\nengine http\n",
        )
        .unwrap();
        fs::write(recipe_dir.join("fixtures").join("inputs.json"), "{}").unwrap();
        fs::write(recipe_dir.join("snapshot.json"), "{}").unwrap();
        fs::write(recipe_dir.join("README.md"), "").unwrap();

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
        let recipe_children = folders.get("trilogy-rec").expect("recipe folder");

        let mut recipe_files: BTreeMap<String, FileKind> = BTreeMap::new();
        let mut recipe_subfolders: BTreeMap<String, Vec<FileNode>> = BTreeMap::new();
        for child in recipe_children {
            match child {
                FileNode::File {
                    name, file_kind, ..
                } => {
                    recipe_files.insert(name.clone(), *file_kind);
                }
                FileNode::Folder { name, children, .. } => {
                    recipe_subfolders.insert(name.clone(), children.clone());
                }
            }
        }
        assert_eq!(
            recipe_files.get("recipe.forage").copied(),
            Some(FileKind::Recipe)
        );
        assert_eq!(
            recipe_files.get("snapshot.json").copied(),
            Some(FileKind::Snapshot)
        );
        assert_eq!(
            recipe_files.get("README.md").copied(),
            Some(FileKind::Other)
        );

        let fixtures = recipe_subfolders.get("fixtures").expect("fixtures folder");
        let inputs = fixtures
            .iter()
            .find_map(|n| match n {
                FileNode::File {
                    name, file_kind, ..
                } if name == "inputs.json" => Some(*file_kind),
                _ => None,
            })
            .expect("inputs.json");
        assert_eq!(inputs, FileKind::Fixture);
    }

    /// FileNode.path is workspace-relative. The frontend derives a
    /// recipe slug from path shape (`<slug>/recipe.forage` → `<slug>`);
    /// absolute paths break that derivation and silently disable every
    /// slug-aware action (Run, Replay, context menu).
    #[test]
    fn build_file_tree_paths_are_workspace_relative() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::write(root.join("forage.toml"), "").unwrap();
        fs::write(root.join("cannabis.forage"), "").unwrap();
        let recipe_dir = root.join("trilogy-rec");
        fs::create_dir_all(recipe_dir.join("fixtures")).unwrap();
        fs::write(recipe_dir.join("recipe.forage"), "").unwrap();
        fs::write(recipe_dir.join("fixtures").join("inputs.json"), "{}").unwrap();

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
        assert_eq!(by_name["trilogy-rec"], PathBuf::from("trilogy-rec"));
        assert_eq!(
            by_name["recipe.forage"],
            PathBuf::from("trilogy-rec/recipe.forage")
        );
        assert_eq!(by_name["fixtures"], PathBuf::from("trilogy-rec/fixtures"));
        assert_eq!(
            by_name["inputs.json"],
            PathBuf::from("trilogy-rec/fixtures/inputs.json")
        );
    }

    /// Folders come before files in a deterministic order. Stable
    /// ordering matters for diffing the tree across refetches in the
    /// UI — a churn-y order would force needless re-renders.
    #[test]
    fn build_file_tree_orders_folders_first() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::write(root.join("forage.toml"), "").unwrap();
        fs::write(root.join("z.forage"), "").unwrap();
        fs::create_dir_all(root.join("a-folder")).unwrap();
        fs::write(root.join("a-folder").join("recipe.forage"), "").unwrap();

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

    #[test]
    fn read_recents_on_corrupt_json_returns_empty() {
        with_data_dir(|| {
            let path = recents_path();
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, "not json {{{").unwrap();
            // Should not panic; logs at warn.
            assert!(read_recents().is_empty());
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
                    fs::write(d.path().join("forage.toml"), "").unwrap();
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
            fs::write(stable.path().join("forage.toml"), "").unwrap();
            record_recent(stable.path(), "stable".into(), 1).unwrap();
            // Record a second entry, then delete the directory so the
            // recents file points at a now-missing path.
            let throwaway = tempfile::tempdir().unwrap();
            fs::write(throwaway.path().join("forage.toml"), "").unwrap();
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
            "name = \"dima/zen-leaf\"\n",
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
        fs::write(inner.join("forage.toml"), "").unwrap();
        let ws = forage_core::workspace::load(&inner).unwrap();
        assert_eq!(derive_workspace_name(&ws), "my-recipes");
    }

    #[test]
    fn new_workspace_rejects_dir_with_existing_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("forage.toml"), "").unwrap();
        let err = write_empty_manifest(tmp.path()).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
    }
}
