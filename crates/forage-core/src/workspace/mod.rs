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
//!
//! Source vs data: the source scan picks up every `.forage` file under
//! the root at any depth, skipping hidden dirs (`.forage/`, `.git/`)
//! and the reserved data dirs `_fixtures/` and `_snapshots/`. A file's
//! role is read off its content (`recipe_header().is_some()`), not its
//! location — `Workspace::recipes()` returns the recipe-bearing files;
//! everything else is a declarations-only contributor.

pub mod fixtures;
pub mod manifest;

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::ast::{AlignmentUri, ForageFile, InputDecl, RecipeEnum, RecipeField, RecipeType};
use crate::parse::{ParseError, parse};

pub use fixtures::{FIXTURES_DIR, SNAPSHOTS_DIR, fixtures_path, snapshot_path};
pub use manifest::{
    LockedDep, Lockfile, Manifest, ManifestError, parse_lockfile, parse_manifest,
    serialize_lockfile, serialize_manifest,
};

/// The well-known manifest filename. Discovery walks ancestors looking
/// for one of these.
pub const MANIFEST_NAME: &str = "forage.toml";

/// The well-known lockfile filename written by `forage update`.
pub const LOCKFILE_NAME: &str = "forage.lock";

/// Reserved data-dir names skipped during source scanning. `_fixtures/`
/// and `_snapshots/` host workspace data keyed by recipe header name;
/// they may contain `.forage` text accidentally (a runtime dump, a
/// scenario YAML named with the wrong extension) and must not feed the
/// source catalog. `.forage/` is hidden anyway via the dotfile filter.
const DATA_DIRS: &[&str] = &[FIXTURES_DIR, SNAPSHOTS_DIR];

/// A discovered workspace: root path, parsed manifest, parsed
/// `forage.lock` (when present), and the list of `.forage` files
/// inside the tree.
///
/// The lockfile is loaded as a sibling of the manifest. A missing
/// lockfile parses to `Lockfile::default()` so workspaces that don't
/// declare hub dependencies open without ceremony; an unparseable
/// lockfile is a structured error.
#[derive(Debug, Clone)]
pub struct Workspace {
    pub root: PathBuf,
    pub manifest: Manifest,
    pub lockfile: Lockfile,
    pub files: Vec<WorkspaceFileEntry>,
}

/// One `.forage` source file inside the workspace. The parsed AST is
/// cached at load time so `recipes()` / `recipe_by_name()` can hand out
/// references without re-reading disk; a syntactically broken file is
/// retained with the parse error so the daemon's status UI can flag
/// it.
#[derive(Debug, Clone)]
pub struct WorkspaceFileEntry {
    /// Absolute path to the file.
    pub path: PathBuf,
    /// Parsed AST when the file parsed clean, or the parse error message
    /// otherwise. The error is held as `String` (not `ParseError`) so
    /// `WorkspaceFileEntry` stays `Clone`-able and cheap to ship through
    /// the daemon's status pipeline.
    pub parsed: Result<ForageFile, String>,
}

impl WorkspaceFileEntry {
    /// Recipe header name if this file declares one and parsed cleanly.
    /// `None` for header-less files and for files that failed to parse.
    pub fn recipe_name(&self) -> Option<&str> {
        self.parsed.as_ref().ok().and_then(|f| f.recipe_name())
    }
}

/// Typed view of a recipe-bearing entry. Constructed by
/// `Workspace::recipes()` / `recipe_by_name()`; `file.recipe_header` is
/// guaranteed `Some` by the constructor so the helper accessors don't
/// need to fall back.
#[derive(Debug, Clone, Copy)]
pub struct RecipeRef<'a> {
    pub path: &'a Path,
    pub file: &'a ForageFile,
}

impl<'a> RecipeRef<'a> {
    /// Recipe header name. The future hub / daemon / CLI key.
    pub fn name(&self) -> &'a str {
        self.file
            .recipe_name()
            .expect("RecipeRef constructed only for files with a recipe header")
    }
}

/// Typed view of a `.forage` file that failed to parse. Used by the
/// daemon's status surface to flag broken files in the editor without
/// aborting workspace load.
#[derive(Debug, Clone, Copy)]
pub struct BrokenFile<'a> {
    pub path: &'a Path,
    pub error: &'a str,
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

/// One recipe's typed signature — its declared inputs and outputs,
/// plus its body so the validator can chase composition references
/// transitively. Composition validation walks the signatures of the
/// recipes the stages reference to type-check each `|` boundary and
/// to detect cycles; the engine / daemon read them to bind upstream
/// records into the downstream input slot.
#[derive(Debug, Clone, PartialEq)]
pub struct RecipeSignature {
    pub inputs: Vec<InputDecl>,
    pub body: crate::ast::RecipeBody,
    /// Resolved set of types this recipe emits. When `emits` is
    /// declared on the source, that's the canonical set; otherwise
    /// it's the set inferred from the body's `emit X { … }` statements
    /// and the browser config's captures. Empty for composition bodies
    /// and header-less files. Pre-computed at signature construction
    /// so composition's stage-resolution doesn't have to re-walk the
    /// body on every check. The validator gets span info for `emits`
    /// diagnostics from the original `ForageFile` directly.
    pub output_types: std::collections::BTreeSet<String>,
}

