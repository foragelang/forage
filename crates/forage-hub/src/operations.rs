//! High-level Studio↔hub operations.
//!
//! These functions own the on-disk shape of a synced recipe. The wire
//! layer ([`crate::client`]) just speaks the REST API; everything that
//! materializes a `PackageVersion` (and the types it references) into
//! a workspace, walks an on-disk recipe back into a sequence of type
//! publishes plus a recipe publish, and tracks the source version in a
//! sidecar lives here so Studio's Tauri commands and the CLI
//! subcommands share one implementation.
//!
//! The hub-side "slug" is the recipe's header name; locally each
//! recipe is one flat file `<workspace>/<recipe>.forage`. Workspace
//! data (`_fixtures/<recipe>.jsonl`, `_snapshots/<recipe>.json`) and
//! the hub-sync sidecar (`.forage/sync/<recipe>.json`) hang off the
//! workspace root keyed on the same recipe-name string.
//!
//! Hub types are first-class citizens: the publish flow extracts each
//! `share`d type in the workspace, publishes it as its own
//! `TypeVersion`, and pins the recipe to the resolved versions via
//! `type_refs`. The sync flow materializes types into the workspace
//! alongside the recipe (one `<workspace>/<Name>.forage` per type) and
//! mirrors them into the hub type cache so the workspace loader can
//! resolve them.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use forage_core::ast::{AlignmentUri as CoreAlignmentUri, FieldType, RecipeType};
use forage_core::workspace::{Workspace, fixtures_path, snapshot_path};

use crate::client::HubClient;
use crate::error::{HubError, HubResult};
use crate::types::{
    AlignmentUri, ForkedFrom, PackageFixture, PackageMetadata, PackageSnapshot, PackageVersion,
    PublishRequest, PublishResponse, PublishTypeRequest, TypeFieldAlignment, TypeRef, TypeVersion,
    VersionSpec,
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
/// (always `<workspace>/<slug>.forage`), the version that landed, the
/// sidecar shape, and the type pins synced alongside it. Callers echo
/// "synced @author/slug@v4" and may surface the type pins.
#[derive(Debug, Clone)]
pub struct SyncOutcome {
    pub recipe_path: PathBuf,
    pub version: u32,
    pub meta: ForageMeta,
    /// Types synced into the workspace as part of this sync. One entry
    /// per `TypeRef` on the recipe; in `(author, name, version)` order
    /// from the recipe's pin list.
    pub type_pins: Vec<TypePin>,
}

/// One synced type pin. The same shape the lockfile writes under
/// `[types]`; surfaced from `sync_from_hub` so the caller can persist
/// it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypePin {
    pub author: String,
    pub name: String,
    pub version: u32,
}

/// Pull `(author, slug, version)` from the hub, materialize the recipe
/// plus every type it pins, write the sidecar, and bump the download
/// counter. The version defaults to `latest` when `version` is `None`.
///
/// The on-disk layout matches the flat workspace shape:
///
/// - `<workspace_root>/<slug>.forage` — the recipe source.
/// - `<workspace_root>/<Name>.forage` — each referenced type, one file
///   per type, named after the bare type name.
/// - `<workspace_root>/_fixtures/<slug>.jsonl` — captured replay data.
/// - `<workspace_root>/_snapshots/<slug>.json` — captured snapshot.
/// - `<workspace_root>/.forage/sync/<slug>.json` — hub-sync sidecar.
///
/// The recipe destination file must be empty (or already a hub-synced
/// copy at an older version). Refusing to overwrite avoids clobbering
/// an in-progress edit when the user `forage sync`'s a recipe whose
/// header name collides with a local file. Type files follow the same
/// rule per type-name.
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

    // Pull each referenced type into the workspace + the hub-type
    // cache. The cache copy is what the workspace loader resolves
    // against at validation time via the lockfile's `[types]` pins;
    // the workspace copy is what the user reads/edits.
    let mut type_pins = Vec::with_capacity(artifact.type_refs.len());
    let cache_root = forage_core::workspace::hub_cache_root();
    for r in &artifact.type_refs {
        let tv = client
            .get_type_version(&r.author, &r.name, VersionSpec::Numbered(r.version))
            .await?;
        write_type_to_workspace(workspace_root, &tv)?;
        write_type_to_cache(&cache_root, &tv)?;
        type_pins.push(TypePin {
            author: r.author.clone(),
            name: r.name.clone(),
            version: r.version,
        });
    }

    write_recipe(&recipe_path, &artifact)?;
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
        type_pins,
    })
}

