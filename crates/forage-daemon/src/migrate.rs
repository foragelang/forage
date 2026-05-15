//! One-shot data migration from path-derived slug keying to recipe
//! header-name keying. Runs when `Daemon::open` is the first opener to
//! advance the daemon DB past schema v3.
//!
//! Pre-Phase-4 the daemon keyed every Run row, deployed-version row,
//! deployment directory, and default output-store file on the
//! path-derived slug (`<slug>/recipe.forage` or `<slug>.forage`). The
//! v3 schema rename in `db::apply_migrations` shifts the column names
//! over but doesn't touch the values; if any pre-existing row still
//! holds a path-derived slug whose underlying recipe declares a
//! different header name, this pass reconciles:
//!
//! - `runs.recipe_name` and `deployed_versions.recipe_name` row values
//!   move from the slug to the header name.
//! - `runs.output` paths whose basename matches the legacy default
//!   (`<workspace>/.forage/data/<slug>.sqlite`) get redirected to the
//!   header-name file; the SQLite file itself is renamed.
//! - `<workspace>/.forage/deployments/<slug>/` directories move to
//!   `<workspace>/.forage/deployments/<header-name>/`.
//!
//! Rows whose `recipe_name` doesn't resolve to any workspace recipe
//! are left alone with a `warn!` log. The daemon doesn't delete user
//! data, even mid-migration; a recipe deleted from disk while a Run
//! row still references it is the user's call to clean up.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, params};

use crate::error::DaemonError;
use forage_core::workspace::discover;

/// Path-derived slug for a `.forage` file inside `root`. Mirrors how
/// the Studio wire used to key recipes before Phase 7: legacy
/// `<root>/<slug>/recipe.forage` files derive `<slug>` from the
/// containing folder; anything else falls back to the file stem. The
/// only consumer is this legacy-keying reconciliation pass — every
/// other surface in the workspace now keys on the recipe header name.
fn slug_from_path(root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).unwrap_or(path);
    let components: Vec<_> = rel.components().collect();
    if components.len() == 2
        && let (Some(dir), Some(file)) = (components.first(), components.get(1))
        && file.as_os_str() == "recipe.forage"
    {
        return Some(dir.as_os_str().to_string_lossy().into_owned());
    }
    path.file_stem().map(|s| s.to_string_lossy().into_owned())
}

/// Apply the legacy-slug → header-name reconciliation. `workspace_root`
/// hosts both the source `.forage` files (driving the lookup table)
/// and the daemon's `.forage/` directory whose files and dirs get
/// renamed in place.
///
/// The migration short-circuits when the workspace has no `.forage`
/// source files: there's nothing to translate against, so any
/// preexisting daemon state must already be in the new shape (the
/// only way to land here is a workspace that was freshly initialized
/// after Phase 4) or genuinely orphaned (no recipe on disk that any
/// existing row could refer to).
pub(crate) fn migrate_legacy_keying(
    conn: &Connection,
    workspace_root: &Path,
) -> Result<(), DaemonError> {
    let lookup = match build_slug_to_name(workspace_root) {
        Some(map) if !map.is_empty() => map,
        // No workspace (no `forage.toml`) or no parseable recipes —
        // nothing to reconcile against.
        _ => {
            tracing::info!(
                workspace = %workspace_root.display(),
                "legacy keying migration: no workspace recipes to reconcile against",
            );
            return Ok(());
        }
    };

    let daemon_dir = workspace_root.join(".forage");
    let deployments_dir = daemon_dir.join("deployments");
    let data_dir = daemon_dir.join("data");

    rewrite_runs(conn, &lookup, &data_dir)?;
    rewrite_deployed_versions(conn, &lookup)?;
    rename_deployment_directories(&lookup, &deployments_dir)?;
    rename_output_stores(&lookup, &data_dir)?;
    Ok(())
}

/// Map from path-derived legacy slug to the recipe's header name. Only
/// entries where the slug differs from the header name are kept —
/// recipes whose slug already equals their header name don't need any
/// renaming work, and folding them into the map would mask legitimate
/// "row points at a missing recipe" cases as no-op hits.
fn build_slug_to_name(workspace_root: &Path) -> Option<HashMap<String, String>> {
    let ws = discover(workspace_root)?;
    let mut out: HashMap<String, String> = HashMap::new();
    for recipe in ws.recipes() {
        let Some(slug) = slug_from_path(&ws.root, recipe.path) else {
            continue;
        };
        let name = recipe.name().to_string();
        if slug == name {
            continue;
        }
        out.insert(slug, name);
    }
    Some(out)
}

