//! Workspace loader: discovers a `forage.toml`, scans the directory tree
//! for `.forage` files, classifies each as a recipe or a declarations
//! file, and merges shared `type`/`enum` declarations (workspace-local
//! plus cached hub deps) into a single `TypeCatalog`.
//!
//! Discovery is an ancestor walk from a starting path. If no marker is
//! found, callers fall back to lonely-recipe mode — the recipe sees no
//! shared declarations.

pub mod manifest;

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::ast::{DeclarationsFile, Recipe, RecipeEnum, RecipeType, WorkspaceFile};
use crate::parse::{ParseError, parse_workspace_file};

pub use manifest::{
    LockedDep, Lockfile, Manifest, ManifestError, parse_lockfile, parse_manifest,
    serialize_lockfile, serialize_manifest,
};

/// The well-known manifest filename. Discovery walks ancestors looking
/// for one of these.
pub const MANIFEST_NAME: &str = "forage.toml";

/// The well-known lockfile filename written by `forage update`.
pub const LOCKFILE_NAME: &str = "forage.lock";

/// A discovered workspace: root path, parsed manifest, and the list of
/// `.forage` files inside the tree.
#[derive(Debug, Clone)]
pub struct Workspace {
    pub root: PathBuf,
    pub manifest: Manifest,
    pub files: Vec<WorkspaceFileEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceFileEntry {
    /// Absolute path to the file.
    pub path: PathBuf,
    pub kind: WorkspaceFileKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceFileKind {
    /// A full recipe file. `slug` is the slug used for runs / hub
    /// publishing — derived from the parent directory name when the
    /// recipe sits at `<slug>/recipe.forage`, or the filename stem
    /// when it lives at workspace root.
    Recipe { slug: String },
    /// A header-less declarations file. Contributes to the catalog but
    /// is never run on its own.
    Declarations,
    /// A `.forage` file that exists on disk but failed to parse. The
    /// entry is retained (rather than silently dropped) so the daemon
    /// can surface a broken-recipe status — a single bad file shouldn't
    /// take down the whole workspace, but it also shouldn't disappear
    /// from the user's view. `slug` is best-effort from path layout;
    /// the parser couldn't tell us whether the file was *meant* to be
    /// a recipe or a declarations file.
    Broken { slug: Option<String>, error: String },
}

/// Merged type/enum namespace for a recipe-validation pass. Built by
/// `Workspace::catalog(recipe_path)`: workspace declarations files first,
/// then cached hub-dep declarations, then the recipe-local declarations
/// (last writer wins inside the recipe-local pass).
#[derive(Debug, Clone, Default)]
pub struct TypeCatalog {
    pub types: HashMap<String, RecipeType>,
    pub enums: HashMap<String, RecipeEnum>,
}

/// Wire shape of a `TypeCatalog`. `TypeCatalog` itself isn't `Serialize`
/// because its component types carry transient state we'd rather not
/// stabilize on the wire; this struct is the deployable snapshot the
/// daemon stores alongside a deployed recipe's source so execution
/// doesn't have to re-resolve declarations from disk.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SerializableCatalog {
    pub types: HashMap<String, RecipeType>,
    pub enums: HashMap<String, RecipeEnum>,
}

impl From<TypeCatalog> for SerializableCatalog {
    fn from(cat: TypeCatalog) -> Self {
        Self {
            types: cat.types,
            enums: cat.enums,
        }
    }
}

impl From<SerializableCatalog> for TypeCatalog {
    fn from(cat: SerializableCatalog) -> Self {
        Self {
            types: cat.types,
            enums: cat.enums,
        }
    }
}

impl TypeCatalog {
    pub fn ty(&self, name: &str) -> Option<&RecipeType> {
        self.types.get(name)
    }
    pub fn recipe_enum(&self, name: &str) -> Option<&RecipeEnum> {
        self.enums.get(name)
    }

    /// Build a catalog from a single recipe's local types — what
    /// lonely-recipe mode uses when no workspace surrounds the file.
    pub fn from_recipe(recipe: &Recipe) -> Self {
        let mut cat = Self::default();
        cat.merge_recipe_local(recipe);
        cat
    }

