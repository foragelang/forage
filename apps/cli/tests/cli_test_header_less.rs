//! `forage test` pointed at a header-less `.forage` file must surface
//! a clean error rather than crash the engine. The HTTP engine
//! expects a recipe header; the CLI is the boundary that rejects the
//! empty case before any transport spins up.

use assert_cmd::Command;
use predicates::prelude::*;
use predicates::str::contains;
use std::fs;

#[test]
fn forage_test_rejects_header_less_file() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("declarations.forage");
    // A pure declarations file — no `recipe "..."` header, so the
    // engine has nothing to run.
    fs::write(&file, "type Item { id: String }\n").unwrap();

    let mut cmd = Command::cargo_bin("forage").unwrap();
    cmd.arg("test")
        .arg(&file)
        .assert()
        .failure()
        .stderr(contains("recipe").and(contains("header")));
}