/// Fetch a recipe version into the hub recipe cache directory
/// (`<cache>/<author>/<slug>/<version>/`) so the workspace loader can
/// resolve cross-recipe references. Types referenced by the recipe are
/// resolved into the parallel type cache via [`fetch_type_to_cache`].
/// Returns the cache directory and the SHA-256 of the raw JSON
/// artifact (used to populate `forage.lock`).
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
    fs::create_dir_all(&dir)?;
    let recipe_path = dir.join(format!("{slug}.forage"));
    write_recipe(&recipe_path, &artifact)?;
    // Pull every referenced type into the parallel type cache so the
    // workspace loader can fold them into the catalog when this recipe
    // is depended on.
    for r in &artifact.type_refs {
        let tv = client
            .get_type_version(&r.author, &r.name, VersionSpec::Numbered(r.version))
            .await?;
        write_type_to_cache(cache_root, &tv)?;
    }
    let sha = sha256_hex(&serde_json::to_string(&artifact)?);
    Ok(FetchedPackage { dir, sha256: sha })
}

/// Fetch a single type version into the hub type cache. Used both by
/// the recipe sync path (every referenced type is mirrored into the
/// cache) and directly by `forage update` when resolving lockfile pins.
pub async fn fetch_type_to_cache(
    client: &HubClient,
    cache_root: &Path,
    author: &str,
    name: &str,
    version: u32,
) -> HubResult<FetchedType> {
    let tv = client
        .get_type_version(author, name, VersionSpec::Numbered(version))
        .await?;
    let path = write_type_to_cache(cache_root, &tv)?;
    let sha = sha256_hex(&serde_json::to_string(&tv)?);
    Ok(FetchedType { path, sha256: sha })
}

/// Result of [`fetch_to_cache`]: on-disk path of the materialized
/// recipe version plus the SHA-256 of its serialized wire artifact.
#[derive(Debug, Clone)]
pub struct FetchedPackage {
    pub dir: PathBuf,
    pub sha256: String,
}

