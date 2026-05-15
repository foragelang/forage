//! Workspace shape migration: legacy
//! `<slug>/recipe.forage` plus `<slug>/fixtures/` plus
//! `<slug>/snapshot.json` becomes the flat shape the rest of the
//! toolchain now expects.
//!
//! Acts as a one-shot script. The runtime does not support the legacy
//! layout going forward — running an unmigrated workspace fails to
//! validate or replay. Dry-run is the default; `--apply` mutates the
//! filesystem.
//!
//! What gets restructured, in this order:
//!
//! 1. **Recipes**: every `<root>/<slug>/recipe.forage` is parsed,
//!    its header name is read, and the file is moved to
//!    `<root>/<header-name>.forage`. Header name beats folder slug
//!    when they differ.
//! 2. **Fixtures**: every `<root>/<slug>/fixtures/*.jsonl` is read as
//!    a stream of `Capture`s and re-emitted at
//!    `<root>/_fixtures/<header-name>.jsonl`. `inputs.json` is left
//!    in place (the new CLI takes `--inputs <path>` explicitly) but
//!    its location is logged.
//! 3. **Snapshots**: `<root>/<slug>/snapshot.json` moves to
//!    `<root>/_snapshots/<header-name>.json`.
//! 4. **Share-keyword insertion**: every header-less `.forage` file
//!    at the workspace root has each of its bare `type`/`enum`/`fn`
//!    declarations rewritten with a leading `share ` so they remain
//!    workspace-visible (the "header-less files at the root contribute
//!    shared types" convention is gone; visibility is now explicit).
//! 5. **Cleanup**: emptied `<slug>/fixtures/` and `<slug>/` directories
//!    are removed. Anything still inside (stray files the user kept)
//!    blocks the cleanup with a `warn!`; the partial migration is
//!    surfaced rather than papered over.
//!
//! Target collisions ("a file already lives at the new path") abort
//! the migration before any write. Each kind of action logs through
//! `tracing` so the operator has a play-by-play.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use forage_core::ast::{FnDecl, ForageFile, RecipeEnum, RecipeType};
use forage_core::parse;
use forage_core::workspace::{
    FIXTURES_DIR, MANIFEST_NAME, SNAPSHOTS_DIR, fixtures_path, snapshot_path,
};
use forage_replay::{Capture, read_jsonl, write_jsonl};

/// What the migration plans to do at `root`. Built without side effects
/// so `--dry-run` can print it; `apply` consumes the plan and mutates
/// the filesystem.
#[derive(Debug, Default)]
pub struct MigrationPlan {
    pub recipe_moves: Vec<RecipeMove>,
    pub fixture_moves: Vec<FixtureMove>,
    pub snapshot_moves: Vec<SnapshotMove>,
    pub share_inserts: Vec<ShareInserts>,
    /// `<slug>/fixtures/inputs.json` files that survive the migration —
    /// the new CLI doesn't load these automatically; they're surfaced so
    /// the user knows where to point `forage run --inputs <path>`.
    pub inputs_files: Vec<PathBuf>,
    /// Directories that will be removed once their contents have been
    /// moved out.
    pub dirs_to_remove: Vec<PathBuf>,
}

#[derive(Debug)]
pub struct RecipeMove {
    pub from: PathBuf,
    pub to: PathBuf,
    pub recipe_name: String,
}

#[derive(Debug)]
pub struct FixtureMove {
    pub recipe_name: String,
    /// Source `.jsonl` files inside `<slug>/fixtures/` to concatenate,
    /// in deterministic order (sorted by file name).
    pub sources: Vec<PathBuf>,
    pub to: PathBuf,
}

#[derive(Debug)]
pub struct SnapshotMove {
    pub from: PathBuf,
    pub to: PathBuf,
}

/// One header-less `.forage` file's worth of `share` insertions —
/// every bare `type`/`enum`/`fn` declaration gets a `share ` prefix.
#[derive(Debug)]
pub struct ShareInserts {
    pub path: PathBuf,
    /// Byte offsets in the original source where `share ` should be
    /// inserted, ascending. Apply in reverse so earlier offsets stay
    /// valid.
    pub offsets: Vec<usize>,
}

