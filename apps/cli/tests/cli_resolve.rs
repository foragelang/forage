//! Argument resolution for the recipe-name-keyed CLI.
//!
//! The CLI accepts a recipe HEADER NAME as the primary unit and falls
//! back to interpreting the argument as a `.forage` file path. These
//! tests pin both branches and the error case where neither resolves.

use assert_cmd::Command;
use std::fs;

const MANIFEST: &str = "description = \"test\"\ncategory = \"scrape\"\ntags = []\n";

/// A minimal no-op recipe: parses, validates, runs to an empty
/// snapshot. Used by the resolve tests so we can exercise the
/// resolver without touching the network or fixtures.
fn empty_recipe(name: &str) -> String {
    format!("recipe \"{name}\"\nengine http\n")
}

/// `forage run <recipe-name>` resolves the header name through the
/// workspace even when the FILE BASENAME differs. The file is
/// `scrape-amazon.forage`; the header inside is `amazon-products`;
/// running `forage run amazon-products` must hit that file.
#[test]
fn run_resolves_recipe_by_header_name_not_file_basename() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path();
    fs::write(ws.join("forage.toml"), MANIFEST).unwrap();
    fs::write(
        ws.join("scrape-amazon.forage"),
        empty_recipe("amazon-products"),
    )
    .unwrap();

    Command::cargo_bin("forage")
        .unwrap()
        .current_dir(ws)
        .arg("run")
        .arg("amazon-products")
        .assert()
        .success();
}

/// The path-alias fallback: pointing at the `.forage` file directly
/// must run the recipe inside, regardless of its header name.
#[test]
fn run_resolves_recipe_by_path_alias() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path();
    fs::write(ws.join("forage.toml"), MANIFEST).unwrap();
    let file = ws.join("scrape-amazon.forage");
    fs::write(&file, empty_recipe("amazon-products")).unwrap();

    Command::cargo_bin("forage")
        .unwrap()
        .current_dir(ws)
        .arg("run")
        .arg(&file)
        .assert()
        .success();
}

/// An unresolved argument (no recipe with that name AND no file at
/// that path) must error clearly. The diagnostic mentions both
/// branches so the user can tell whether they typoed the name or
/// the path.
#[test]
fn run_errors_when_neither_name_nor_path_resolves() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path();
    fs::write(ws.join("forage.toml"), MANIFEST).unwrap();

    Command::cargo_bin("forage")
        .unwrap()
        .current_dir(ws)
        .arg("run")
        .arg("does-not-exist")
        .assert()
        .failure()
        .stderr(predicates::str::contains("does-not-exist"));
}

/// Resolution outside a workspace falls straight to the path alias.
/// A bare name like `amazon-products` has no workspace to look in
/// and no file at that path; the error must say so.
#[test]
fn run_errors_when_no_workspace_and_arg_is_not_a_path() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path();
    // No forage.toml — lonely-recipe mode.

    Command::cargo_bin("forage")
        .unwrap()
        .current_dir(ws)
        .arg("run")
        .arg("amazon-products")
        .assert()
        .failure()
        .stderr(predicates::str::contains("workspace"));
}

/// `forage new <recipe-name>` scaffolds `<workspace>/<recipe-name>.forage`
/// at the workspace ROOT (not in a slug folder), with the body
/// `recipe "<recipe-name>" engine http` and a trailing blank line so
/// the file is ready to extend.
#[test]
fn new_scaffolds_flat_workspace_root_file() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path();
    fs::write(ws.join("forage.toml"), MANIFEST).unwrap();

    Command::cargo_bin("forage")
        .unwrap()
        .current_dir(ws)
        .arg("new")
        .arg("amazon-products")
        .assert()
        .success();

    let scaffolded = ws.join("amazon-products.forage");
    assert!(scaffolded.is_file(), "scaffolded file must exist");
    let body = fs::read_to_string(&scaffolded).unwrap();
    assert!(
        body.starts_with("recipe \"amazon-products\" engine http"),
        "scaffolded body lacks the right header: {body:?}"
    );
    // The recipe should ALSO resolve via `recipe_by_name` immediately
    // — the workspace's scanner finds files at root-level.
    Command::cargo_bin("forage")
        .unwrap()
        .current_dir(ws)
        .arg("run")
        .arg("amazon-products")
        .assert()
        .success();
}

/// `forage new` against a workspace already holding a `.forage` file
/// at the target path must refuse rather than clobbering local work.
#[test]
fn new_refuses_to_overwrite_existing_file() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path();
    fs::write(ws.join("forage.toml"), MANIFEST).unwrap();
    fs::write(ws.join("amazon-products.forage"), "// already mine\n").unwrap();

    Command::cargo_bin("forage")
        .unwrap()
        .current_dir(ws)
        .arg("new")
        .arg("amazon-products")
        .assert()
        .failure()
        .stderr(predicates::str::contains("already exists"));
}

/// `forage new --engine browser amazon-products` produces a recipe
/// header with the browser engine kind.
#[test]
fn new_supports_browser_engine() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path();
    fs::write(ws.join("forage.toml"), MANIFEST).unwrap();

    Command::cargo_bin("forage")
        .unwrap()
        .current_dir(ws)
        .arg("new")
        .arg("amazon-products")
        .arg("--engine")
        .arg("browser")
        .assert()
        .success();

    let body = fs::read_to_string(ws.join("amazon-products.forage")).unwrap();
    assert!(
        body.contains("engine browser"),
        "browser-engine scaffold missing header tag: {body:?}",
    );
}

/// `forage new` outside any workspace must complain about the missing
/// `forage.toml` rather than scaffolding into cwd silently.
#[test]
fn new_requires_a_workspace() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    // No forage.toml anywhere up the tree.

    Command::cargo_bin("forage")
        .unwrap()
        .current_dir(dir)
        .arg("new")
        .arg("solo")
        .assert()
        .failure()
        .stderr(predicates::str::contains("workspace"));
}

/// `forage test --update` writes the produced snapshot to the
/// recipe-name-keyed `_snapshots/<recipe>.json` under the workspace
/// data directory — not next to the recipe source file.
#[test]
fn test_update_writes_snapshot_under_underscore_snapshots() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path();
    fs::write(ws.join("forage.toml"), MANIFEST).unwrap();
    fs::write(ws.join("solo.forage"), empty_recipe("solo")).unwrap();

    Command::cargo_bin("forage")
        .unwrap()
        .current_dir(ws)
        .arg("test")
        .arg("solo")
        .arg("--update")
        .assert()
        .success();

    let snap = ws.join("_snapshots").join("solo.json");
    assert!(snap.is_file(), "snapshot landed at unexpected path");
}