/// Result of [`fetch_type_to_cache`]: on-disk path of the cached
/// `.forage` source for the type plus the SHA-256 of its serialized
/// wire artifact.
#[derive(Debug, Clone)]
pub struct FetchedType {
    pub path: PathBuf,
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

/// One publishable type extracted from a workspace. Carries the
/// type's bare name, the standalone `share type Name { … }` source
/// fragment, and the alignments parsed off the AST.
#[derive(Debug, Clone)]
pub struct SharedTypeSource {
    pub name: String,
    pub source: String,
    pub alignments: Vec<AlignmentUri>,
    pub field_alignments: Vec<TypeFieldAlignment>,
}

/// Plan for a single recipe publish: the types to push first (each
/// resolves to a `TypeRef` on the recipe payload), followed by the
/// recipe payload itself.
///
/// Construction is offline — `assemble_publish_plan` doesn't talk to
/// the hub. The caller (the publish driver) walks `types` posting each
/// one in turn, collecting the resolved versions, then fills
/// `recipe_payload.type_refs` and posts the recipe.
#[derive(Debug, Clone)]
pub struct PublishPlan {
    pub types: Vec<SharedTypeSource>,
    /// The recipe `PublishRequest` minus the `type_refs` field. The
    /// driver fills `type_refs` in declaration-name-sorted order
    /// matching `types[..]` after each type publish resolves to a
    /// concrete `(author, name, version)`.
    pub recipe_payload: PublishRequest,
}

/// Build a publish plan for the workspace recipe `recipe_name`. The
/// plan lists every `share`d type the workspace contributes (one
/// publishable unit per declared type, regardless of which file
/// declares it) and the recipe payload that pins them. The caller
/// drives the actual publishes — see [`publish_from_workspace`] for
/// the canonical sequence.
///
/// Types are pulled from:
/// - Every workspace `.forage` file that's not the focal recipe and
///   declares at least one `share` type.
/// - The focal recipe's own `share` types (yes — a recipe file may
///   carry shareable types; they get hoisted to the hub like any other
///   shared declaration).
///
/// File-local types (`type Foo { ... }` without `share`) stay inlined
/// in the recipe source and are not publishable. `share fn` and `share
/// enum` are carried in the recipe `decls` field but the type-only
/// resource on the hub doesn't model them — they ride along in the
/// recipe source.
pub fn assemble_publish_plan(
    workspace: &Workspace,
    recipe_name: &str,
    author: &str,
    description: String,
    category: String,
    tags: Vec<String>,
) -> HubResult<PublishPlan> {
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

    let parsed_recipe = forage_core::parse(&recipe).map_err(|e| {
        HubError::Generic(format!("parse recipe {recipe_name:?} for publish: {e}"))
    })?;
    let types = collect_shared_types(workspace, &recipe)?;
    let fixtures = read_fixtures(&workspace.root, recipe_name)?;
    let snapshot = read_snapshot(&workspace.root, recipe_name)?;
    let meta = read_meta(&workspace.root, recipe_name)?;
    let base_version = meta.map(|m| m.base_version);

    // Plan recipe-side TypeRefs in the same order `types` lists them.
    // The driver overwrites this list after publishing each type with
    // the server-resolved version. We pre-populate `version: 0` so the
    // shape is right; passing a 0 to the hub would fail validation, so
    // the driver MUST overwrite before posting.
    let type_refs: Vec<TypeRef> = types
        .iter()
        .map(|t| TypeRef {
            author: author.to_string(),
            name: t.name.clone(),
            version: 0,
        })
        .collect();

    // Partition the recipe's type pins by the role the recipe gives
    // each one: input (declared via `input <name>: T?`) or output
    // (declared `emits T | U | …` when present, else inferred from the
    // body's `emit X { … }` statements). A single type can be in both
    // (enrichment recipes like `input T → emits T`). Types the recipe
    // pins but doesn't directly read or emit — `share` types shipped
    // alongside without participating in the recipe's signature —
    // appear in `type_refs` only.
    let mut input_names = std::collections::BTreeSet::new();
    for inp in &parsed_recipe.inputs {
        collect_referenced_type_names(&inp.ty, &mut input_names);
    }
    let output_names: std::collections::BTreeSet<String> = match &parsed_recipe.emits {
        Some(decl) => decl.types.iter().cloned().collect(),
        None => parsed_recipe.emit_types(),
    };
    let input_type_refs: Vec<TypeRef> = type_refs
        .iter()
        .filter(|r| input_names.contains(&r.name))
        .cloned()
        .collect();
    let output_type_refs: Vec<TypeRef> = type_refs
        .iter()
        .filter(|r| output_names.contains(&r.name))
        .cloned()
        .collect();

    Ok(PublishPlan {
        types,
        recipe_payload: PublishRequest {
            description,
            category,
            tags,
            recipe,
            type_refs,
            input_type_refs,
            output_type_refs,
            fixtures,
            snapshot,
            base_version,
        },
    })
}

/// Walk a [`FieldType`] and collect every record / Ref<T> / enum name
/// it references into `out`. Recurses through `Array(T)`. Scalars
/// (`String`, `Int`, etc.) contribute nothing. Used by the publish
/// flow to surface the named types an `input <name>: T` declaration
/// pulls in.
fn collect_referenced_type_names(
    ty: &FieldType,
    out: &mut std::collections::BTreeSet<String>,
) {
    match ty {
        FieldType::String | FieldType::Int | FieldType::Double | FieldType::Bool => {}
        FieldType::Array(inner) => collect_referenced_type_names(inner, out),
        FieldType::Record(name) | FieldType::Ref(name) | FieldType::EnumRef(name) => {
            out.insert(name.clone());
        }
    }
}

/// Execute a publish plan against the hub. Publishes each type first
/// (with server-side content-hash dedup making this idempotent across
/// republishes of unchanged types), then posts the recipe with the
/// resolved type pins, then updates the per-recipe sidecar.
///
/// The type publishes inherit the recipe's `description` / `category`
/// / `tags` so a workspace that doesn't carry per-type metadata still
/// publishes something coherent. One metadata triple per publish.
pub async fn publish_from_workspace(
    client: &HubClient,
    workspace: &Workspace,
    recipe_name: &str,
    author: &str,
    description: String,
    category: String,
    tags: Vec<String>,
) -> HubResult<PublishResponse> {
    let mut plan = assemble_publish_plan(
        workspace,
        recipe_name,
        author,
        description.clone(),
        category.clone(),
        tags.clone(),
    )?;

    for (i, ty) in plan.types.iter().enumerate() {
        let prior = client.get_type(author, &ty.name).await?;
        let base_version = prior.as_ref().map(|m| m.latest_version);
        let req = PublishTypeRequest {
            description: description.clone(),
            category: category.clone(),
            tags: tags.clone(),
            source: ty.source.clone(),
            alignments: ty.alignments.clone(),
            field_alignments: ty.field_alignments.clone(),
            base_version,
        };
        let resp = client.publish_type_version(author, &ty.name, &req).await?;
        plan.recipe_payload.type_refs[i].version = resp.version;
    }
    // Each input/output partition was built from `type_refs` with a
    // placeholder `version: 0`; thread the server-resolved versions
    // back through so the recipe publish ships the same pin as
    // `type_refs`.
    let resolved_by_name: BTreeMap<String, u32> = plan
        .recipe_payload
        .type_refs
        .iter()
        .map(|r| (r.name.clone(), r.version))
        .collect();
    for r in plan
        .recipe_payload
        .input_type_refs
        .iter_mut()
        .chain(plan.recipe_payload.output_type_refs.iter_mut())
    {
        if let Some(v) = resolved_by_name.get(&r.name) {
            r.version = *v;
        }
    }

    let resp = client
        .publish_version(author, recipe_name, &plan.recipe_payload)
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

/// Walk the workspace and extract every `share` type as a publishable
/// unit. One entry per `share type Name { ... }` declaration anywhere
/// in the workspace (including the focal recipe).
///
/// The returned source for each type is a standalone fragment beginning
/// with `share type Name` — the hub-side regex demands that header. We
/// recompose the fragment from the AST + source bytes so a type buried
/// in a file alongside other declarations still ships as a clean unit.
fn collect_shared_types(
    workspace: &Workspace,
    focal_source: &str,
) -> HubResult<Vec<SharedTypeSource>> {
    let mut by_name: BTreeMap<String, SharedTypeSource> = BTreeMap::new();

    // Sibling workspace files.
    for entry in &workspace.files {
        let Ok(parsed) = entry.parsed.as_ref() else {
            continue;
        };
        let src = match fs::read_to_string(&entry.path) {
            Ok(s) => s,
            Err(e) => {
                return Err(HubError::Io(io::Error::new(
                    e.kind(),
                    format!("read {}: {e}", entry.path.display()),
                )));
            }
        };
        for ty in &parsed.types {
            if !ty.shared {
                continue;
            }
            let fragment = extract_type_fragment(&src, ty)?;
            let publishable = build_publishable_type(ty, fragment);
            // Two files declaring `share type Foo` is a workspace-level
            // collision the validator surfaces. Here, last-writer-wins
            // by sibling iteration order — the publish flow doesn't try
            // to repair a duplicate, just picks one and lets the
            // validator do the diagnostic work upstream.
            by_name.insert(publishable.name.clone(), publishable);
        }
    }

    // Focal recipe's own share types — already covered by the sibling
    // loop above when the recipe is in `workspace.files`, but we double
    // back over `focal_source` defensively so a lonely-mode call (where
    // the workspace doesn't index the recipe path) still publishes the
    // recipe's shared types. In the workspace case the AST in
    // `workspace.files` is authoritative, so the loop above already
    // populated the map; this is a safety net for non-workspace publish
    // flows.
    let parsed = forage_core::parse(focal_source).map_err(|e| {
        HubError::Generic(format!("re-parse focal recipe: {e}"))
    })?;
    for ty in &parsed.types {
        if !ty.shared {
            continue;
        }
        if by_name.contains_key(&ty.name) {
            continue;
        }
        let fragment = extract_type_fragment(focal_source, ty)?;
        let publishable = build_publishable_type(ty, fragment);
        by_name.insert(publishable.name.clone(), publishable);
    }

    Ok(by_name.into_values().collect())
}

fn build_publishable_type(ty: &RecipeType, source: String) -> SharedTypeSource {
    let alignments = ty
        .alignments
        .iter()
        .map(core_alignment_to_wire)
        .collect();
    let field_alignments = ty
        .fields
        .iter()
        .map(|f| TypeFieldAlignment {
            field: f.name.clone(),
            alignment: f.alignment.as_ref().map(core_alignment_to_wire),
        })
        .collect();
    SharedTypeSource {
        name: ty.name.clone(),
        source,
        alignments,
        field_alignments,
    }
}

fn core_alignment_to_wire(a: &CoreAlignmentUri) -> AlignmentUri {
    AlignmentUri {
        ontology: a.ontology.clone(),
        term: a.term.clone(),
    }
}

/// Cut a single `share type Name { … }` block out of a source file by
/// span. The AST carries a byte-range span for the whole declaration;
/// we expand backwards over any leading `share` keyword to ensure the
/// fragment publishes-and-round-trips as a publishable header.
///
/// The hub's TYPE_HEAD_NAME_RE expects the fragment to begin with
/// `share type Name` after comments / whitespace, so the cut must
/// preserve the `share` token.
fn extract_type_fragment(src: &str, ty: &RecipeType) -> HubResult<String> {
    let start = ty.span.start;
    let end = ty.span.end;
    if start > end || end > src.len() {
        return Err(HubError::Generic(format!(
            "type {:?} has span [{start}..{end}] but source is {} bytes",
            ty.name,
            src.len()
        )));
    }
    let bytes = src.as_bytes();

    // The span starts at the `type` keyword; back up over whitespace
    // to include the `share` modifier. The parser stamps the span on
    // the `type` token, so the `share` (if any) sits to its left
    // separated only by whitespace.
    let mut cursor = start;
    while cursor > 0 && bytes[cursor - 1].is_ascii_whitespace() {
        cursor -= 1;
    }
    let share_start = cursor.saturating_sub(5);
    if cursor >= 5 && &src[share_start..cursor] == "share" {
        cursor = share_start;
    } else {
        // Caller wouldn't ship a non-`share` type as a publishable
        // unit; the loop above only inserts `share`d types into the
        // map, so this branch is structurally unreachable from
        // `collect_shared_types`. Surface it as an error rather than
        // synthesizing a `share` prefix in case the caller wires this
        // helper into a path where the invariant doesn't hold.
        return Err(HubError::Generic(format!(
            "type {:?} is not preceded by `share`; only `share` types are publishable",
            ty.name
        )));
    }

    let fragment = src[cursor..end].to_string();
    let trimmed = fragment.trim_end();
    let mut out = String::with_capacity(trimmed.len() + 1);
    out.push_str(trimmed);
    out.push('\n');
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

/// Write the recipe source to `recipe_path`. Pure I/O; type
/// materialization is its own step via [`write_type_to_workspace`].
fn write_recipe(recipe_path: &Path, artifact: &PackageVersion) -> HubResult<()> {
    if let Some(parent) = recipe_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(recipe_path, &artifact.recipe)?;
    Ok(())
}

/// Lay one type artifact into the workspace as `<workspace>/<Name>.forage`.
/// Refuses to overwrite a file that's not a hub-synced copy at an
/// older version — same rule as the recipe path.
fn write_type_to_workspace(workspace_root: &Path, tv: &TypeVersion) -> HubResult<()> {
    let path = workspace_root.join(format!("{}.forage", tv.name));
    if path.exists() {
        // For a freshly-published or freshly-downloaded type the file
        // either doesn't exist (first sync) or already holds a
        // hub-synced copy of the same type. We don't track per-type
        // sidecars yet; refuse to clobber a file that's locally owned
        // (different first line / no marker) so the user sees a clear
        // error instead of a silent overwrite.
        let current = fs::read_to_string(&path).map_err(|e| {
            HubError::Io(io::Error::new(
                e.kind(),
                format!("read existing {}: {e}", path.display()),
            ))
        })?;
        if !current.trim_start().starts_with("share type") {
            return Err(HubError::Generic(format!(
                "{} already exists locally and is not a `share type` file; \
                 remove it before syncing",
                path.display()
            )));
        }
    }
    fs::write(&path, &tv.source)?;
    Ok(())
}

/// Cache root holding hub-fetched types. One file per
/// `(author, name, version)`: `<cache>/types/<author>/<Name>/<v>.forage`.
/// The workspace loader reads this subtree when resolving lockfile
/// `[types]` pins.
pub fn type_cache_path(cache_root: &Path, author: &str, name: &str, version: u32) -> PathBuf {
    cache_root
        .join("types")
        .join(author)
        .join(name)
        .join(format!("{version}.forage"))
}

/// Mirror a `TypeVersion` into the type cache. Returns the on-disk
/// path of the cached source so callers can record it in the lockfile
/// digest if desired.
fn write_type_to_cache(cache_root: &Path, tv: &TypeVersion) -> HubResult<PathBuf> {
    let path = type_cache_path(cache_root, &tv.author, &tv.name, tv.version);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, &tv.source)?;
    Ok(path)
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

    fn type_version(author: &str, name: &str, version: u32, source: &str) -> TypeVersion {
        TypeVersion {
            author: author.into(),
            name: name.into(),
            version,
            source: source.into(),
            alignments: Vec::new(),
            field_alignments: Vec::new(),
            base_version: None,
            published_at: 0,
            published_by: author.into(),
        }
    }

    fn artifact(author: &str, slug: &str, v: u32, type_refs: Vec<TypeRef>) -> PackageVersion {
        PackageVersion {
            author: author.into(),
            slug: slug.into(),
            version: v,
            recipe: format!(
                "recipe \"{slug}\"\nengine http\n\nstep s {{ method \"GET\" url \"https://example.test\" }}\n"
            ),
            type_refs,
            input_type_refs: Vec::new(),
            output_type_refs: Vec::new(),
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

    /// `write_recipe` lays the recipe file at the supplied path;
    /// `write_fixtures_and_snapshot` lays workspace data keyed on the
    /// recipe-name. Types ride through their own cache + workspace
    /// writes.
    #[test]
    fn writers_lay_flat_workspace_shape() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path();
        let art = artifact("alice", "zen-leaf", 4, Vec::new());
        let recipe_path = ws.join("zen-leaf.forage");
        write_recipe(&recipe_path, &art).unwrap();
        write_fixtures_and_snapshot(ws, &art.slug, &art).unwrap();

        assert!(recipe_path.is_file());
        assert!(ws.join("_fixtures").join("zen-leaf.jsonl").is_file());
        assert!(ws.join("_snapshots").join("zen-leaf.json").is_file());
        assert!(!ws.join("zen-leaf").join("recipe.forage").exists());
    }

    #[test]
    fn write_type_to_workspace_lands_as_named_file() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path();
        let tv = type_version(
            "alice",
            "Product",
            1,
            "share type Product {\n    id: String\n}\n",
        );
        write_type_to_workspace(ws, &tv).unwrap();
        assert!(ws.join("Product.forage").is_file());
        let back = fs::read_to_string(ws.join("Product.forage")).unwrap();
        assert!(back.starts_with("share type Product"));
    }

    #[test]
    fn write_type_to_workspace_refuses_non_share_clobber() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path();
        // A locally-authored file that doesn't start with `share type`
        // — typically the user's own recipe or scratch.
        fs::write(
            ws.join("Product.forage"),
            "recipe \"local\"\nengine http\n",
        )
        .unwrap();
        let tv = type_version(
            "alice",
            "Product",
            1,
            "share type Product {\n    id: String\n}\n",
        );
        let err = write_type_to_workspace(ws, &tv).unwrap_err();
        assert!(format!("{err}").contains("already exists"));
    }

    #[test]
    fn write_type_to_cache_lays_versioned_file() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = tmp.path();
        let tv = type_version("alice", "Product", 4, "share type Product {}\n");
        let path = write_type_to_cache(cache, &tv).unwrap();
        assert_eq!(
            path,
            cache.join("types").join("alice").join("Product").join("4.forage"),
        );
        assert!(path.is_file());
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

    /// `assemble_publish_plan` resolves the recipe via
    /// `Workspace::recipe_by_name`, extracts every `share` type as its
    /// own publishable unit, and threads the workspace's fixtures /
    /// snapshot / sidecar through to the recipe payload. The
    /// recipe-side `type_refs` are placeholders (version: 0) until the
    /// driver overwrites them with server-resolved values.
    #[test]
    fn assemble_plan_extracts_share_types_individually() {
        let tmp = tempfile::tempdir().unwrap();
        let ws_root = tmp.path();
        fs::write(
            ws_root.join("forage.toml"),
            "name = \"alice/bar\"\ndescription = \"\"\ncategory = \"x\"\ntags = []\n",
        )
        .unwrap();
        fs::write(
            ws_root.join("bar.forage"),
            "recipe \"bar\"\nengine http\nemits Product\n\
             step s { method \"GET\" url \"https://example.test\" }\n",
        )
        .unwrap();
        fs::write(
            ws_root.join("shared.forage"),
            "share type Product { id: String }\n\
             share type Variant { id: String }\n",
        )
        .unwrap();
        // A sibling with only file-local decls must stay home.
        fs::write(
            ws_root.join("local-only.forage"),
            "type LocalOnly { id: String }\n",
        )
        .unwrap();

        let ws = forage_core::workspace::load(ws_root).unwrap();
        let plan = assemble_publish_plan(
            &ws,
            "bar",
            "alice",
            "desc".into(),
            "scrape".into(),
            vec!["t".into()],
        )
        .unwrap();
        assert_eq!(plan.types.len(), 2, "Product + Variant, not LocalOnly");
        let names: Vec<&str> = plan.types.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["Product", "Variant"]);
        for ty in &plan.types {
            assert!(
                ty.source.trim_start().starts_with("share type"),
                "fragment must start with `share type`: {:?}",
                ty.source,
            );
        }
        assert_eq!(plan.recipe_payload.type_refs.len(), 2);
        for r in &plan.recipe_payload.type_refs {
            assert_eq!(r.author, "alice");
            assert_eq!(r.version, 0, "driver overwrites this with the server-resolved version");
        }
        // `output Product` in the recipe header surfaces in
        // `output_type_refs`; the unread `Variant` doesn't. Both are
        // still pinned in `type_refs` because they ride with the
        // workspace.
        let output_names: Vec<&str> = plan
            .recipe_payload
            .output_type_refs
            .iter()
            .map(|r| r.name.as_str())
            .collect();
        assert_eq!(output_names, vec!["Product"]);
        assert!(
            plan.recipe_payload.input_type_refs.is_empty(),
            "recipe with no `input` declarations contributes no input_type_refs",
        );
    }

    #[test]
    fn assemble_plan_without_sidecar_means_first_publish() {
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
        let plan = assemble_publish_plan(
            &ws,
            "fresh",
            "alice",
            "desc".into(),
            "scrape".into(),
            vec![],
        )
        .unwrap();
        assert_eq!(plan.recipe_payload.base_version, None);
        assert!(plan.types.is_empty());
    }

    #[test]
    fn assemble_plan_errors_on_unknown_recipe_name() {
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
        let err = assemble_publish_plan(
            &ws,
            "missing",
            "alice",
            "".into(),
            "x".into(),
            vec![],
        )
        .unwrap_err();
        assert!(format!("{err}").contains("missing"), "unexpected: {err}");
    }
}