/// Build the migration plan for `root`. Does not touch the
/// filesystem.
pub fn plan(root: &Path) -> Result<MigrationPlan> {
    let root = root
        .canonicalize()
        .with_context(|| format!("canonicalizing {}", root.display()))?;
    let manifest = root.join(MANIFEST_NAME);
    if !manifest.is_file() {
        bail!(
            "{} is not a Forage workspace (no {} at the root)",
            root.display(),
            MANIFEST_NAME,
        );
    }

    let mut plan = MigrationPlan::default();

    // Scan immediate children of the workspace root: each subdirectory
    // is a candidate legacy `<slug>/` recipe directory.
    let mut entries: Vec<PathBuf> = fs::read_dir(&root)
        .with_context(|| format!("reading {}", root.display()))?
        .filter_map(|r| r.ok().map(|e| e.path()))
        .collect();
    entries.sort();

    // First pass: collect legacy recipes and reserve target paths so we
    // can detect intra-migration collisions (two recipes whose header
    // names collide at the new flat layout).
    let mut reserved_recipe_files: BTreeMap<PathBuf, PathBuf> = BTreeMap::new();
    let mut reserved_fixtures: BTreeMap<PathBuf, String> = BTreeMap::new();
    let mut reserved_snapshots: BTreeMap<PathBuf, String> = BTreeMap::new();

    for child in &entries {
        if !child.is_dir() {
            continue;
        }
        let dir_name = match child.file_name().and_then(|s| s.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        // Skip the daemon's hidden state and the new data dirs (so a
        // re-run after a partial migration doesn't try to walk into
        // them).
        if dir_name.starts_with('.') || dir_name == FIXTURES_DIR || dir_name == SNAPSHOTS_DIR {
            continue;
        }
        let legacy_recipe = child.join("recipe.forage");
        if !legacy_recipe.is_file() {
            continue;
        }
        let source = fs::read_to_string(&legacy_recipe)
            .with_context(|| format!("reading {}", legacy_recipe.display()))?;
        let file = parse(&source)
            .map_err(|e| anyhow::anyhow!("parse {}: {e}", legacy_recipe.display()))?;
        let Some(name) = file.recipe_name().map(str::to_string) else {
            bail!(
                "{} has no `recipe \"<name>\"` header; cannot pick a flat-shape filename",
                legacy_recipe.display(),
            );
        };

        let target_recipe = root.join(format!("{name}.forage"));
        if target_recipe.exists() {
            bail!(
                "{} already exists; refusing to move {} on top of it",
                target_recipe.display(),
                legacy_recipe.display(),
            );
        }
        if let Some(other) = reserved_recipe_files.get(&target_recipe) {
            bail!(
                "two legacy recipes both want to flatten to {}: {} and {}",
                target_recipe.display(),
                other.display(),
                legacy_recipe.display(),
            );
        }
        reserved_recipe_files.insert(target_recipe.clone(), legacy_recipe.clone());

        plan.recipe_moves.push(RecipeMove {
            from: legacy_recipe,
            to: target_recipe,
            recipe_name: name.clone(),
        });

        // Fixtures.
        let fixtures_dir = child.join("fixtures");
        let mut fixture_inputs: Option<PathBuf> = None;
        let mut fixture_sources: Vec<PathBuf> = Vec::new();
        let mut leftover_in_fixtures: Vec<PathBuf> = Vec::new();
        if fixtures_dir.is_dir() {
            let mut fx_entries: Vec<PathBuf> = fs::read_dir(&fixtures_dir)
                .with_context(|| format!("reading {}", fixtures_dir.display()))?
                .filter_map(|r| r.ok().map(|e| e.path()))
                .collect();
            fx_entries.sort();
            for fx in fx_entries {
                if !fx.is_file() {
                    leftover_in_fixtures.push(fx);
                    continue;
                }
                let name = fx
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or_default()
                    .to_string();
                if name == "inputs.json" {
                    fixture_inputs = Some(fx);
                    continue;
                }
                let ext = fx
                    .extension()
                    .and_then(|s| s.to_str())
                    .unwrap_or_default();
                if ext == "jsonl" {
                    fixture_sources.push(fx);
                } else {
                    leftover_in_fixtures.push(fx);
                }
            }
        }
        if !fixture_sources.is_empty() {
            let to = fixtures_path(&root, &name);
            if to.exists() {
                bail!(
                    "{} already exists; refusing to overwrite while migrating {}",
                    to.display(),
                    fixtures_dir.display(),
                );
            }
            if let Some(other) = reserved_fixtures.get(&to) {
                bail!(
                    "two legacy fixtures both want to flatten to {}: recipe {:?} and {:?}",
                    to.display(),
                    other,
                    name,
                );
            }
            reserved_fixtures.insert(to.clone(), name.clone());
            plan.fixture_moves.push(FixtureMove {
                recipe_name: name.clone(),
                sources: fixture_sources,
                to,
            });
        }
        if let Some(p) = &fixture_inputs {
            plan.inputs_files.push(p.clone());
        }

        // Snapshot.
        let legacy_snapshot = child.join("snapshot.json");
        if legacy_snapshot.is_file() {
            let to = snapshot_path(&root, &name);
            if to.exists() {
                bail!(
                    "{} already exists; refusing to overwrite while migrating {}",
                    to.display(),
                    legacy_snapshot.display(),
                );
            }
            if let Some(other) = reserved_snapshots.get(&to) {
                bail!(
                    "two legacy snapshots both want to flatten to {}: recipe {:?} and {:?}",
                    to.display(),
                    other,
                    name,
                );
            }
            reserved_snapshots.insert(to.clone(), name.clone());
            plan.snapshot_moves.push(SnapshotMove {
                from: legacy_snapshot,
                to,
            });
        }

        // Schedule directory removals. `<slug>/fixtures/` first so its
        // parent `<slug>/` can be removed afterwards. The actual
        // removal still checks emptiness at apply time — a stray file
        // outside this migration's awareness aborts the cleanup with a
        // warn rather than silently losing data.
        if fixtures_dir.is_dir() && leftover_in_fixtures.is_empty() && fixture_inputs.is_none() {
            // `inputs.json`, when present, stays in place and pins
            // `<slug>/fixtures/` open. Same goes for any leftover file
            // we don't recognize.
            plan.dirs_to_remove.push(fixtures_dir);
        }
        // The recipe directory only goes when nothing's left in it —
        // verified at apply time. We still schedule it so the user
        // sees the intent in dry-run.
        plan.dirs_to_remove.push(child.clone());
    }

    // Second pass: header-less `.forage` files at the workspace root.
    // Subdirectory `.forage` files are recipe-bearing (handled above)
    // or unrelated; the share-insertion rule only applies to
    // header-less files at the workspace root.
    for child in &entries {
        if !child.is_file() {
            continue;
        }
        if child.extension().is_none_or(|e| e != "forage") {
            continue;
        }
        let source = fs::read_to_string(child)
            .with_context(|| format!("reading {}", child.display()))?;
        let file = parse(&source)
            .map_err(|e| anyhow::anyhow!("parse {}: {e}", child.display()))?;
        if !file.recipe_headers.is_empty() {
            continue;
        }
        let offsets = share_insertion_offsets(&file);
        if offsets.is_empty() {
            continue;
        }
        plan.share_inserts.push(ShareInserts {
            path: child.clone(),
            offsets,
        });
    }

    Ok(plan)
}

/// Apply `plan` to disk. Logs each action and aborts on the first
/// failure; the half-state is surfaced so the operator can decide
/// whether to re-run after fixing the cause or restore from a backup.
pub fn apply(plan: &MigrationPlan) -> Result<()> {
    for r in &plan.recipe_moves {
        if let Some(parent) = r.to.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        tracing::info!(
            recipe = %r.recipe_name,
            from = %r.from.display(),
            to = %r.to.display(),
            "migrate recipe: rename"
        );
        fs::rename(&r.from, &r.to)
            .with_context(|| format!("renaming {} → {}", r.from.display(), r.to.display()))?;
    }

    for f in &plan.fixture_moves {
        let mut captures: Vec<Capture> = Vec::new();
        for src in &f.sources {
            let mut parsed = read_jsonl(src)
                .map_err(|e| anyhow::anyhow!("read {}: {e}", src.display()))?;
            captures.append(&mut parsed);
        }
        tracing::info!(
            recipe = %f.recipe_name,
            captures = captures.len(),
            to = %f.to.display(),
            sources = ?f.sources.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
            "migrate fixtures: concatenate"
        );
        write_jsonl(&f.to, &captures)
            .map_err(|e| anyhow::anyhow!("write {}: {e}", f.to.display()))?;
        for src in &f.sources {
            fs::remove_file(src)
                .with_context(|| format!("removing {}", src.display()))?;
        }
    }

    for s in &plan.snapshot_moves {
        if let Some(parent) = s.to.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        tracing::info!(
            from = %s.from.display(),
            to = %s.to.display(),
            "migrate snapshot: rename"
        );
        fs::rename(&s.from, &s.to)
            .with_context(|| format!("renaming {} → {}", s.from.display(), s.to.display()))?;
    }

    for s in &plan.share_inserts {
        let source = fs::read_to_string(&s.path)
            .with_context(|| format!("reading {}", s.path.display()))?;
        let updated = apply_share_inserts(&source, &s.offsets);
        tracing::info!(
            path = %s.path.display(),
            inserts = s.offsets.len(),
            "migrate declarations: prefix bare types/enums/fns with `share`"
        );
        fs::write(&s.path, updated)
            .with_context(|| format!("writing {}", s.path.display()))?;
    }

    for p in &plan.inputs_files {
        tracing::warn!(
            path = %p.display(),
            "leaving legacy inputs.json in place; pass it explicitly with `forage run --inputs <path>`"
        );
    }

    // Directory cleanup. Remove only directories that are actually
    // empty — a leftover file the migration didn't know about pins the
    // parent open and surfaces a warning.
    for dir in &plan.dirs_to_remove {
        match fs::read_dir(dir) {
            Ok(mut it) => {
                if it.next().is_some() {
                    tracing::warn!(
                        path = %dir.display(),
                        "not removing directory: unexpected leftover content"
                    );
                    continue;
                }
            }
            Err(e) => {
                tracing::warn!(
                    path = %dir.display(),
                    "not removing directory: {}",
                    e,
                );
                continue;
            }
        }
        tracing::info!(
            path = %dir.display(),
            "migrate cleanup: remove empty directory"
        );
        fs::remove_dir(dir)
            .with_context(|| format!("removing {}", dir.display()))?;
    }

    Ok(())
}

/// Compute the byte offsets in `file`'s source where `share ` should be
/// inserted. Span starts mark the `type`/`enum`/`fn` keyword (the
/// parser places `share` outside the recorded span), so the insertion
/// point is just `span.start`. Returned ascending; callers apply in
/// reverse.
fn share_insertion_offsets(file: &ForageFile) -> Vec<usize> {
    let mut out: Vec<usize> = Vec::new();
    let push_if_bare_type = |t: &RecipeType, out: &mut Vec<usize>| {
        if !t.shared {
            out.push(t.span.start);
        }
    };
    let push_if_bare_enum = |e: &RecipeEnum, out: &mut Vec<usize>| {
        if !e.shared {
            out.push(e.span.start);
        }
    };
    let push_if_bare_fn = |f: &FnDecl, out: &mut Vec<usize>| {
        if !f.shared {
            out.push(f.span.start);
        }
    };
    for t in &file.types {
        push_if_bare_type(t, &mut out);
    }
    for e in &file.enums {
        push_if_bare_enum(e, &mut out);
    }
    for f in &file.functions {
        push_if_bare_fn(f, &mut out);
    }
    out.sort_unstable();
    out
}

/// Apply `share ` insertions at the given byte offsets. Offsets must
/// be ascending; we apply in reverse so each splice leaves earlier
/// offsets pointing at the same bytes.
fn apply_share_inserts(source: &str, offsets: &[usize]) -> String {
    let mut out = source.to_string();
    for &off in offsets.iter().rev() {
        out.insert_str(off, "share ");
    }
    out
}

/// Format a plan as a multi-line dry-run report. Empty plans render
/// as a single "nothing to do" line so the user sees explicit
/// confirmation rather than empty output.
pub fn render_plan(plan: &MigrationPlan) -> String {
    let mut out = String::new();
    if plan.recipe_moves.is_empty()
        && plan.fixture_moves.is_empty()
        && plan.snapshot_moves.is_empty()
        && plan.share_inserts.is_empty()
        && plan.inputs_files.is_empty()
        && plan.dirs_to_remove.is_empty()
    {
        out.push_str("nothing to migrate; workspace is already in the flat shape\n");
        return out;
    }
    for r in &plan.recipe_moves {
        out.push_str(&format!(
            "move recipe   {} → {} (recipe \"{}\")\n",
            r.from.display(),
            r.to.display(),
            r.recipe_name,
        ));
    }
    for f in &plan.fixture_moves {
        out.push_str(&format!(
            "merge fixtures [{} files] → {} (recipe \"{}\")\n",
            f.sources.len(),
            f.to.display(),
            f.recipe_name,
        ));
        for src in &f.sources {
            out.push_str(&format!("    · {}\n", src.display()));
        }
    }
    for s in &plan.snapshot_moves {
        out.push_str(&format!(
            "move snapshot {} → {}\n",
            s.from.display(),
            s.to.display(),
        ));
    }
    for s in &plan.share_inserts {
        out.push_str(&format!(
            "share-prefix {} bare declaration(s) in {}\n",
            s.offsets.len(),
            s.path.display(),
        ));
    }
    for p in &plan.inputs_files {
        out.push_str(&format!(
            "leave inputs  {} (pass via --inputs at run time)\n",
            p.display(),
        ));
    }
    for d in &plan.dirs_to_remove {
        out.push_str(&format!("remove dir    {} (if empty after moves)\n", d.display()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, body: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, body).unwrap();
    }

    const MANIFEST: &str = "description = \"\"\ncategory = \"\"\ntags = []\n";

    /// End-to-end: a legacy workspace with two recipes, fixtures, a
    /// snapshot, and a header-less declarations file all land in the
    /// flat shape.
    #[test]
    fn full_migration_lands_in_flat_shape() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(&root.join(MANIFEST_NAME), MANIFEST);
        write(
            &root.join("remedy-baltimore").join("recipe.forage"),
            "recipe \"remedy-baltimore\"\nengine http\n",
        );
        write(
            &root.join("trilogy-med").join("recipe.forage"),
            "recipe \"trilogy-med\"\nengine http\n",
        );
        // One JSONL of captures for remedy-baltimore.
        write(
            &root
                .join("remedy-baltimore")
                .join("fixtures")
                .join("captures.jsonl"),
            "{\"kind\":\"http\",\"url\":\"https://x\",\"method\":\"GET\",\"status\":200,\"body\":\"{}\"}\n",
        );
        // inputs.json stays in place (warn).
        write(
            &root
                .join("remedy-baltimore")
                .join("fixtures")
                .join("inputs.json"),
            "{}\n",
        );
        // Snapshot for remedy-baltimore.
        write(
            &root.join("remedy-baltimore").join("snapshot.json"),
            "{\"records\":[],\"diagnostic\":{\"unmet_expectations\":[]}}\n",
        );
        // Header-less declarations file: bare types and one already-share.
        write(
            &root.join("cannabis.forage"),
            "type Dispensary { id: String }\nshare type Product { id: String }\nenum Status { ACTIVE, INACTIVE }\n",
        );

        let plan = plan(root).unwrap();
        apply(&plan).unwrap();

        // Recipes moved.
        assert!(root.join("remedy-baltimore.forage").is_file());
        assert!(root.join("trilogy-med.forage").is_file());
        // Fixtures concatenated.
        let fx_path = root.join("_fixtures").join("remedy-baltimore.jsonl");
        assert!(fx_path.is_file(), "fixture file missing");
        let fx = read_jsonl(&fx_path).unwrap();
        assert_eq!(fx.len(), 1);
        // Snapshot moved.
        assert!(
            root.join("_snapshots").join("remedy-baltimore.json").is_file(),
            "snapshot did not move",
        );
        // Cannabis got share-prefixed where missing, untouched where
        // already share.
        let cannabis = fs::read_to_string(root.join("cannabis.forage")).unwrap();
        assert!(cannabis.contains("share type Dispensary"));
        assert!(cannabis.contains("share type Product"));
        assert!(cannabis.contains("share enum Status"));
        // Old <slug>/ dirs are gone except where inputs.json kept
        // the fixtures dir open.
        assert!(!root.join("trilogy-med").exists());
        assert!(
            root.join("remedy-baltimore").join("fixtures").join("inputs.json").is_file(),
            "inputs.json must stay where it was",
        );
        assert!(
            !root.join("remedy-baltimore").join("recipe.forage").exists(),
            "legacy recipe file must be moved",
        );
    }

    /// Running the migration twice is a no-op the second time. A flat
    /// workspace has nothing to move and no header-less declarations
    /// to prefix.
    #[test]
    fn migration_is_idempotent_on_flat_workspace() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(&root.join(MANIFEST_NAME), MANIFEST);
        write(
            &root.join("flat.forage"),
            "recipe \"flat\"\nengine http\n",
        );
        write(
            &root.join("decls.forage"),
            "share type Foo { id: String }\n",
        );

        let p1 = plan(root).unwrap();
        apply(&p1).unwrap();
        let p2 = plan(root).unwrap();
        assert!(p2.recipe_moves.is_empty());
        assert!(p2.fixture_moves.is_empty());
        assert!(p2.snapshot_moves.is_empty());
        assert!(p2.share_inserts.is_empty());
        assert!(p2.dirs_to_remove.is_empty());
    }

    /// The flat-shape file basename is the recipe header name, NOT the
    /// folder slug, when they differ. A recipe filed under
    /// `old-slug/recipe.forage` whose header says `recipe
    /// "new-name"` lands at `new-name.forage`.
    #[test]
    fn header_name_wins_over_folder_slug() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(&root.join(MANIFEST_NAME), MANIFEST);
        write(
            &root.join("old-slug").join("recipe.forage"),
            "recipe \"new-name\"\nengine http\n",
        );
        let plan = plan(root).unwrap();
        apply(&plan).unwrap();
        assert!(root.join("new-name.forage").is_file());
        assert!(!root.join("old-slug.forage").exists());
        assert!(!root.join("old-slug").exists());
    }

    /// Share insertion is idempotent: a file that already carries
    /// `share` on every decl is left untouched, and re-running the
    /// migration does nothing more.
    #[test]
    fn share_insertion_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(&root.join(MANIFEST_NAME), MANIFEST);
        let decls = "share type Foo { id: String }\nshare enum Status { ACTIVE }\nshare fn double($x) { $x }\n";
        write(&root.join("shared.forage"), decls);

        let p1 = plan(root).unwrap();
        assert!(p1.share_inserts.is_empty(), "no bare decls; nothing to prefix");
        apply(&p1).unwrap();
        assert_eq!(fs::read_to_string(root.join("shared.forage")).unwrap(), decls);
    }

    /// Insertion preserves the rest of the source: comments, blank
    /// lines, and existing `share`-marked decls survive byte-for-byte.
    #[test]
    fn share_insertion_preserves_surrounding_text() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(&root.join(MANIFEST_NAME), MANIFEST);
        let src = "// header comment\n\ntype Foo {\n    id: String\n}\n\nshare type Bar { id: String }\n\nenum Status {\n    ACTIVE,\n    INACTIVE,\n}\n";
        write(&root.join("cannabis.forage"), src);
        let plan = plan(root).unwrap();
        apply(&plan).unwrap();
        let updated = fs::read_to_string(root.join("cannabis.forage")).unwrap();
        let expected = "// header comment\n\nshare type Foo {\n    id: String\n}\n\nshare type Bar { id: String }\n\nshare enum Status {\n    ACTIVE,\n    INACTIVE,\n}\n";
        assert_eq!(updated, expected);
    }

    /// Plan fails when the destination already exists. The caller can
    /// fix the conflict (delete the stale flat-shape file, finish
    /// the partial migration manually) and re-run.
    #[test]
    fn plan_errors_on_destination_collision() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(&root.join(MANIFEST_NAME), MANIFEST);
        write(
            &root.join("zen-leaf-elkridge").join("recipe.forage"),
            "recipe \"zen-leaf-elkridge\"\nengine http\n",
        );
        // Already-migrated file at the new location.
        write(
            &root.join("zen-leaf-elkridge.forage"),
            "recipe \"zen-leaf-elkridge\"\nengine http\n",
        );
        let err = plan(root).unwrap_err();
        assert!(
            err.to_string().contains("already exists"),
            "error should mention the destination collision; got: {err}",
        );
    }

    /// A workspace without a `forage.toml` isn't a workspace; the
    /// planner must refuse rather than try to operate on a random
    /// directory.
    #[test]
    fn plan_errors_without_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        let err = plan(tmp.path()).unwrap_err();
        assert!(err.to_string().contains("forage.toml"));
    }
}