    fn merge_decls(&mut self, decls: &DeclarationsFile) {
        for t in &decls.types {
            self.types.insert(t.name.clone(), t.clone());
        }
        for e in &decls.enums {
            self.enums.insert(e.name.clone(), e.clone());
        }
    }

    fn merge_recipe_local(&mut self, recipe: &Recipe) {
        for t in &recipe.types {
            self.types.insert(t.name.clone(), t.clone());
        }
        for e in &recipe.enums {
            self.enums.insert(e.name.clone(), e.clone());
        }
    }
}

#[derive(Debug, Error)]
pub enum WorkspaceError {
    #[error("workspace root not readable: {0}")]
    Io(#[from] io::Error),
    #[error("malformed manifest at {path}: {source}")]
    Manifest {
        path: PathBuf,
        #[source]
        source: ManifestError,
    },
    #[error("declarations file at {path} failed to parse: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: ParseError,
    },
    #[error("recipe at {path} failed to parse: {source}")]
    RecipePathInvalid {
        path: PathBuf,
        #[source]
        source: ParseError,
    },
    #[error("expected a recipe at {path} but found a header-less declarations file")]
    ExpectedRecipe { path: PathBuf },
    #[error(
        "type '{name}' is declared in multiple workspace declarations files: {}",
        paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ")
    )]
    DuplicateType { name: String, paths: Vec<PathBuf> },
    #[error(
        "enum '{name}' is declared in multiple workspace declarations files: {}",
        paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ")
    )]
    DuplicateEnum { name: String, paths: Vec<PathBuf> },
}

/// Walk ancestors of `start` looking for a `forage.toml`. Returns
/// `None` for lonely-recipe mode (no marker anywhere up the tree).
pub fn discover(start: &Path) -> Option<Workspace> {
    let start = if start.is_file() {
        start.parent()?
    } else {
        start
    };
    let mut cur = Some(start.to_path_buf());
    while let Some(dir) = cur {
        let marker = dir.join(MANIFEST_NAME);
        if marker.is_file() {
            return load(&dir).ok();
        }
        cur = dir.parent().map(Path::to_path_buf);
    }
    None
}

/// Load (or re-load) a workspace rooted at `root`. `root` must contain
/// a `forage.toml`; the manifest is parsed and the directory tree is
/// scanned for `.forage` files. The root is canonicalized so callers
/// can compare roots by equality regardless of how the path was passed
/// in (relative, symlink, trailing slash, ...).
pub fn load(root: &Path) -> Result<Workspace, WorkspaceError> {
    let root = root.canonicalize()?;
    let manifest_path = root.join(MANIFEST_NAME);
    let manifest_src = fs::read_to_string(&manifest_path)?;
    let manifest = parse_manifest(&manifest_src).map_err(|source| WorkspaceError::Manifest {
        path: manifest_path.clone(),
        source,
    })?;
    let mut files = Vec::new();
    scan_dir(&root, &root, &mut files)?;
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(Workspace {
        root,
        manifest,
        files,
    })
}

/// Re-scan the workspace tree on disk and refresh `files`. Manifest is
/// re-read too so toggling `[deps]` outside Studio is picked up.
pub fn refresh(ws: &mut Workspace) -> Result<(), WorkspaceError> {
    let fresh = load(&ws.root)?;
    *ws = fresh;
    Ok(())
}

