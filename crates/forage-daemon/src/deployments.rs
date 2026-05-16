//! Filesystem layout for deployed recipe versions.
//!
//! Each deployed `(recipe_name, version)` lives at
//! `<daemon_dir>/deployments/<recipe_name>/v<n>/` with one file:
//!
//! - `module.json`: the serialized [`LinkedModule`] — root recipe,
//!   transitively-resolved composition stages, and the unified type
//!   catalog. The closure is self-contained: the runtime executes the
//!   deployment without consulting other deployed versions, the
//!   workspace, or the lockfile at run time.
//!
//! Atomic writes: the deploy path materializes a temp directory first,
//! then `fs::rename`s it onto the final path. Half-written deploys are
//! impossible — readers either see the full pair or no directory at
//! all.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use forage_core::LinkedModule;

use crate::error::DaemonError;

const MODULE_FILE: &str = "module.json";

/// Final on-disk path for one deployed version.
pub(crate) fn deployment_dir(deployments_root: &Path, recipe_name: &str, version: u32) -> PathBuf {
    deployments_root
        .join(recipe_name)
        .join(format!("v{version}"))
}

/// Highest `v<n>` directory present on disk for `recipe_name`,
/// regardless of whether a matching `deployed_versions` row exists.
/// The deploy path uses this to pick a version that bumps past stray
/// directories — a stray dir on disk with no DB row is the documented
/// failure mode when an FS write succeeded but the SQLite txn rolled
/// back. Without this scan the next deploy would pick the stray dir's
/// number and `fs::rename` would fail with `ENOTEMPTY`.
pub(crate) fn max_version_on_disk(
    deployments_root: &Path,
    recipe_name: &str,
) -> io::Result<Option<u32>> {
    let recipe_dir = deployments_root.join(recipe_name);
    let read = match fs::read_dir(&recipe_dir) {
        Ok(r) => r,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let mut highest: Option<u32> = None;
    for entry in read.flatten() {
        let name = entry.file_name();
        let s = name.to_string_lossy();
        // Skip `.tmp-…` work directories. Only finalized `v<n>` dirs
        // contribute to the max.
        let Some(rest) = s.strip_prefix('v') else {
            continue;
        };
        let Ok(n) = rest.parse::<u32>() else {
            continue;
        };
        highest = Some(match highest {
            Some(prev) if prev >= n => prev,
            _ => n,
        });
    }
    Ok(highest)
}

/// Materialize a version directory atomically. Writes the serialized
/// module to a temp dir, then renames into place. Caller is
/// responsible for ensuring `version` hasn't been used yet (the DB
/// gates this via `PRIMARY KEY (slug, version)` on `deployed_versions`).
pub(crate) fn write_atomic(
    deployments_root: &Path,
    recipe_name: &str,
    version: u32,
    module: &LinkedModule,
) -> io::Result<()> {
    let recipe_dir = deployments_root.join(recipe_name);
    fs::create_dir_all(&recipe_dir)?;

    // Temp dir lives as a sibling of the final dir so the rename is
    // same-filesystem (and therefore atomic). The ULID tail prevents
    // collisions across racing deploys.
    let tmp_name = format!(".tmp-v{version}-{}", ulid::Ulid::new());
    let tmp_dir = recipe_dir.join(&tmp_name);
    fs::create_dir_all(&tmp_dir)?;

    // Best-effort cleanup if anything below fails — leaving a stray
    // `.tmp-…` around is harmless but noisy.
    let cleanup = |path: &Path| {
        if let Err(e) = fs::remove_dir_all(path) {
            tracing::warn!(path = %path.display(), error = %e, "failed to remove deploy tempdir");
        }
    };

    let module_path = tmp_dir.join(MODULE_FILE);
    let module_body = match serde_json::to_vec_pretty(module) {
        Ok(b) => b,
        Err(e) => {
            cleanup(&tmp_dir);
            return Err(io::Error::new(io::ErrorKind::InvalidData, e));
        }
    };
    if let Err(e) = fs::write(&module_path, module_body) {
        cleanup(&tmp_dir);
        return Err(e);
    }

    let final_dir = deployment_dir(deployments_root, recipe_name, version);
    if let Err(e) = fs::rename(&tmp_dir, &final_dir) {
        cleanup(&tmp_dir);
        return Err(e);
    }
    Ok(())
}

/// Read back a deployed version's linked module. `UnknownDeployment`
/// surfaces when the version directory is absent (e.g. stale Run
/// pointer after a wipe). A pre-linked-runtime deployment (a directory
/// holding `recipe.forage` + `catalog.json` rather than `module.json`)
/// surfaces a directed `Corrupt` error rather than a bare ENOENT — the
/// shape changed in this release and the user needs to clear the
/// stale directory.
pub(crate) fn read_deployed(
    deployments_root: &Path,
    recipe_name: &str,
    version: u32,
) -> Result<LinkedModule, DaemonError> {
    let dir = deployment_dir(deployments_root, recipe_name, version);
    if !dir.is_dir() {
        return Err(DaemonError::UnknownDeployment {
            recipe_name: recipe_name.to_string(),
            version,
        });
    }
    let module_path = dir.join(MODULE_FILE);
    if !module_path.is_file() {
        return Err(DaemonError::Corrupt {
            detail: format!(
                "deployment {recipe_name} v{version} at {} is missing {MODULE_FILE} — \
                 this directory was written by a pre-linked-runtime daemon. \
                 Clear `.forage/deployments/` and redeploy (see RELEASING.md).",
                dir.display(),
            ),
        });
    }
    let raw = fs::read_to_string(module_path)?;
    let module: LinkedModule = serde_json::from_str(&raw)?;
    Ok(module)
}
