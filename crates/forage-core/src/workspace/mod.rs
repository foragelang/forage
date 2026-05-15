//! Workspace loader: discovers a `forage.toml`, scans the directory tree
//! for `.forage` files, parses each, and merges `share`d
//! `type`/`enum` declarations from every sibling file (plus cached hub
//! deps) into a single `TypeCatalog` for one focal file.
//!
//! Visibility is per declaration: `share type Foo { … }` joins the
//! workspace-wide catalog; a bare `type Foo { … }` stays file-scoped.
//! The focal file always contributes both its file-local and `share`d
//! declarations, so a recipe can see anything it declared at home plus
//! anything other files chose to publish.
//!
//! Cross-file `share` collisions are surfaced by
//! `validate_workspace_shared` in the validator, not by the catalog —
//! the catalog merges last-writer-wins so the focal file's own types
//! always shadow any same-name `share`d type from elsewhere.
//!
//! Discovery is an ancestor walk from a starting path. If no marker is
//! found, callers fall back to lonely-file mode — the file sees no
//! shared declarations.

pub mod manifest;

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::ast::{ForageFile, RecipeEnum, RecipeType};
use crate::parse::{ParseError, parse};

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

/// Whether a `.forage` file inside the workspace declares a recipe.
/// File location carries no semantics; this purely reflects the file's
/// content — does it have a `recipe "<name>" engine <kind>` header.
///
/// `slug` is the historical run / hub key. During this transition it is
/// derived from path layout (`<slug>/recipe.forage` → `slug`, otherwise
/// the file stem); Phase 3 of the simplification swaps this for the
/// recipe header name across the daemon, CLI, and Studio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceFileKind {
    /// A file that declares a recipe header. Runnable.
    Recipe { slug: String },
    /// A header-less file. Contributes shared declarations to the
    /// workspace catalog but is never run on its own.
    Declarations,
    /// A `.forage` file that exists on disk but failed to parse. The
    /// entry is retained (rather than silently dropped) so the daemon
    /// can surface a broken status — a single bad file shouldn't take
    /// down the whole workspace, but it also shouldn't disappear from
    /// the user's view.
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

    /// Build a catalog from one file's local types — what lonely-file
    /// mode uses when no workspace surrounds the file.
    pub fn from_file(file: &ForageFile) -> Self {
        let mut cat = Self::default();
        cat.merge_all(file);
        cat
    }

    /// Merge every type and enum declared in `file` into the catalog,
    /// last-writer-wins per name. Used for the focal file (which sees
    /// both its file-local and `share`d decls) and for hub-dep packages
    /// (where the `share` flag isn't author-controlled yet — see
    /// `scan_package_declarations`).
    pub fn merge_all(&mut self, file: &ForageFile) {
        for t in &file.types {
            self.types.insert(t.name.clone(), t.clone());
        }
        for e in &file.enums {
            self.enums.insert(e.name.clone(), e.clone());
        }
    }

    /// Merge only the `share`d types and enums from `file`. Used when
    /// folding a non-focal sibling into the workspace catalog: a bare
    /// `type Foo { … }` stays private to its declaring file.
    pub fn merge_shared(&mut self, file: &ForageFile) {
        for t in &file.types {
            if t.shared {
                self.types.insert(t.name.clone(), t.clone());
            }
        }
        for e in &file.enums {
            if e.shared {
                self.enums.insert(e.name.clone(), e.clone());
            }
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
    #[error("workspace file at {path} failed to parse: {source}")]
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
            // A single broken file used to abort the entire workspace
            // load, which cascaded into "Studio won't even start" when
            // the daemon library held an unparseable file. Capture the
            // failure as a `Broken` entry instead — the engine won't
            // try to run it, but the daemon surfaces it through the
            // recipe-status API so the user can find and fix it. Slug
            // is derived from path layout where possible.
            let parsed = match parse(&source) {
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
            // A file with a `recipe "<name>" engine <kind>` header is
            // runnable. Header-less files contribute declarations to
            // the workspace catalog but aren't run on their own.
            let kind = if parsed.recipe_header().is_some() {
                match derive_slug(root, &path) {
                    Some(slug) => WorkspaceFileKind::Recipe { slug },
                    None => WorkspaceFileKind::Broken {
                        slug: None,
                        error: "recipe path has no recoverable file stem".into(),
                    },
                }
            } else {
                WorkspaceFileKind::Declarations
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

    /// All `.forage` files that failed to parse, in path order. Used by
    /// the daemon's recipe-status surface to flag unparseable files in
    /// the editor without aborting workspace load.
    pub fn broken(&self) -> impl Iterator<Item = &WorkspaceFileEntry> {
        self.files
            .iter()
            .filter(|f| matches!(f.kind, WorkspaceFileKind::Broken { .. }))
    }

    /// Build a merged `TypeCatalog` for validating `file` in this
    /// workspace.
    ///
    /// Merge order:
    /// 1. `share`d types/enums from every other workspace file. Files
    ///    contribute only what they `share`; bare `type Foo { … }` stays
    ///    private to its declaring file.
    /// 2. Cached hub-dep declarations (currently treated as all-visible
    ///    — see `scan_package_declarations`).
    /// 3. Every type/enum in the focal `file`, file-local and `share`d
    ///    alike — a file always sees everything it declared at home.
    ///
    /// Last writer wins per name, so step 3 lets a focal file shadow a
    /// `share`d type from elsewhere by redeclaring it locally. Cross-file
    /// `share` collisions are surfaced by `validate_workspace_shared`
    /// in the validator (the catalog itself is silent about them so the
    /// LSP gets one diagnostic per collision instead of two).
    ///
    /// `read` controls how sibling files are loaded. Pass an
    /// `fs::read_to_string`-backed closure ([`Workspace::catalog_from_disk`])
    /// to read straight off disk; pass a closure that prefers live
    /// buffer contents (LSP, Studio) when an editor has unsaved edits.
    ///
    /// The focal file is identified by content, not path: any sibling
    /// whose `share`d decls overlap with the focal will simply be
    /// overwritten by step 3, so identifying the focal precisely doesn't
    /// matter for correctness.
    pub fn catalog<R>(&self, file: &ForageFile, read: R) -> Result<TypeCatalog, WorkspaceError>
    where
        R: Fn(&Path) -> io::Result<String>,
    {
        let mut cat = TypeCatalog::default();

        // 1. Every other workspace file contributes its `share`d
        //    declarations. Broken files are skipped here — they're
        //    already surfaced via `Workspace::broken()` for the daemon's
        //    status UI.
        for entry in &self.files {
            if !matches!(
                entry.kind,
                WorkspaceFileKind::Recipe { .. } | WorkspaceFileKind::Declarations
            ) {
                continue;
            }
            let parsed = read_workspace_file(&entry.path, &read)?;
            cat.merge_shared(&parsed);
        }

        // 2. Cached hub-dep declarations. Each dep is a package directory
        //    under the cache root; every header-less `.forage` file
        //    inside it contributes its types. Hub-deps don't yet have
        //    author-controlled `share` markers (the typed-hub program
        //    will revisit this), so all package-level types are visible
        //    to consumers. Collisions between deps shadow earlier deps
        //    in iteration order — `manifest.deps` is a BTreeMap keyed by
        //    slug, so iteration is deterministic.
        for (slug, version) in &self.manifest.deps {
            let Some(pkg) = crate::workspace::resolve_dep(slug, *version) else {
                continue;
            };
            scan_package_declarations(&pkg, &mut cat)?;
        }

        // 3. Focal file: every type/enum, file-local plus `share`d.
        //    Overwrites any same-name `share`d entry from step 1, which
        //    is what gives the focal file file-local precedence.
        cat.merge_all(file);
        Ok(cat)
    }

    /// Disk-backed convenience over [`Workspace::catalog`]: reads the
    /// recipe file from `recipe_path`, parses it, and routes shared
    /// declarations files through `fs::read_to_string`. Use this from
    /// the CLI and any other caller that doesn't carry the parsed
    /// recipe in memory.
    pub fn catalog_from_disk(&self, recipe_path: &Path) -> Result<TypeCatalog, WorkspaceError> {
        let recipe_src = fs::read_to_string(recipe_path).map_err(WorkspaceError::Io)?;
        let file = parse(&recipe_src).map_err(|source| WorkspaceError::RecipePathInvalid {
            path: recipe_path.to_path_buf(),
            source,
        })?;
        if file.recipe_header().is_none() {
            return Err(WorkspaceError::ExpectedRecipe {
                path: recipe_path.to_path_buf(),
            });
        }
        self.catalog(&file, |p| fs::read_to_string(p))
    }
}

/// Read and parse a workspace file, returning its `ForageFile`. Used
/// when folding sibling `share`d declarations into the focal catalog.
fn read_workspace_file<R>(path: &Path, read: &R) -> Result<ForageFile, WorkspaceError>
where
    R: Fn(&Path) -> io::Result<String>,
{
    let src = read(path)?;
    parse(&src).map_err(|source| WorkspaceError::Parse {
        path: path.to_path_buf(),
        source,
    })
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
        // Only header-less files contribute shared declarations from a
        // hub package. Recipe-bearing files inside a published package
        // are runnable inheritances; their types are file-local.
        //
        // TODO(typed-hub): hub-deps don't yet emit `share` markers, so
        // every type in a header-less package file is treated as
        // workspace-visible. The typed-hub program will revisit this so
        // hub packages declare their exports explicitly.
        if let Ok(file) = parse(&src) {
            if file.recipe_header().is_none() {
                cat.merge_all(&file);
            }
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
    fn catalog_merges_shared_decls_with_recipe_local() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(&root.join(MANIFEST_NAME), STARTER_MANIFEST);
        write(
            &root.join("cannabis.forage"),
            "share type Dispensary { id: String, name: String }\n\
             share type Product { id: String }\n",
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

    /// A bare (non-`share`d) type declared in a sibling file is private
    /// to that file. The focal recipe must not see it.
    #[test]
    fn bare_type_in_sibling_stays_file_scoped() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(&root.join(MANIFEST_NAME), STARTER_MANIFEST);
        write(
            &root.join("cannabis.forage"),
            "type LocalThing { id: String }\n\
             share type Dispensary { id: String }\n",
        );
        let recipe_path = root.join("rec").join("recipe.forage");
        write(&recipe_path, "recipe \"rec\"\nengine http\n");
        let ws = load(root).unwrap();
        let cat = ws.catalog_from_disk(&recipe_path).unwrap();
        assert!(
            cat.ty("Dispensary").is_some(),
            "share type must reach the focal catalog",
        );
        assert!(
            cat.ty("LocalThing").is_none(),
            "bare type must stay file-local",
        );
    }

    /// Two files each declaring `share type Product` does not error at
    /// the catalog level — the cross-file
    /// `DuplicateSharedDeclaration` validator rule owns that diagnostic
    /// now. The catalog merges last-writer-wins so the focal file sees
    /// *some* `Product`.
    #[test]
    fn duplicate_share_types_across_files_merge_without_error() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(&root.join(MANIFEST_NAME), STARTER_MANIFEST);
        write(&root.join("a.forage"), "share type Product { id: String }\n");
        write(
            &root.join("b.forage"),
            "share type Product { id: String, name: String }\n",
        );
        let recipe_path = root.join("r").join("recipe.forage");
        write(&recipe_path, "recipe \"r\"\nengine http\n");
        let ws = load(root).unwrap();
        let cat = ws.catalog_from_disk(&recipe_path).expect("catalog builds");
        assert!(
            cat.ty("Product").is_some(),
            "one of the share types should still reach the recipe; collision is the validator's concern",
        );
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

    /// Two sibling files each declaring a bare `type Product` is no
    /// longer an error at the catalog level — each is private to its
    /// file. The focal recipe sees no `Product` from either.
    #[test]
    fn duplicate_bare_types_across_files_do_not_collide() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(&root.join(MANIFEST_NAME), STARTER_MANIFEST);
        write(&root.join("a.forage"), "type Product { id: String }\n");
        write(&root.join("b.forage"), "type Product { id: String }\n");
        let recipe_path = root.join("r").join("recipe.forage");
        write(&recipe_path, "recipe \"r\"\nengine http\n");
        let ws = load(root).unwrap();
        let cat = ws
            .catalog_from_disk(&recipe_path)
            .expect("bare-type duplicates across files don't error");
        assert!(
            cat.ty("Product").is_none(),
            "neither file's bare Product should leak into the recipe",
        );
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
            "share type Dispensary { id: String, name: String? }\n\
             share type Product { id: String, terpenes: String? }\n\
             share enum MenuType { RECREATIONAL, MEDICAL }\n",
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