fn scan_dir(
    root: &Path,
    dir: &Path,
    out: &mut Vec<WorkspaceFileEntry>,
) -> Result<(), WorkspaceError> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        // Skip hidden directories (`.forage/`, `.git/`, etc.) and the
        // standard build/output sinks. Keeps the scan cheap on large
        // libraries.
        if name_str.starts_with('.') {
            continue;
        }
        let ft = entry.file_type()?;
        if ft.is_dir() {
            scan_dir(root, &path, out)?;
            continue;
        }
        if ft.is_file() && path.extension().is_some_and(|e| e == "forage") {
            let source = fs::read_to_string(&path)?;
            // A single broken recipe used to abort the entire workspace
            // load, which cascaded into "Studio won't even start" when
            // the daemon library held an unparseable file. Capture the
            // failure as a `Broken` entry instead — the engine won't
            // try to run it, but the daemon surfaces it through the
            // recipe-status API so the user can find and fix it. Slug
            // is derived from path layout where possible since the
            // parser can't tell us whether the file was *meant* to be
            // a recipe or a declarations file.
            let parsed = match parse_workspace_file(&source) {
                Ok(p) => p,
                Err(err) => {
                    let slug = derive_slug(root, &path);
                    out.push(WorkspaceFileEntry {
                        path,
                        kind: WorkspaceFileKind::Broken {
                            slug,
                            error: err.to_string(),
                        },
                    });
                    continue;
                }
            };
            let kind = match parsed {
                WorkspaceFile::Recipe(_) => match derive_slug(root, &path) {
                    Some(slug) => WorkspaceFileKind::Recipe { slug },
                    None => WorkspaceFileKind::Broken {
                        slug: None,
                        error: "recipe path has no recoverable file stem".into(),
                    },
                },
                WorkspaceFile::Declarations(_) => WorkspaceFileKind::Declarations,
            };
            out.push(WorkspaceFileEntry { path, kind });
        }
    }
    Ok(())
}

/// Slug for a recipe at `path` inside `root`. The canonical layout is
/// `<root>/<slug>/recipe.forage` (matches the existing Studio library
/// layout); loose recipes fall back to the file stem. Returns `None`
/// when the path has no recoverable stem — callers surface that as an
/// invalid-path workspace error rather than papering over it.
fn derive_slug(root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).unwrap_or(path);
    let components: Vec<_> = rel.components().collect();
    if components.len() == 2 {
        if let (Some(dir), Some(file)) = (components.first(), components.get(1)) {
            if file.as_os_str() == "recipe.forage" {
                return Some(dir.as_os_str().to_string_lossy().into_owned());
            }
        }
    }
    path.file_stem().map(|s| s.to_string_lossy().into_owned())
}

impl Workspace {
    /// Look up a recipe entry by slug.
    pub fn recipe_for(&self, slug: &str) -> Option<&WorkspaceFileEntry> {
        self.files.iter().find(|f| matches!(&f.kind, WorkspaceFileKind::Recipe { slug: s } if s == slug))
    }

    /// All recipe entries in the workspace, in path order.
    pub fn recipes(&self) -> impl Iterator<Item = &WorkspaceFileEntry> {
        self.files
            .iter()
            .filter(|f| matches!(f.kind, WorkspaceFileKind::Recipe { .. }))
    }

    /// All declarations-file entries in the workspace, in path order.
    pub fn declarations(&self) -> impl Iterator<Item = &WorkspaceFileEntry> {
        self.files
            .iter()
            .filter(|f| matches!(f.kind, WorkspaceFileKind::Declarations))
    }

    /// All `.forage` files that failed to parse, in path order. Used by
    /// the daemon's recipe-status surface to flag unparseable files in
    /// the editor without aborting workspace load.
    pub fn broken(&self) -> impl Iterator<Item = &WorkspaceFileEntry> {
        self.files
            .iter()
            .filter(|f| matches!(f.kind, WorkspaceFileKind::Broken { .. }))
    }

