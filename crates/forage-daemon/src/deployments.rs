//! Filesystem layout for deployed recipe versions.
//!
//! Each deployed `(slug, version)` lives at
//! `<daemon_dir>/deployments/<slug>/v<n>/` with two files:
//!
//! - `recipe.forage`: the immutable source text the scheduler executes.
//! - `catalog.json`: the `SerializableCatalog` resolved at deploy time.
//!
//! Atomic writes: the deploy path materializes a temp directory first,
//! then `fs::rename`s it onto the final path. Half-written deploys are
//! impossible — readers either see the full pair or no directory at
//! all.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use forage_core::SerializableCatalog;

use crate::error::DaemonError;

const RECIPE_FILE: &str = "recipe.forage";
const CATALOG_FILE: &str = "catalog.json";

/// Final on-disk path for one deployed version.
pub(crate) fn deployment_dir(deployments_root: &Path, slug: &str, version: u32) -> PathBuf {
    deployments_root.join(slug).join(format!("v{version}"))
}

/// Materialize a version directory atomically. Writes the source +
/// catalog to a temp dir, then renames into place. Caller is
/// responsible for ensuring `version` hasn't been used yet (the DB
/// gates this via `PRIMARY KEY (slug, version)`).
pub(crate) fn write_atomic(
    deployments_root: &Path,
    slug: &str,
    version: u32,
    source: &str,
    catalog: &SerializableCatalog,
) -> io::Result<()> {
    let slug_dir = deployments_root.join(slug);
    fs::create_dir_all(&slug_dir)?;

    // Temp dir lives as a sibling of the final dir so the rename is
    // same-filesystem (and therefore atomic). The ULID tail prevents
    // collisions across racing deploys.
    let tmp_name = format!(".tmp-v{version}-{}", ulid::Ulid::new());
    let tmp_dir = slug_dir.join(&tmp_name);
    fs::create_dir_all(&tmp_dir)?;

    // Best-effort cleanup if anything below fails — leaving a stray
    // `.tmp-…` around is harmless but noisy.
    let cleanup = |path: &Path| {
        if let Err(e) = fs::remove_dir_all(path) {
            tracing::warn!(path = %path.display(), error = %e, "failed to remove deploy tempdir");
        }
    };

    let recipe_path = tmp_dir.join(RECIPE_FILE);
    if let Err(e) = fs::write(&recipe_path, source) {
        cleanup(&tmp_dir);
        return Err(e);
    }
    let catalog_path = tmp_dir.join(CATALOG_FILE);
    let catalog_body = match serde_json::to_vec_pretty(catalog) {
        Ok(b) => b,
        Err(e) => {
            cleanup(&tmp_dir);
            return Err(io::Error::new(io::ErrorKind::InvalidData, e));
        }
    };
    if let Err(e) = fs::write(&catalog_path, catalog_body) {
        cleanup(&tmp_dir);
        return Err(e);
    }

    let final_dir = deployment_dir(deployments_root, slug, version);
    if let Err(e) = fs::rename(&tmp_dir, &final_dir) {
        cleanup(&tmp_dir);
        return Err(e);
    }
    Ok(())
}

/// Read back a deployed version's source + catalog. `UnknownDeployment`
/// surfaces when the version directory is absent (e.g. stale Run
/// pointer after a wipe).
pub(crate) fn read_deployed(
    deployments_root: &Path,
    slug: &str,
    version: u32,
) -> Result<(String, SerializableCatalog), DaemonError> {
    let dir = deployment_dir(deployments_root, slug, version);
    if !dir.is_dir() {
        return Err(DaemonError::UnknownDeployment {
            slug: slug.to_string(),
            version,
        });
    }
    let source = fs::read_to_string(dir.join(RECIPE_FILE))?;
    let catalog_raw = fs::read_to_string(dir.join(CATALOG_FILE))?;
    let catalog: SerializableCatalog = serde_json::from_str(&catalog_raw)?;
    Ok((source, catalog))
}
