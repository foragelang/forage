//! `forage test` invoked against a directory whose `recipe.forage` is
//! header-less must surface a clean error, not panic the HTTP engine.
//! The engine expects a recipe header; the CLI is the boundary that
//! has to reject the empty case before the engine ever sees it.

use assert_cmd::Command;
use predicates::prelude::*;
use predicates::str::contains;
use std::fs;

#[test]
fn forage_test_rejects_header_less_file() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    // A pure declarations file — no `recipe "..."` header, so the
    // engine has nothing to run.
    fs::write(dir.join("recipe.forage"), "type Item { id: String }\n").unwrap();

    let mut cmd = Command::cargo_bin("forage").unwrap();
    cmd.arg("test")
        .arg(dir)
        .assert()
        .failure()
        .stderr(contains("recipe").and(contains("header")));
}