    /// Build a merged `TypeCatalog` for validating one recipe in this
    /// workspace.
    ///
    /// Merge order: workspace declarations files → cached hub-dep
    /// declarations files → recipe-local declarations (last writer wins
    /// in the final pass, so a recipe can shadow a shared type by
    /// redeclaring it).
    ///
    /// Two workspace-level declarations files declaring the same name
    /// is a hard error — the namespace would be ambiguous and the user
    /// has to choose.
    ///
    /// `read` controls how shared declarations files are loaded. Pass
    /// `read_to_string`-backed [`Workspace::catalog_from_disk`] to read
    /// straight off disk; pass a closure that prefers live buffer
    /// contents (LSP, Studio) when an editor has unsaved edits.
    pub fn catalog<R>(&self, recipe: &Recipe, read: R) -> Result<TypeCatalog, WorkspaceError>
    where
        R: Fn(&Path) -> io::Result<String>,
    {
        let mut cat = TypeCatalog::default();
        let mut type_origins: HashMap<String, PathBuf> = HashMap::new();
        let mut enum_origins: HashMap<String, PathBuf> = HashMap::new();

        // 1. Workspace declarations files.
        for entry in self.declarations() {
            let decls = read_declarations(&entry.path, &read)?;
            for t in &decls.types {
                if let Some(prev) = type_origins.get(&t.name) {
                    return Err(WorkspaceError::DuplicateType {
                        name: t.name.clone(),
                        paths: vec![prev.clone(), entry.path.clone()],
                    });
                }
                type_origins.insert(t.name.clone(), entry.path.clone());
                cat.types.insert(t.name.clone(), t.clone());
            }
            for e in &decls.enums {
                if let Some(prev) = enum_origins.get(&e.name) {
                    return Err(WorkspaceError::DuplicateEnum {
                        name: e.name.clone(),
                        paths: vec![prev.clone(), entry.path.clone()],
                    });
                }
                enum_origins.insert(e.name.clone(), entry.path.clone());
                cat.enums.insert(e.name.clone(), e.clone());
            }
        }

        // 2. Cached hub-dep declarations files. Each dep is a package
        //    directory under the cache root; we treat every `.forage`
        //    file inside it that has no recipe header as a shared
        //    declarations file. Collisions between deps shadow earlier
        //    deps in iteration order — TOML preserves insertion in
        //    BTreeMap by key, which is sorted, so behavior is
        //    deterministic.
        for (slug, version) in &self.manifest.deps {
            let Some(pkg) = crate::workspace::resolve_dep(slug, *version) else {
                continue;
            };
            scan_package_declarations(&pkg, &mut cat)?;
        }

        // 3. Recipe-local declarations.
        cat.merge_recipe_local(recipe);
        Ok(cat)
    }

    /// Disk-backed convenience over [`Workspace::catalog`]: reads the
    /// recipe file from `recipe_path`, parses it, and routes shared
    /// declarations files through `fs::read_to_string`. Use this from
    /// the CLI and any other caller that doesn't carry the parsed
    /// recipe in memory.
    pub fn catalog_from_disk(&self, recipe_path: &Path) -> Result<TypeCatalog, WorkspaceError> {
        let recipe_src = fs::read_to_string(recipe_path).map_err(WorkspaceError::Io)?;
        let recipe = match parse_workspace_file(&recipe_src) {
            Ok(WorkspaceFile::Recipe(r)) => r,
            Ok(WorkspaceFile::Declarations(_)) => {
                return Err(WorkspaceError::ExpectedRecipe {
                    path: recipe_path.to_path_buf(),
                });
            }
            Err(source) => {
                return Err(WorkspaceError::RecipePathInvalid {
                    path: recipe_path.to_path_buf(),
                    source,
                });
            }
        };
        self.catalog(&recipe, |p| fs::read_to_string(p))
    }
}

fn read_declarations<R>(path: &Path, read: &R) -> Result<DeclarationsFile, WorkspaceError>
where
    R: Fn(&Path) -> io::Result<String>,
{
    let src = read(path)?;
    match parse_workspace_file(&src).map_err(|source| WorkspaceError::Parse {
        path: path.to_path_buf(),
        source,
    })? {
        WorkspaceFile::Declarations(d) => Ok(d),
        // Should never happen — `scan_dir` already classified this as
        // a declarations file. Re-read defensively, treating mis-tagged
        // recipes as empty.
        WorkspaceFile::Recipe(_) => Ok(DeclarationsFile::default()),
    }
}

fn scan_package_declarations(pkg: &Path, cat: &mut TypeCatalog) -> Result<(), WorkspaceError> {
    if !pkg.is_dir() {
        return Ok(());
    }
    for entry in walkdir::WalkDir::new(pkg).into_iter() {
        let entry = entry.map_err(|e| io::Error::other(format!("{e}")))?;
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "forage") {
            continue;
        }
        let Ok(src) = fs::read_to_string(path) else {
            continue;
        };
        if let Ok(WorkspaceFile::Declarations(d)) = parse_workspace_file(&src) {
            cat.merge_decls(&d);
        }
    }
    Ok(())
}

