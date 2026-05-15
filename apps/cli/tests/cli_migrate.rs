//! End-to-end tests for `forage migrate`: a tempdir workspace in the
//! legacy `<slug>/recipe.forage` shape is fed through the CLI binary
//! and the resulting flat shape gets verified.

use assert_cmd::Command;
use std::fs;

const MANIFEST: &str = "description = \"test\"\ncategory = \"scrape\"\ntags = []\n";

/// Write a file, creating parent directories as needed.
fn write(path: &std::path::Path, body: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, body).unwrap();
}

/// Dry-run prints planned actions but leaves the workspace untouched.
/// Re-running with `--apply` materializes them.
#[test]
fn dry_run_changes_nothing_apply_materializes() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path();
    write(&ws.join("forage.toml"), MANIFEST);
    write(
        &ws.join("zen-leaf").join("recipe.forage"),
        "recipe \"zen-leaf\"\nengine http\n",
    );
    write(
        &ws.join("zen-leaf").join("fixtures").join("captures.jsonl"),
        "{\"kind\":\"http\",\"url\":\"https://x\",\"method\":\"GET\",\"status\":200,\"body\":\"{}\"}\n",
    );

    // Dry-run.
    Command::cargo_bin("forage")
        .unwrap()
        .arg("migrate")
        .arg(ws)
        .assert()
        .success()
        .stdout(predicates::str::contains("move recipe"));
    assert!(
        ws.join("zen-leaf").join("recipe.forage").is_file(),
        "dry-run must not have moved the recipe yet",
    );
    assert!(!ws.join("zen-leaf.forage").exists());

    // Apply.
    Command::cargo_bin("forage")
        .unwrap()
        .arg("migrate")
        .arg(ws)
        .arg("--apply")
        .assert()
        .success();
    assert!(ws.join("zen-leaf.forage").is_file());
    assert!(
        ws.join("_fixtures").join("zen-leaf.jsonl").is_file(),
        "fixtures must land under _fixtures/",
    );
    assert!(!ws.join("zen-leaf").exists(), "legacy dir must be gone");
}

/// A workspace already in the flat shape produces a "nothing to
/// migrate" dry-run and `--apply` is a no-op.
#[test]
fn no_op_on_flat_workspace() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path();
    write(&ws.join("forage.toml"), MANIFEST);
    write(
        &ws.join("flat.forage"),
        "recipe \"flat\"\nengine http\n",
    );
    Command::cargo_bin("forage")
        .unwrap()
        .arg("migrate")
        .arg(ws)
        .assert()
        .success()
        .stdout(predicates::str::contains("nothing to migrate"));
}

/// The migrated workspace passes `forage test --update` end-to-end
/// against an empty replay set — confirms the post-migration shape
/// is actually runnable, not just shaped right on disk.
#[test]
fn migrated_workspace_runs_through_forage_test() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path();
    write(&ws.join("forage.toml"), MANIFEST);
    write(
        &ws.join("noop").join("recipe.forage"),
        "recipe \"noop\"\nengine http\n",
    );
    Command::cargo_bin("forage")
        .unwrap()
        .arg("migrate")
        .arg(ws)
        .arg("--apply")
        .assert()
        .success();
    Command::cargo_bin("forage")
        .unwrap()
        .current_dir(ws)
        .arg("test")
        .arg("noop")
        .arg("--update")
        .assert()
        .success();
    assert!(ws.join("_snapshots").join("noop.json").is_file());
}