/// Recipe-name → signature lookup. The validator's pipe-stage check
/// consults this; the engine's composition runner uses it to align
/// upstream records with the downstream input slot.
///
/// `RecipeSignatures::default()` is the lonely-file mode — no other
/// recipes are visible, so composition fails fast with
/// `UnknownComposeStage`. Workspace and daemon callers populate the
/// map from their respective recipe catalogs.
#[derive(Debug, Clone, Default)]
pub struct RecipeSignatures {
    by_name: HashMap<String, RecipeSignature>,
}

impl RecipeSignatures {
    pub fn insert(&mut self, name: String, sig: RecipeSignature) {
        self.by_name.insert(name, sig);
    }

    pub fn get(&self, name: &str) -> Option<&RecipeSignature> {
        self.by_name.get(name)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &RecipeSignature)> {
        self.by_name.iter()
    }

    /// Resolve a recipe's output type set, chasing composition chains
    /// when the recipe itself doesn't declare `emits` and has a
    /// composition body.
    ///
    /// For most recipes this is just `sig.output_types` — declared
    /// `emits` if present, otherwise inferred from the body's emit
    /// statements (precomputed at construction). The chain walk only
    /// kicks in when the recipe is a composition without an `emits`
    /// clause: we follow `compose A | B | C` to its terminal stage,
    /// look that stage up here, and recurse — handling composed
    /// compositions as well.
    ///
    /// Returns an empty set when:
    /// - the recipe isn't in the map
    /// - the composition's terminal stage is a hub-dep reference
    ///   (`@author/name`) that isn't resolvable from this map
    /// - the chain bottoms out at a stage that's not in the map
    ///
    /// The validator's `EmptyComposeStage` rule catches the second and
    /// third cases during validation; this method tolerates them so
    /// callers reading mid-edit / not-yet-validated workspaces don't
    /// crash.
    pub fn resolve_output_types(&self, recipe_name: &str) -> std::collections::BTreeSet<String> {
        self.resolve_output_types_visited(recipe_name, &mut Vec::new())
    }

    fn resolve_output_types_visited(
        &self,
        recipe_name: &str,
        visited: &mut Vec<String>,
    ) -> std::collections::BTreeSet<String> {
        // Defense against cycles. The validator's ComposeCycle rule is
        // the real enforcement; this avoids infinite recursion against
        // a not-yet-validated workspace.
        if visited.iter().any(|n| n == recipe_name) {
            return std::collections::BTreeSet::new();
        }
        let Some(sig) = self.by_name.get(recipe_name) else {
            return std::collections::BTreeSet::new();
        };
        if !sig.output_types.is_empty() {
            return sig.output_types.clone();
        }
        // The recipe's own projection is empty. Two cases reach this:
        // scraping/empty bodies that emit nothing (return the empty
        // set unchanged) and composition bodies without a declared
        // `emits` (chase the chain's terminal stage).
        let crate::ast::RecipeBody::Composition(comp) = &sig.body else {
            return std::collections::BTreeSet::new();
        };
        let Some(final_stage) = comp.stages.last() else {
            return std::collections::BTreeSet::new();
        };
        if final_stage.author.is_some() {
            // Hub-dep stages aren't resolved through this map; the
            // validator rejects them with `HubDepStageUnsupported`.
            return std::collections::BTreeSet::new();
        }
        visited.push(recipe_name.to_string());
        let resolved = self.resolve_output_types_visited(&final_stage.name, visited);
        visited.pop();
        resolved
    }
}

impl RecipeSignature {
    /// Project one `ForageFile` into a signature record.
    pub fn from_file(file: &ForageFile) -> Self {
        Self {
            inputs: file.inputs.clone(),
            body: file.body.clone(),
            output_types: file.resolved_output_types(),
        }
    }
}

impl TypeCatalog {
    /// Raw declaration as it was written. Use this when you need the
    /// surface shape — the validator's extension-chain walker reads
    /// `extends` off the raw declaration; the LSP rename-symbol pass
    /// keys on the raw field list. For "what does this type look like
    /// to an `emit` site / a snapshot consumer / a JSON-LD writer," use
    /// `lookup` instead — that walks the extension chain and folds in
    /// parent fields + alignments per the propagation policy.
    pub fn ty(&self, name: &str) -> Option<&RecipeType> {
        self.types.get(name)
    }
    pub fn recipe_enum(&self, name: &str) -> Option<&RecipeEnum> {
        self.enums.get(name)
    }

    /// Effective type after the `extends` chain is resolved. Returns
    /// the child's own fields and type-level alignments fused with
    /// every ancestor's:
    ///
    /// - **Fields:** parent's fields prepended (in parent's declaration
    ///   order), then any child fields the parent didn't already
    ///   declare. A field redeclared by the child overrides the
    ///   parent's entry (the child's per-field alignment wins; a child
    ///   that redeclares without an `aligns` clause drops the
    ///   parent's, per the propagation policy).
    /// - **Type-level alignments:** parent's first, then child's
    ///   added ones. Duplicates (same ontology+term across the chain)
    ///   collapse to one entry so JSON-LD output and discover-by-
    ///   alignment indexing don't double-count.
    /// - **`extends`:** stripped on the returned type — the chain is
    ///   already flattened.
    /// - **`shared`, `name`, `span`:** taken from the child.
    ///
    /// Cycles are short-circuited: if the chain revisits a name
    /// already in the path, the walk stops and the partial fold is
    /// returned. Validator's `CircularExtension` rule surfaces the
    /// cycle through the diagnostic channel; the lookup itself
    /// degrades gracefully so downstream consumers don't panic.
    ///
    /// Returns `None` only when `name` is not in the catalog at all.
    /// A reachable parent with a missing further-up parent yields the
    /// fold up to where the chain breaks — the validator's
    /// `UnknownExtendedType` rule names the missing link.
    pub fn lookup(&self, name: &str) -> Option<RecipeType> {
        let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();
        self.resolve(name, &mut visited)
    }

