//! Per-recipe data directories keyed by recipe header name.
//!
//! Fixtures (captures replayed at run time) live at
//! `<workspace>/_fixtures/<recipe>.jsonl` — one JSON capture per line.
//! Snapshots (the canonical run output a publish ships alongside its
//! recipe) live at `<workspace>/_snapshots/<recipe>.json`.
//!
//! Path resolution flows through these helpers so every caller agrees
//! on the layout; string-building the paths inline at consumer sites
//! is how shapes drift.

use std::path::{Path, PathBuf};

/// The directory under `<workspace>` that holds replay captures, one
/// file per recipe. The source scan in `workspace::load` skips this
/// directory.
pub const FIXTURES_DIR: &str = "_fixtures";

/// The directory under `<workspace>` that holds published-run
/// snapshots, one file per recipe. The source scan in `workspace::load`
/// skips this directory.
pub const SNAPSHOTS_DIR: &str = "_snapshots";

/// `<workspace>/_fixtures/<recipe_name>.jsonl`. The replay transport
/// reads from this path; `forage record` and hub-sync write to it.
pub fn fixtures_path(workspace_root: &Path, recipe_name: &str) -> PathBuf {
    workspace_root
        .join(FIXTURES_DIR)
        .join(format!("{recipe_name}.jsonl"))
}

/// `<workspace>/_snapshots/<recipe_name>.json`. The hub publish/sync
/// round-trip reads/writes the canonical-run snapshot here.
pub fn snapshot_path(workspace_root: &Path, recipe_name: &str) -> PathBuf {
    workspace_root
        .join(SNAPSHOTS_DIR)
        .join(format!("{recipe_name}.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixtures_path_lives_under_underscore_fixtures() {
        let root = Path::new("/ws");
        assert_eq!(
            fixtures_path(root, "remedy-baltimore"),
            PathBuf::from("/ws/_fixtures/remedy-baltimore.jsonl"),
        );
    }

    #[test]
    fn snapshot_path_lives_under_underscore_snapshots() {
        let root = Path::new("/ws");
        assert_eq!(
            snapshot_path(root, "trilogy-med"),
            PathBuf::from("/ws/_snapshots/trilogy-med.json"),
        );
    }

    /// The data-dir constants must match the skip-list in the workspace
    /// scanner. A drift here would silently start picking the recipe-
    /// keyed JSONL captures up as source files.
    #[test]
    fn data_dir_names_match_workspace_skip_list() {
        assert_eq!(FIXTURES_DIR, "_fixtures");
        assert_eq!(SNAPSHOTS_DIR, "_snapshots");
    }
}
