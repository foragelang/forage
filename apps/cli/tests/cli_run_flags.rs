//! CLI flag wiring: `--sample`, `--replay`, `--replay-from`, and
//! `--mode dev|prod`. The tests cover the four cases on a single
//! recipe + a small captures file so the visible record count
//! changes with the flag values.

use assert_cmd::Command;
use predicates::str::contains;
use std::fs;

const MANIFEST: &str = "description = \"\"\ncategory = \"\"\ntags = []\n";

const RECIPE: &str = r#"recipe "items"
engine http

type Item {
    id: String
}

step list {
    method "GET"
    url    "https://api.example.com/items"
}

for $i in $list[*] {
    emit Item {
        id ← $i.id
    }
}
"#;

/// Build a captures JSONL holding `count` records under
/// `_fixtures/items.jsonl` so `--replay` against the workspace
/// resolves to it.
fn plant_fixtures(workspace: &std::path::Path, count: usize) {
    let mut body = String::from("[");
    for i in 0..count {
        if i > 0 {
            body.push(',');
        }
        body.push_str(&format!(r#"{{"id":"r-{i}"}}"#));
    }
    body.push(']');
    let exchange = serde_json::json!({
        "kind": "http",
        "url": "https://api.example.com/items",
        "method": "GET",
        "request_headers": {},
        "request_body": null,
        "status": 200,
        "response_headers": {},
        "body": body,
    });
    let path = workspace.join("_fixtures").join("items.jsonl");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        path,
        format!("{}\n", serde_json::to_string(&exchange).unwrap()),
    )
    .unwrap();
}

fn workspace() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path();
    fs::write(ws.join("forage.toml"), MANIFEST).unwrap();
    fs::write(ws.join("items.forage"), RECIPE).unwrap();
    tmp
}

#[test]
fn replay_with_sample_caps_record_count() {
    let tmp = workspace();
    let ws = tmp.path();
    plant_fixtures(ws, 100);

    Command::cargo_bin("forage")
        .unwrap()
        .current_dir(ws)
        .args(["run", "items", "--replay", "--sample", "5", "--format", "json"])
        .assert()
        .success()
        // Five Item records — the snapshot's `records` array has
        // exactly five entries with `type_name: "Item"`.
        .stdout(contains(r#""typeName": "Item""#).count(5));
}

#[test]
fn mode_dev_expands_to_sample_and_replay() {
    let tmp = workspace();
    let ws = tmp.path();
    plant_fixtures(ws, 100);

    // `--mode dev` is sugar for `--sample 10 --replay`. The captures
    // hold 100 records; the dev preset clamps to 10.
    Command::cargo_bin("forage")
        .unwrap()
        .current_dir(ws)
        .args(["run", "items", "--mode", "dev", "--format", "json"])
        .assert()
        .success()
        .stdout(contains(r#""typeName": "Item""#).count(10));
}

#[test]
fn replay_from_overrides_default_fixture_path() {
    let tmp = workspace();
    let ws = tmp.path();
    // Default fixtures path is empty — only `--replay-from` carries
    // the captures we expect to emit against.
    let captures = ws.join("custom-captures.jsonl");
    let exchange = serde_json::json!({
        "kind": "http",
        "url": "https://api.example.com/items",
        "method": "GET",
        "status": 200,
        "body": r#"[{"id":"a"},{"id":"b"},{"id":"c"}]"#,
    });
    fs::write(
        &captures,
        format!("{}\n", serde_json::to_string(&exchange).unwrap()),
    )
    .unwrap();

    Command::cargo_bin("forage")
        .unwrap()
        .current_dir(ws)
        .args([
            "run",
            "items",
            "--replay-from",
            captures.to_str().unwrap(),
            "--format",
            "json",
        ])
        .assert()
        .success()
        .stdout(contains(r#""typeName": "Item""#).count(3));
}

#[test]
fn mode_dev_explicit_sample_overrides_preset_default() {
    let tmp = workspace();
    let ws = tmp.path();
    plant_fixtures(ws, 100);

    // The dev preset bundles `--sample 10`, but an explicit
    // `--sample 25` on the command line should win.
    Command::cargo_bin("forage")
        .unwrap()
        .current_dir(ws)
        .args([
            "run", "items", "--mode", "dev", "--sample", "25", "--format", "json",
        ])
        .assert()
        .success()
        .stdout(contains(r#""typeName": "Item""#).count(25));
}

#[test]
fn mode_prod_is_explicit_no_flags() {
    let tmp = workspace();
    let ws = tmp.path();
    // No live network is reachable; `--mode prod` (no replay) should
    // surface the live transport error rather than silently
    // succeeding. The error message names the URL the recipe tried
    // to hit.
    let assertion = Command::cargo_bin("forage")
        .unwrap()
        .current_dir(ws)
        .args(["run", "items", "--mode", "prod", "--format", "json"])
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&assertion.get_output().stderr).to_string();
    assert!(
        stderr.contains("api.example.com")
            || stderr.contains("dns")
            || stderr.contains("DNS")
            || stderr.contains("connect"),
        "unexpected error output: {stderr}",
    );
}