    fn resolve(
        &self,
        name: &str,
        visited: &mut std::collections::HashSet<String>,
    ) -> Option<RecipeType> {
        let raw = self.types.get(name)?;
        if !visited.insert(name.to_string()) {
            // Cycle — return the raw shape so downstream code can
            // operate on something concrete. The validator already
            // owns the diagnostic.
            let mut concrete = raw.clone();
            concrete.extends = None;
            return Some(concrete);
        }
        let Some(ext) = &raw.extends else {
            let mut out = raw.clone();
            out.extends = None;
            return Some(out);
        };
        let Some(parent) = self.resolve(&ext.name, visited) else {
            // Parent missing from catalog — surface the child as-is
            // (with `extends` stripped) so emit / field checks against
            // it operate on something concrete. The validator's
            // `UnknownExtendedType` rule names the missing link
            // separately so the author lands on it.
            let mut concrete = raw.clone();
            concrete.extends = None;
            return Some(concrete);
        };

        // Fields: parent's first (in parent's order), then child's own
        // entries. A child redeclaration replaces the parent's slot in
        // place so the parent's field-order intuition for adapter
        // recipes carries over.
        let mut fields: Vec<RecipeField> = parent.fields.clone();
        for child_field in &raw.fields {
            if let Some(slot) = fields.iter_mut().find(|f| f.name == child_field.name) {
                *slot = child_field.clone();
            } else {
                fields.push(child_field.clone());
            }
        }

        // Alignments: parent's first, child's added ones afterwards.
        // Duplicates are squashed on the (ontology, term) key so a
        // child that redeclares the parent's `aligns
        // schema.org/Product` doesn't double-up in JSON-LD output.
        let mut alignments: Vec<AlignmentUri> = parent.alignments.clone();
        for a in &raw.alignments {
            if !alignments
                .iter()
                .any(|existing| existing.ontology == a.ontology && existing.term == a.term)
            {
                alignments.push(a.clone());
            }
        }

        Some(RecipeType {
            name: raw.name.clone(),
            fields,
            shared: raw.shared,
            alignments,
            extends: None,
            span: raw.span.clone(),
        })
    }

    /// Raw (un-extended) types in name-sorted order. Use this when the
    /// declaration shape matters — workspace tools that compare
    /// `share`d sources, the LSP's symbol pass — not for runtime
    /// consumers that need the effective shape after extension. For
    /// the snapshot's `record_types`, use `types_sorted_effective`.
    pub fn types_sorted(&self) -> Vec<&RecipeType> {
        let mut out: Vec<&RecipeType> = self.types.values().collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    /// Effective types in name-sorted order, each with its `extends`
    /// chain flattened. Engines stamp `record_types` from this so a
    /// child type's snapshot entry carries every inherited field +
    /// alignment from its parent — JSON-LD output and snapshot
    /// consumers see the resolved shape, not the raw declaration.
    pub fn types_sorted_effective(&self) -> Vec<RecipeType> {
        let mut names: Vec<&String> = self.types.keys().collect();
        names.sort();
        names
            .into_iter()
            .map(|n| {
                self.lookup(n)
                    .expect("name comes from the catalog's own keys")
            })
            .collect()
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
    #[error("malformed lockfile at {path}: {source}")]
    Lockfile {
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
/// a `forage.toml`; the manifest is parsed, the optional `forage.lock`
/// is loaded if present, and the directory tree is scanned for
/// `.forage` files. The root is canonicalized so callers can compare
/// roots by equality regardless of how the path was passed in
/// (relative, symlink, trailing slash, ...).
pub fn load(root: &Path) -> Result<Workspace, WorkspaceError> {
    let root = root.canonicalize()?;
    let manifest_path = root.join(MANIFEST_NAME);
    let manifest_src = fs::read_to_string(&manifest_path)?;
    let manifest = parse_manifest(&manifest_src).map_err(|source| WorkspaceError::Manifest {
        path: manifest_path.clone(),
        source,
    })?;
    let lockfile = load_lockfile(&root)?;
    let mut files = Vec::new();
    scan_dir(&root, &mut files)?;
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(Workspace {
        root,
        manifest,
        lockfile,
        files,
    })
}

/// Read `forage.lock` if it exists, otherwise return the default
/// (empty) shape. The lockfile is optional: workspaces that don't
/// depend on hub-published artifacts open fine without one.
fn load_lockfile(root: &Path) -> Result<Lockfile, WorkspaceError> {
    let path = root.join(LOCKFILE_NAME);
    let raw = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Lockfile::default()),
        Err(e) => return Err(WorkspaceError::Io(e)),
    };
    parse_lockfile(&raw).map_err(|source| WorkspaceError::Lockfile { path, source })
}

/// Re-scan the workspace tree on disk and refresh `files`. Manifest is
/// re-read too so toggling `[deps]` outside Studio is picked up.
pub fn refresh(ws: &mut Workspace) -> Result<(), WorkspaceError> {
    let fresh = load(&ws.root)?;
    *ws = fresh;
    Ok(())
}

fn scan_dir(dir: &Path, out: &mut Vec<WorkspaceFileEntry>) -> Result<(), WorkspaceError> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        // Skip hidden directories (`.forage/`, `.git/`, etc.) and the
        // reserved data dirs `_fixtures/` / `_snapshots/`. Source files
        // live anywhere else in the tree.
        if name_str.starts_with('.') {
            continue;
        }
        let ft = entry.file_type()?;
        if ft.is_dir() {
            if DATA_DIRS.contains(&name_str.as_ref()) {
                continue;
            }
            scan_dir(&path, out)?;
            continue;
        }
        if ft.is_file() && path.extension().is_some_and(|e| e == "forage") {
            let source = fs::read_to_string(&path)?;
            // A single broken file used to abort the entire workspace
            // load, which cascaded into "Studio won't even start" when
            // the daemon library held an unparseable file. Capture the
            // failure as a parse-error entry instead — the engine won't
            // try to run it, but the daemon surfaces it through the
            // recipe-status API so the user can find and fix it.
            let parsed = parse(&source).map_err(|e| e.to_string());
            out.push(WorkspaceFileEntry { path, parsed });
        }
    }
    Ok(())
}