// --- Hub cache resolution -------------------------------------------------

/// Where hub-published packages are cached on disk. `~/Library/Forage/
/// Cache/hub/` on macOS; the platform data dir's `Forage/Cache/hub/`
/// elsewhere. Override with `FORAGE_HUB_CACHE` (tests, alternative
/// installs).
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

/// On-disk location of a fetched hub package, or `None` when the
/// package isn't cached. Layout:
/// `<cache>/<author>/<slug>/<version>/`.
pub fn resolve_dep(slug: &str, version: u32) -> Option<PathBuf> {
    let (author, name) = slug.split_once('/')?;
    let dir = hub_cache_root()
        .join(author)
        .join(name)
        .join(version.to_string());
    if dir.is_dir() { Some(dir) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(path: &Path, body: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, body).unwrap();
    }

    /// Minimal valid manifest for tests that don't care about its
    /// contents — every required field present with empty values, no
    /// publishable `name`.
    const STARTER_MANIFEST: &str = "description = \"\"\ncategory = \"\"\ntags = []\n";

    #[test]
    fn discover_walks_ancestors() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(&root.join(MANIFEST_NAME), STARTER_MANIFEST);
        let nested = root.join("a").join("b");
        fs::create_dir_all(&nested).unwrap();
        let ws = discover(&nested).expect("should find workspace");
        assert_eq!(ws.root.canonicalize().unwrap(), root.canonicalize().unwrap());
    }

    #[test]
    fn discover_returns_none_without_marker() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(discover(tmp.path()).is_none());
    }

    #[test]
    fn workspace_classifies_recipe_and_declarations() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(&root.join(MANIFEST_NAME), STARTER_MANIFEST);
        write(
            &root.join("cannabis.forage"),
            "type Dispensary { id: String }\n",
        );
        write(
            &root.join("trilogy-rec").join("recipe.forage"),
            "recipe \"trilogy-rec\"\nengine http\n",
        );
        let ws = load(root).unwrap();
        let mut kinds: Vec<(String, &WorkspaceFileKind)> = ws
            .files
            .iter()
            .map(|f| {
                (
                    f.path.file_name().unwrap().to_string_lossy().into_owned(),
                    &f.kind,
                )
            })
            .collect();
        kinds.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(kinds.len(), 2);
        assert_eq!(kinds[0].0, "cannabis.forage");
        assert!(matches!(kinds[0].1, WorkspaceFileKind::Declarations));
        assert_eq!(kinds[1].0, "recipe.forage");
        assert!(
            matches!(kinds[1].1, WorkspaceFileKind::Recipe { slug } if slug == "trilogy-rec"),
            "got {:?}",
            kinds[1].1
        );
    }

    #[test]
    fn catalog_merges_workspace_decls_with_recipe_local() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(&root.join(MANIFEST_NAME), STARTER_MANIFEST);
        write(
            &root.join("cannabis.forage"),
            "type Dispensary { id: String, name: String }\n\
             type Product { id: String }\n",
        );
        let recipe_path = root.join("rec").join("recipe.forage");
        // Recipe-local override of Product adds a `terpenes` field.
        write(
            &recipe_path,
            "recipe \"rec\"\nengine http\n\
             type Product { id: String, terpenes: String? }\n",
        );
        let ws = load(root).unwrap();
        let cat = ws.catalog_from_disk(&recipe_path).unwrap();
        let dispensary = cat.ty("Dispensary").expect("Dispensary from workspace");
        assert_eq!(dispensary.fields.len(), 2);
        let product = cat.ty("Product").expect("Product");
        // Recipe-local override wins.
        assert_eq!(product.fields.len(), 2);
        assert!(product.fields.iter().any(|f| f.name == "terpenes"));
    }

    #[test]
    fn broken_recipe_is_captured_not_dropped() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(&root.join(MANIFEST_NAME), STARTER_MANIFEST);
        // One good recipe plus one syntactically-broken one. The
        // workspace must still load — the broken one becomes a
        // Broken entry so the daemon can surface it.
        write(
            &root.join("good").join("recipe.forage"),
            "recipe \"good\"\nengine http\n",
        );
        write(
            &root.join("bad").join("recipe.forage"),
            // Missing `engine` line + dangling `for` makes the parser
            // bail out. Exact error text is the parser's concern; we
            // just need *some* parse failure.
            "recipe \"bad\"\nfor in {{ }}\n",
        );

        let ws = load(root).expect("load must succeed despite broken file");
        let recipes: Vec<_> = ws.recipes().collect();
        let broken: Vec<_> = ws.broken().collect();

        assert_eq!(recipes.len(), 1);
        assert!(
            matches!(&recipes[0].kind, WorkspaceFileKind::Recipe { slug } if slug == "good"),
            "got {:?}",
            recipes[0].kind,
        );
        assert_eq!(broken.len(), 1);
        match &broken[0].kind {
            WorkspaceFileKind::Broken { slug, error } => {
                assert_eq!(slug.as_deref(), Some("bad"));
                assert!(!error.is_empty(), "error message should not be empty");
            }
            other => panic!("expected Broken, got {other:?}"),
        }
    }

    #[test]
    fn declarations_parse_failure_is_captured_too() {
        // A header-less `.forage` file with a syntax error also lands
        // in the Broken bucket — slug derived from filename stem since
        // it isn't under a `<slug>/recipe.forage` layout.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(&root.join(MANIFEST_NAME), STARTER_MANIFEST);
        write(
            &root.join("shared.forage"),
            // `type` without a name is a parse error.
            "type { id: String }\n",
        );
        let ws = load(root).unwrap();
        let broken: Vec<_> = ws.broken().collect();
        assert_eq!(broken.len(), 1);
        match &broken[0].kind {
            WorkspaceFileKind::Broken { slug, .. } => {
                assert_eq!(slug.as_deref(), Some("shared"));
            }
            other => panic!("expected Broken, got {other:?}"),
        }
    }

    #[test]
    fn duplicate_type_across_decls_files_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(&root.join(MANIFEST_NAME), STARTER_MANIFEST);
        write(&root.join("a.forage"), "type Product { id: String }\n");
        write(&root.join("b.forage"), "type Product { id: String }\n");
        let recipe_path = root.join("r").join("recipe.forage");
        write(&recipe_path, "recipe \"r\"\nengine http\n");
        let ws = load(root).unwrap();
        let err = ws.catalog_from_disk(&recipe_path).expect_err("should fail");
        match err {
            WorkspaceError::DuplicateType { name, paths } => {
                assert_eq!(name, "Product");
                assert_eq!(paths.len(), 2);
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    /// Round-trip a non-trivial catalog through `SerializableCatalog`
    /// and back. The daemon stores the wire shape on disk per deployed
    /// version, so any field that gets dropped here silently loses
    /// validation context at run time.
    #[test]
    fn serializable_catalog_round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(&root.join(MANIFEST_NAME), STARTER_MANIFEST);
        write(
            &root.join("cannabis.forage"),
            "type Dispensary { id: String, name: String? }\n\
             type Product { id: String, terpenes: String? }\n\
             enum MenuType { RECREATIONAL, MEDICAL }\n",
        );
        let recipe_path = root.join("rec").join("recipe.forage");
        write(
            &recipe_path,
            "recipe \"rec\"\nengine http\n\
             type Variant { id: String, weight: Double }\n\
             enum Status { ACTIVE, RETIRED }\n",
        );
        let ws = load(root).unwrap();
        let original = ws.catalog_from_disk(&recipe_path).unwrap();
        assert!(!original.types.is_empty());
        assert!(!original.enums.is_empty());

        let wire = SerializableCatalog::from(original.clone());
        let json = serde_json::to_string(&wire).unwrap();
        let decoded: SerializableCatalog = serde_json::from_str(&json).unwrap();
        let back = TypeCatalog::from(decoded);

        assert_eq!(back.types, original.types);
        assert_eq!(back.enums, original.enums);
    }
}