fn rewrite_runs(
    conn: &Connection,
    lookup: &HashMap<String, String>,
    data_dir: &Path,
) -> Result<(), DaemonError> {
    let mut stmt =
        conn.prepare("SELECT id, recipe_name, output_path FROM runs")?;
    let rows: Vec<(String, String, String)> = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<_>>()?;
    drop(stmt);

    for (id, current_name, output_path) in rows {
        let Some(new_name) = lookup.get(&current_name) else {
            continue;
        };
        // Redirect the default-shaped output path
        // (`<workspace>/.forage/data/<old-slug>.sqlite`) to the
        // header-name file; a Run with a custom output path the user
        // chose stays as-is. The file rename itself happens in
        // `rename_output_stores` so multiple rows pointing at the
        // same SQLite file all converge on one rename call.
        let default_old = data_dir.join(format!("{current_name}.sqlite"));
        let output_path_buf = PathBuf::from(&output_path);
        let new_output = if output_path_buf == default_old {
            data_dir.join(format!("{new_name}.sqlite"))
        } else {
            output_path_buf
        };

        tracing::info!(
            run_id = %id,
            from = %current_name,
            to = %new_name,
            "migrate runs row: recipe_name rewrite",
        );
        conn.execute(
            "UPDATE runs SET recipe_name = ?1, output_path = ?2 WHERE id = ?3",
            params![new_name, new_output.to_string_lossy(), id],
        )?;
    }
    Ok(())
}

fn rewrite_deployed_versions(
    conn: &Connection,
    lookup: &HashMap<String, String>,
) -> Result<(), DaemonError> {
    let mut stmt = conn.prepare("SELECT DISTINCT recipe_name FROM deployed_versions")?;
    let names: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<_>>()?;
    drop(stmt);

    for current_name in names {
        let Some(new_name) = lookup.get(&current_name) else {
            continue;
        };
        tracing::info!(
            from = %current_name,
            to = %new_name,
            "migrate deployed_versions rows: recipe_name rewrite",
        );
        conn.execute(
            "UPDATE deployed_versions SET recipe_name = ?1 WHERE recipe_name = ?2",
            params![new_name, current_name],
        )?;
    }
    Ok(())
}

fn rename_deployment_directories(
    lookup: &HashMap<String, String>,
    deployments_dir: &Path,
) -> Result<(), DaemonError> {
    if !deployments_dir.is_dir() {
        return Ok(());
    }
    for (old_slug, new_name) in lookup {
        let from = deployments_dir.join(old_slug);
        let to = deployments_dir.join(new_name);
        if !from.is_dir() {
            continue;
        }
        if to.exists() {
            // The destination already exists — leave both in place and
            // shout. We have two pieces of state and no obvious rule
            // for which to keep; deleting either could lose work.
            tracing::warn!(
                from = %from.display(),
                to = %to.display(),
                "skipping deployments dir rename: destination already exists",
            );
            continue;
        }
        tracing::info!(
            from = %from.display(),
            to = %to.display(),
            "migrate deployments dir: rename",
        );
        fs::rename(&from, &to)?;
    }
    Ok(())
}

fn rename_output_stores(
    lookup: &HashMap<String, String>,
    data_dir: &Path,
) -> Result<(), DaemonError> {
    if !data_dir.is_dir() {
        return Ok(());
    }
    for (old_slug, new_name) in lookup {
        let from = data_dir.join(format!("{old_slug}.sqlite"));
        let to = data_dir.join(format!("{new_name}.sqlite"));
        if !from.is_file() {
            continue;
        }
        if to.exists() {
            tracing::warn!(
                from = %from.display(),
                to = %to.display(),
                "skipping output store rename: destination already exists",
            );
            continue;
        }
        tracing::info!(
            from = %from.display(),
            to = %to.display(),
            "migrate output store: rename",
        );
        fs::rename(&from, &to)?;
    }
    Ok(())
}