impl Workspace {
    /// All recipe-bearing files in the workspace, in path order. A
    /// recipe is any `.forage` file that parsed clean *and* declares a
    /// `recipe "<name>" engine <kind>` header. Header-less files
    /// contribute shared declarations to the workspace catalog but
    /// aren't surfaced here.
    pub fn recipes(&self) -> impl Iterator<Item = RecipeRef<'_>> {
        self.files.iter().filter_map(|entry| {
            let file = entry.parsed.as_ref().ok()?;
            file.recipe_header()?;
            Some(RecipeRef {
                path: &entry.path,
                file,
            })
        })
    }

    /// First recipe in path order whose header name equals `name`. The
    /// recipe namespace is flat across the workspace; duplicates are a
    /// validator concern (cross-file `DuplicateRecipeName`), not a
    /// discovery failure — the workspace still loads so the user can
    /// see both files and resolve the collision.
    pub fn recipe_by_name(&self, name: &str) -> Option<RecipeRef<'_>> {
        self.recipes().find(|r| r.name() == name)
    }

    /// Every `.forage` file in the workspace whose contents failed to
    /// parse, in path order. The daemon's recipe-status surface joins
    /// this with its own deployment view so a syntactically broken file
    /// stays visible (rather than silently dropping out of the list).
    pub fn broken(&self) -> impl Iterator<Item = BrokenFile<'_>> {
        self.files.iter().filter_map(|entry| match &entry.parsed {
            Err(error) => Some(BrokenFile {
                path: &entry.path,
                error,
            }),
            Ok(_) => None,
        })
    }

    /// Build a merged `TypeCatalog` for validating `file` in this
    /// workspace.
    ///
    /// Merge order:
    /// 1. `share`d types/enums from every other workspace file. Files
    ///    contribute only what they `share`; bare `type Foo { … }` stays
    ///    private to its declaring file.
    /// 2. Hub-cached types from the lockfile's `[types]` pins. Each
    ///    entry resolves to a `.forage` source body in the type cache
    ///    (`<cache>/types/<author>/<Name>/<version>.forage`); reading
    ///    the body and merging its `share` types into the catalog.
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
            if entry.parsed.is_err() {
                continue;
            }
            let parsed = read_workspace_file(&entry.path, &read)?;
            cat.merge_shared(&parsed);
        }

        // 2. Hub-cached types from the lockfile's `[types]` pins. The
        //    publish/sync flow writes one `.forage` per
        //    `(author, name, version)` under
        //    `<cache>/types/<author>/<Name>/<v>.forage`. Pre-1.0 volume
        //    is small enough that we re-read + re-parse on every catalog
        //    build; future passes can memoize.
        let cache = hub_cache_root();
        for (slug, locked) in &self.lockfile.types {
            let Some((author, name)) = slug.split_once('/') else {
                continue;
            };
            let path = type_cache_file(&cache, author, name, locked.version);
            let Ok(src) = fs::read_to_string(&path) else {
                // A missing cache file means the user hasn't run
                // `forage update` since the lockfile was written. The
                // workspace still loads; the catalog just won't have
                // the missing type. The validator's `UnknownType`
                // rule surfaces this as a recipe-level diagnostic.
                tracing::debug!(
                    type_slug = %slug,
                    version = locked.version,
                    cache_path = %path.display(),
                    "lockfile type pin not in cache",
                );
                continue;
            };
            let Ok(parsed) = parse(&src) else {
                tracing::warn!(
                    type_slug = %slug,
                    version = locked.version,
                    "cached type source failed to parse",
                );
                continue;
            };
            cat.merge_shared(&parsed);
        }

        // 3. Focal file: every type/enum, file-local plus `share`d.
        //    Overwrites any same-name `share`d entry from step 1, which
        //    is what gives the focal file file-local precedence.
        cat.merge_all(file);
        Ok(cat)
    }

    /// Build a name → signature lookup for every recipe in the
    /// workspace. Composition validation walks this to type-check
    /// stage boundaries; the engine reads it to bind upstream records
    /// into the downstream stage's input slot.
    ///
    /// Broken (unparseable) files are skipped — they're surfaced
    /// separately via `Workspace::broken()`.
    pub fn recipe_signatures(&self) -> RecipeSignatures {
        let mut out = RecipeSignatures::default();
        for entry in &self.files {
            let Ok(file) = &entry.parsed else { continue };
            let Some(header) = file.recipe_header() else {
                continue;
            };
            out.insert(header.name.clone(), RecipeSignature::from_file(file));
        }
        out
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

// --- Hub cache resolution -------------------------------------------------

/// Where hub-published packages are cached on disk. `~/Library/Forage/
/// Cache/hub/` on macOS; the platform data dir's `Forage/Cache/hub/`
/// elsewhere. Override with `FORAGE_HUB_CACHE` (tests, alternative
/// installs).
pub fn hub_cache_root() -> PathBuf {
    if let Ok(p) = std::env::var("FORAGE_HUB_CACHE")
        && !p.is_empty()
    {
        return PathBuf::from(p);
    }
    if cfg!(target_os = "macos")
        && let Some(home) = dirs::home_dir()
    {
        return home.join("Library").join("Forage").join("Cache").join("hub");
    }
    if let Some(data) = dirs::data_dir() {
        return data.join("Forage").join("Cache").join("hub");
    }
    PathBuf::from(".forage-cache").join("hub")
}

/// On-disk path of a single cached type version. Mirrors the layout
/// the publish/sync flow writes to:
/// `<cache>/types/<author>/<Name>/<version>.forage`.
pub fn type_cache_file(cache_root: &Path, author: &str, name: &str, version: u32) -> PathBuf {
    cache_root
        .join("types")
        .join(author)
        .join(name)
        .join(format!("{version}.forage"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
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

    /// `recipes()` reports every file that parsed cleanly *and* declares
    /// a header; header-less files are visible via `files` but not
    /// `recipes()`. Files live at any depth in the tree.
    #[test]
    fn recipes_discovers_files_at_any_depth() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(&root.join(MANIFEST_NAME), STARTER_MANIFEST);
        // Header-less file at the root.
        write(
            &root.join("cannabis.forage"),
            "type Dispensary { id: String }\n",
        );
        // Recipe at the root.
        write(
            &root.join("remedy.forage"),
            "recipe \"remedy\"\nengine http\n",
        );
        // Recipe nested in a subdirectory.
        write(
            &root.join("subdir").join("nested.forage"),
            "recipe \"nested\"\nengine http\n",
        );
        // Recipe with a non-matching file stem in another subdirectory.
        write(
            &root.join("other").join("inner.forage"),
            "recipe \"inner\"\nengine http\n",
        );

        let ws = load(root).unwrap();
        let names: Vec<&str> = ws.recipes().map(|r| r.name()).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, vec!["inner", "nested", "remedy"]);
        assert_eq!(ws.files.len(), 4, "all four files in `files`");
    }

    /// Files under `_fixtures/`, `_snapshots/`, and `.forage/` are
    /// workspace data, not source. The source scan must skip them at
    /// any depth.
    #[test]
    fn data_dirs_excluded_from_source_scan() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(&root.join(MANIFEST_NAME), STARTER_MANIFEST);
        write(
            &root.join("real.forage"),
            "recipe \"real\"\nengine http\n",
        );
        // Files inside data dirs at the root: must be skipped.
        write(
            &root.join("_fixtures").join("real.forage"),
            "recipe \"shadow\"\nengine http\n",
        );
        write(
            &root.join("_snapshots").join("snap.forage"),
            "type Bogus { id: String }\n",
        );
        write(
            &root.join(".forage").join("hidden.forage"),
            "type Hidden { id: String }\n",
        );
        // Files inside data dirs nested inside a normal folder: also
        // must be skipped (the filter must trigger at every depth).
        write(
            &root.join("nested").join("_fixtures").join("inner.forage"),
            "recipe \"inner-shadow\"\nengine http\n",
        );

        let ws = load(root).unwrap();
        let paths: Vec<&Path> = ws.files.iter().map(|f| f.path.as_path()).collect();
        assert_eq!(paths.len(), 1, "only the source file is scanned: {paths:?}");
        assert!(paths[0].ends_with("real.forage"));
        let recipes: Vec<&str> = ws.recipes().map(|r| r.name()).collect();
        assert_eq!(recipes, vec!["real"]);
    }

    /// `recipe_by_name` resolves a recipe regardless of which file holds
    /// it. Path layout is incidental.
    #[test]
    fn recipe_by_name_resolves_across_layouts() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(&root.join(MANIFEST_NAME), STARTER_MANIFEST);
        write(
            &root.join("flat-one.forage"),
            "recipe \"flat-one\"\nengine http\n",
        );
        write(
            &root.join("dir").join("recipe.forage"),
            "recipe \"in-dir\"\nengine http\n",
        );
        write(
            &root.join("deep").join("more").join("file.forage"),
            "recipe \"deep-one\"\nengine http\n",
        );

        let ws = load(root).unwrap();
        let flat = ws.recipe_by_name("flat-one").expect("flat-one");
        assert!(flat.path.ends_with("flat-one.forage"));
        let nested = ws.recipe_by_name("in-dir").expect("in-dir");
        assert!(nested.path.ends_with("dir/recipe.forage"));
        let deep = ws.recipe_by_name("deep-one").expect("deep-one");
        assert!(deep.path.ends_with("deep/more/file.forage"));
        assert!(ws.recipe_by_name("missing").is_none());
    }

    /// Two recipes declaring the same header name don't break workspace
    /// load. The validator surfaces the collision via
    /// `DuplicateRecipeName`; the discovery API returns the first match
    /// in path order so callers can still resolve *some* file. The
    /// recipe-name namespace stays flat across the workspace.
    #[test]
    fn duplicate_recipe_names_resolve_to_first_in_path_order() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(&root.join(MANIFEST_NAME), STARTER_MANIFEST);
        write(
            &root.join("a.forage"),
            "recipe \"dup\"\nengine http\n",
        );
        write(
            &root.join("z.forage"),
            "recipe \"dup\"\nengine http\n",
        );
        let ws = load(root).unwrap();
        let dup_count = ws.recipes().filter(|r| r.name() == "dup").count();
        assert_eq!(dup_count, 2, "both duplicates surface in recipes()");
        let pick = ws.recipe_by_name("dup").expect("dup resolves");
        assert!(
            pick.path.ends_with("a.forage"),
            "path-order tiebreak picks a.forage; got {:?}",
            pick.path,
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
        // parse-error entry so the daemon can surface it.
        write(
            &root.join("good.forage"),
            "recipe \"good\"\nengine http\n",
        );
        write(
            &root.join("bad.forage"),
            // Missing `engine` line + dangling `for` makes the parser
            // bail out. Exact error text is the parser's concern; we
            // just need *some* parse failure.
            "recipe \"bad\"\nfor in {{ }}\n",
        );

        let ws = load(root).expect("load must succeed despite broken file");
        let recipe_names: Vec<&str> = ws.recipes().map(|r| r.name()).collect();
        assert_eq!(recipe_names, vec!["good"]);

        let broken: Vec<BrokenFile<'_>> = ws.broken().collect();
        assert_eq!(broken.len(), 1);
        assert!(broken[0].path.ends_with("bad.forage"));
        assert!(!broken[0].error.is_empty());
    }

    #[test]
    fn declarations_parse_failure_is_captured_too() {
        // A header-less `.forage` file with a syntax error also lands
        // in the broken bucket.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(&root.join(MANIFEST_NAME), STARTER_MANIFEST);
        write(
            &root.join("shared.forage"),
            // `type` without a name is a parse error.
            "type { id: String }\n",
        );
        let ws = load(root).unwrap();
        let broken: Vec<BrokenFile<'_>> = ws.broken().collect();
        assert_eq!(broken.len(), 1);
        assert!(broken[0].path.ends_with("shared.forage"));
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

    /// The catalog pulls hub-cached types in through the lockfile's
    /// `[types]` pins. Each pin resolves to
    /// `<cache>/types/<author>/<Name>/<v>.forage`; the workspace loader
    /// reads + parses the file and folds its `share` types into the
    /// catalog. Tests serialize against the process-global
    /// `FORAGE_HUB_CACHE` env var.
    #[test]
    fn catalog_folds_hub_cached_type_pins() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = tmp.path().join("hub-cache");
        let cache_type = type_cache_file(&cache, "alice", "Product", 4);
        std::fs::create_dir_all(cache_type.parent().unwrap()).unwrap();
        std::fs::write(
            &cache_type,
            "share type Product {\n    id: String\n    name: String\n}\n",
        )
        .unwrap();

        let ws_root = tmp.path().join("ws");
        std::fs::create_dir_all(&ws_root).unwrap();
        std::fs::write(
            ws_root.join(MANIFEST_NAME),
            "name = \"bob/uses-product\"\ndescription = \"\"\ncategory = \"x\"\ntags = []\n",
        )
        .unwrap();
        // Lockfile pins @alice/Product@4 in the `[types]` table.
        std::fs::write(
            ws_root.join(LOCKFILE_NAME),
            "[types.\"alice/Product\"]\nversion = 4\nhash = \"\"\n",
        )
        .unwrap();
        let recipe_path = ws_root.join("uses-product.forage");
        std::fs::write(
            &recipe_path,
            "recipe \"uses-product\"\nengine http\nstep s { method \"GET\" url \"x\" }\n",
        )
        .unwrap();

        let prev = std::env::var("FORAGE_HUB_CACHE").ok();
        // SAFETY: env mutation is unsafe in Rust 2024; the test runs
        // serially against the global env. The test restores the prior
        // value before returning.
        unsafe { std::env::set_var("FORAGE_HUB_CACHE", &cache); }

        let ws = load(&ws_root).unwrap();
        let cat = ws.catalog_from_disk(&recipe_path).unwrap();

        // SAFETY: see above.
        match prev {
            Some(v) => unsafe { std::env::set_var("FORAGE_HUB_CACHE", v) },
            None => unsafe { std::env::remove_var("FORAGE_HUB_CACHE") },
        }

        let product = cat.ty("Product").expect("hub-cached Product should be in catalog");
        assert_eq!(product.fields.len(), 2);
        assert!(product.fields.iter().any(|f| f.name == "name"));
    }

    /// A lockfile pin pointing at a missing cache file degrades
    /// gracefully: the workspace still loads, the catalog skips the
    /// type, and the validator surfaces the missing type at recipe
    /// validation time via its `UnknownType` rule.
    #[test]
    fn missing_cached_type_does_not_break_load() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = tmp.path().join("hub-cache");
        // Intentionally do NOT write the cache file.
        std::fs::create_dir_all(&cache).unwrap();

        let ws_root = tmp.path().join("ws");
        std::fs::create_dir_all(&ws_root).unwrap();
        std::fs::write(
            ws_root.join(MANIFEST_NAME),
            "name = \"bob/uses-product\"\ndescription = \"\"\ncategory = \"x\"\ntags = []\n",
        )
        .unwrap();
        std::fs::write(
            ws_root.join(LOCKFILE_NAME),
            "[types.\"alice/Missing\"]\nversion = 1\nhash = \"\"\n",
        )
        .unwrap();
        let recipe_path = ws_root.join("uses-missing.forage");
        std::fs::write(
            &recipe_path,
            "recipe \"uses-missing\"\nengine http\nstep s { method \"GET\" url \"x\" }\n",
        )
        .unwrap();

        let prev = std::env::var("FORAGE_HUB_CACHE").ok();
        // SAFETY: env mutation is unsafe in Rust 2024.
        unsafe { std::env::set_var("FORAGE_HUB_CACHE", &cache); }

        let ws = load(&ws_root).unwrap();
        let cat = ws.catalog_from_disk(&recipe_path).unwrap();

        // SAFETY: see above.
        match prev {
            Some(v) => unsafe { std::env::set_var("FORAGE_HUB_CACHE", v) },
            None => unsafe { std::env::remove_var("FORAGE_HUB_CACHE") },
        }

        assert!(cat.ty("Missing").is_none());
    }

    #[test]
    fn lookup_returns_raw_type_when_no_extends_chain() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(&root.join(MANIFEST_NAME), STARTER_MANIFEST);
        write(
            &root.join("decls.forage"),
            "share type Plain { id: String, name: String }\n",
        );
        let recipe_path = root.join("r.forage");
        write(&recipe_path, "recipe \"r\"\nengine http\n");
        let ws = load(root).unwrap();
        let cat = ws.catalog_from_disk(&recipe_path).unwrap();
        let plain = cat.lookup("Plain").expect("Plain in catalog");
        assert_eq!(plain.fields.len(), 2);
        assert!(plain.extends.is_none(), "lookup strips extends");
    }

    #[test]
    fn lookup_folds_parent_fields_and_alignments_into_child() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(&root.join(MANIFEST_NAME), STARTER_MANIFEST);
        write(
            &root.join("decls.forage"),
            "share type Parent\n\
                 aligns schema.org/Product\n\
             {\n\
                 id:   String aligns schema.org/identifier\n\
                 name: String aligns schema.org/name\n\
             }\n\
             share type Child extends Parent@v1\n\
                 aligns wikidata/Q2424752\n\
             {\n\
                 extra: String aligns schema.org/sku\n\
             }\n",
        );
        let recipe_path = root.join("r.forage");
        write(&recipe_path, "recipe \"r\"\nengine http\n");
        let ws = load(root).unwrap();
        let cat = ws.catalog_from_disk(&recipe_path).unwrap();

        let child = cat.lookup("Child").expect("Child in catalog");
        let names: Vec<&str> = child.fields.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, vec!["id", "name", "extra"], "parent fields first, child appended");
        // Parent alignment carried.
        assert!(child
            .alignments
            .iter()
            .any(|a| a.ontology == "schema.org" && a.term == "Product"));
        // Child alignment added.
        assert!(child
            .alignments
            .iter()
            .any(|a| a.ontology == "wikidata" && a.term == "Q2424752"));
        // Inherited per-field alignment carried through unchanged.
        let id = child.fields.iter().find(|f| f.name == "id").unwrap();
        assert_eq!(id.alignment.as_ref().unwrap().term, "identifier");
    }

    #[test]
    fn lookup_field_override_drops_parent_alignment_when_child_omits() {
        // Parent declares `name: String aligns schema.org/name`. Child
        // redeclares `name: String` with no alignment. Per the
        // propagation policy the child's redeclaration wins — the
        // parent's field-level alignment is dropped on the child's
        // effective shape.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(&root.join(MANIFEST_NAME), STARTER_MANIFEST);
        write(
            &root.join("decls.forage"),
            "share type Parent {\n\
                 id:   String\n\
                 name: String aligns schema.org/name\n\
             }\n\
             share type Child extends Parent@v1 {\n\
                 name: String\n\
                 extra: String\n\
             }\n",
        );
        let recipe_path = root.join("r.forage");
        write(&recipe_path, "recipe \"r\"\nengine http\n");
        let ws = load(root).unwrap();
        let cat = ws.catalog_from_disk(&recipe_path).unwrap();
        let child = cat.lookup("Child").expect("Child in catalog");
        let name = child.fields.iter().find(|f| f.name == "name").unwrap();
        assert!(
            name.alignment.is_none(),
            "child override without alignment drops the parent's"
        );
    }

    #[test]
    fn lookup_field_override_substitutes_alignment_when_child_provides_one() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(&root.join(MANIFEST_NAME), STARTER_MANIFEST);
        write(
            &root.join("decls.forage"),
            "share type Parent {\n\
                 id:   String\n\
                 name: String aligns schema.org/name\n\
             }\n\
             share type Child extends Parent@v1 {\n\
                 name: String aligns wikidata/P2561\n\
                 extra: String\n\
             }\n",
        );
        let recipe_path = root.join("r.forage");
        write(&recipe_path, "recipe \"r\"\nengine http\n");
        let ws = load(root).unwrap();
        let cat = ws.catalog_from_disk(&recipe_path).unwrap();
        let child = cat.lookup("Child").expect("Child in catalog");
        let name = child.fields.iter().find(|f| f.name == "name").unwrap();
        let align = name.alignment.as_ref().expect("child's alignment kept");
        assert_eq!(align.ontology, "wikidata");
        assert_eq!(align.term, "P2561");
    }

    #[test]
    fn lookup_three_step_chain_folds_all_ancestors() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(&root.join(MANIFEST_NAME), STARTER_MANIFEST);
        write(
            &root.join("decls.forage"),
            "share type A { id: String }\n\
             share type B extends A@v1 { b: String }\n\
             share type C extends B@v1 { c: String }\n",
        );
        let recipe_path = root.join("r.forage");
        write(&recipe_path, "recipe \"r\"\nengine http\n");
        let ws = load(root).unwrap();
        let cat = ws.catalog_from_disk(&recipe_path).unwrap();
        let c = cat.lookup("C").expect("C in catalog");
        let names: Vec<&str> = c.fields.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, vec!["id", "b", "c"]);
    }

    #[test]
    fn lookup_cycle_degrades_gracefully() {
        // Cycle in `extends`. The validator surfaces it; the catalog
        // doesn't loop forever — it returns the partial fold so
        // downstream code that calls `lookup` against either node
        // gets something concrete to work with.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(&root.join(MANIFEST_NAME), STARTER_MANIFEST);
        write(
            &root.join("decls.forage"),
            "share type A extends B@v1 { a: String }\n\
             share type B extends A@v1 { b: String }\n",
        );
        let recipe_path = root.join("r.forage");
        write(&recipe_path, "recipe \"r\"\nengine http\n");
        let ws = load(root).unwrap();
        let cat = ws.catalog_from_disk(&recipe_path).unwrap();
        // `lookup` must complete without panic / infinite loop. The
        // exact field set on either node is undefined under a cycle
        // — we only assert termination + a non-empty result.
        let a = cat.lookup("A").expect("A in catalog");
        assert!(a.extends.is_none(), "lookup strips extends on cycle");
        let b = cat.lookup("B").expect("B in catalog");
        assert!(b.extends.is_none());
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

    /// `RecipeSignatures::resolve_output_types` reports the terminal
    /// stage's emits for a composition recipe that has no declared
    /// `emits` clause of its own. The notebook picker and any other
    /// "what does this recipe produce" consumer needs this so a
    /// `compose A | B` recipe registers as a producer of B's output
    /// type even without an `emits` declaration on the composition.
    #[test]
    fn resolve_output_types_chases_composition_chain() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(&root.join(MANIFEST_NAME), STARTER_MANIFEST);
        write(
            &root.join("Product.forage"),
            "share type Product { id: String }\n",
        );
        write(
            &root.join("scrape.forage"),
            "recipe \"scrape\"\nengine http\n\
             step list { method \"GET\" url \"https://x.test\" }\n\
             emit Product { id ← \"a\" }\n",
        );
        write(
            &root.join("enrich.forage"),
            "recipe \"enrich\"\nengine http\n\
             input prior: [Product]\n\
             step list { method \"GET\" url \"https://x.test\" }\n\
             emit Product { id ← \"b\" }\n",
        );
        // Composition recipe carries no `emits` clause; its output
        // must resolve to `enrich`'s output (the chain's terminal
        // stage) via chain resolution.
        write(
            &root.join("composed.forage"),
            "recipe \"composed\"\nengine http\n\
             compose \"scrape\" | \"enrich\"\n",
        );
        let ws = load(root).unwrap();
        let signatures = ws.recipe_signatures();

        // The composition recipe's own `output_types` field is empty
        // (no declared emits, no body emits).
        let composed_sig = signatures.get("composed").expect("composed signature");
        assert!(
            composed_sig.output_types.is_empty(),
            "composition without emits has empty local output_types: {:?}",
            composed_sig.output_types,
        );

        // Chain resolution walks to the terminal stage and reports
        // its output.
        let resolved = signatures.resolve_output_types("composed");
        let expected: BTreeSet<String> = ["Product".to_string()].into_iter().collect();
        assert_eq!(resolved, expected);
    }

    /// Multi-level composition: `outer = compose middle | tail`;
    /// `middle = compose leaf | passthrough`; the chain bottoms out
    /// at scraping recipes that emit Product. `resolve_output_types`
    /// chases through both composition levels to find Product.
    #[test]
    fn resolve_output_types_recurses_through_composed_compositions() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(&root.join(MANIFEST_NAME), STARTER_MANIFEST);
        write(
            &root.join("Product.forage"),
            "share type Product { id: String }\n",
        );
        write(
            &root.join("leaf.forage"),
            "recipe \"leaf\"\nengine http\n\
             step list { method \"GET\" url \"https://x.test\" }\n\
             emit Product { id ← \"a\" }\n",
        );
        write(
            &root.join("passthrough.forage"),
            "recipe \"passthrough\"\nengine http\n\
             input prior: [Product]\n\
             step list { method \"GET\" url \"https://x.test\" }\n\
             emit Product { id ← \"b\" }\n",
        );
        write(
            &root.join("tail.forage"),
            "recipe \"tail\"\nengine http\n\
             input prior: [Product]\n\
             step list { method \"GET\" url \"https://x.test\" }\n\
             emit Product { id ← \"c\" }\n",
        );
        write(
            &root.join("middle.forage"),
            "recipe \"middle\"\nengine http\n\
             compose \"leaf\" | \"passthrough\"\n",
        );
        write(
            &root.join("outer.forage"),
            "recipe \"outer\"\nengine http\n\
             compose \"middle\" | \"tail\"\n",
        );
        let ws = load(root).unwrap();
        let signatures = ws.recipe_signatures();
        let resolved = signatures.resolve_output_types("outer");
        let expected: BTreeSet<String> = ["Product".to_string()].into_iter().collect();
        assert_eq!(resolved, expected);
    }
}
