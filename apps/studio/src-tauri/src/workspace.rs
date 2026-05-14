//! Studio's filesystem-backed workspace at `~/Library/Forage/Recipes/`.
//!
//! Each recipe lives in `<workspace>/<slug>/recipe.forage`; the
//! workspace itself is marked by a `forage.toml` at the root and may
//! include header-less declarations files shared across recipes.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::Serialize;
use ts_rs::TS;

use forage_core::workspace::{MANIFEST_NAME, Manifest, Workspace, serialize_manifest};

/// On-disk location of the user's recipe workspace.
///
/// Honors `FORAGE_WORKSPACE_ROOT` first — useful for tests (sandbox
/// into a tempdir) and for users who want to point Studio at a repo
/// checkout instead of the OS-conventional workspace directory.
pub fn workspace_root() -> PathBuf {
    if let Ok(override_dir) = std::env::var("FORAGE_WORKSPACE_ROOT") {
        if !override_dir.is_empty() {
            return PathBuf::from(override_dir);
        }
    }
    if cfg!(target_os = "macos") {
        if let Some(home) = dirs::home_dir() {
            return home.join("Library").join("Forage").join("Recipes");
        }
    }
    if let Some(data) = dirs::data_dir() {
        return data.join("Forage").join("Recipes");
    }
    PathBuf::from(".forage-recipes")
}

/// Drop an empty `forage.toml` at `<workspace_root>/forage.toml` if it
/// doesn't exist. Studio calls this on app init so an existing folder
/// of recipes silently becomes a workspace on first launch.
pub fn ensure_workspace_manifest(root: &Path) -> std::io::Result<()> {
    fs::create_dir_all(root)?;
    let path = root.join(MANIFEST_NAME);
    if path.exists() {
        return Ok(());
    }
    let body = serialize_manifest(&Manifest::default())
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
    fs::write(&path, body)
}

pub fn recipe_path(slug: &str) -> PathBuf {
    workspace_root().join(slug).join("recipe.forage")
}

pub fn recipe_dir(slug: &str) -> PathBuf {
    workspace_root().join(slug)
}

pub fn create_recipe(template_slug: Option<&str>) -> std::io::Result<String> {
    let root = workspace_root();
    fs::create_dir_all(&root)?;
    // Find an `untitled-N` slug that doesn't exist yet.
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
            return Err(std::io::Error::other("too many untitled recipes"));
        }
    }
}

pub fn read_source(slug: &str) -> std::io::Result<String> {
    fs::read_to_string(recipe_path(slug))
}

/// Delete a recipe directory under the workspace root.
///
/// Refuses anything that isn't a single path segment (no slashes, no `..`),
/// so a malicious slug can't escape the workspace root with `../etc/passwd`.
/// The slug must already exist as a directory directly under the workspace.
pub fn delete_recipe(slug: &str) -> std::io::Result<()> {
    delete_recipe_in(&workspace_root(), slug)
}

/// Test-friendly variant of `delete_recipe` that takes an explicit root.
fn delete_recipe_in(root: &Path, slug: &str) -> std::io::Result<()> {
    if slug.is_empty() || slug.contains('/') || slug.contains('\\') || slug == "." || slug == ".." {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("invalid recipe slug: {slug:?}"),
        ));
    }
    let dir = root.join(slug);
    // Confirm the target sits inside the workspace root before deleting — a
    // hardlink or symlink would otherwise let us nuke unrelated content.
    let canonical = dir.canonicalize()?;
    let root_canonical = root.canonicalize()?;
    if !canonical.starts_with(&root_canonical) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("recipe path {canonical:?} escapes workspace root {root_canonical:?}"),
        ));
    }
    fs::remove_dir_all(&dir)
}

pub fn read_inputs(slug: &str) -> indexmap::IndexMap<String, serde_json::Value> {
    let path = recipe_dir(slug).join("fixtures").join("inputs.json");
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

pub fn read_captures(slug: &str) -> Vec<forage_replay::Capture> {
    let path = recipe_dir(slug).join("fixtures").join("captures.jsonl");
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
pub fn breakpoints_path() -> PathBuf {
    workspace_root().join("breakpoints.json")
}

pub fn read_breakpoints() -> io::Result<std::collections::HashMap<String, Vec<String>>> {
    let path = breakpoints_path();
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
    map: &std::collections::HashMap<String, Vec<String>>,
) -> std::io::Result<()> {
    let path = breakpoints_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let body = serde_json::to_string_pretty(map)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
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

#[allow(dead_code)]
pub fn ensure_path<P: AsRef<Path>>(p: P) -> std::io::Result<()> {
    if let Some(parent) = p.as_ref().parent() {
        fs::create_dir_all(parent)?;
    }
    Ok(())
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

        delete_recipe_in(tmp.path(), "to-delete").unwrap();

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
            let err = delete_recipe_in(tmp.path(), bad).unwrap_err();
            assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput, "slug {bad:?}");
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

        let err = delete_recipe_in(tmp.path(), "evil").unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
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
        // Random non-tracked file under the recipe dir.
        fs::write(recipe_dir.join("README.md"), "").unwrap();

        let tree = build_file_tree(root).unwrap();
        let FileNode::Folder { children, .. } = tree else {
            panic!("expected Folder at root");
        };

        // Hidden entries are gone.
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

        // Collect (relative-name, file_kind) pairs for assertions.
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

        // Root-level classification.
        assert_eq!(
            by_name.get("forage.toml").copied(),
            Some(FileKind::Manifest)
        );
        assert_eq!(
            by_name.get("cannabis.forage").copied(),
            Some(FileKind::Declarations)
        );
        // Recipe folder is present.
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
        // Random file is classified as Other, not silently dropped.
        assert_eq!(
            recipe_files.get("README.md").copied(),
            Some(FileKind::Other)
        );

        // Fixtures subfolder contains a fixture file.
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
        // Root folder's relative path is empty.
        assert_eq!(root_path.as_os_str(), "");

        // Collect (name, path) pairs at every depth.
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
        // a-folder is a directory; both .forage and forage.toml are
        // files. The directory must come first.
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
}
