//! Parity test for the `run_replay` export. Exercises `run_replay_inner`
//! — the pure-Rust core the wasm-bindgen wrapper delegates to — so the
//! test runs on the native target with a real tokio runtime. The wasm
//! wrapper just translates JS values into the same shape.

use indexmap::IndexMap;
use serde_json::json;

use forage_core::EvalValue;
use forage_wasm::{DeclFile, ReplayError, run_replay_inner};

fn capture_line(url: &str, body: &str) -> String {
    json!({
        "kind": "http",
        "url": url,
        "method": "GET",
        "request_headers": {},
        "request_body": null,
        "status": 200,
        "response_headers": {},
        "body": body,
    })
    .to_string()
}

#[tokio::test]
async fn run_replay_inner_runs_a_simple_recipe() {
    let source = r#"
        recipe "smoke"
        engine http
        type Item { id: String }
        step list {
            method "GET"
            url "https://api.example.com/items"
        }
        for $i in $list.items[*] {
            emit Item { id ← $i.id }
        }
    "#;
    let jsonl = capture_line(
        "https://api.example.com/items",
        r#"{"items":[{"id":"a"},{"id":"b"}]}"#,
    );

    let snapshot = run_replay_inner(source, &[], &jsonl, IndexMap::new(), IndexMap::new())
        .await
        .expect("replay run succeeds");
    assert_eq!(snapshot.records.len(), 2);
    assert_eq!(snapshot.records[0].type_name, "Item");
}

#[tokio::test]
async fn run_replay_inner_merges_decls_into_catalog() {
    // The recipe references a type that lives only in the shared decls
    // file. Without `merge_decls` the validator would reject; with it,
    // the run succeeds and emits a record of that type.
    let decls = vec![DeclFile {
        name: "shared.forage".into(),
        source: "type Item { id: String }".into(),
    }];
    let source = r#"
        recipe "shared"
        engine http
        step list {
            method "GET"
            url "https://api.example.com/items"
        }
        for $i in $list.items[*] {
            emit Item { id ← $i.id }
        }
    "#;
    let jsonl = capture_line(
        "https://api.example.com/items",
        r#"{"items":[{"id":"x"}]}"#,
    );
    let snapshot = run_replay_inner(source, &decls, &jsonl, IndexMap::new(), IndexMap::new())
        .await
        .expect("replay run with shared decls succeeds");
    assert_eq!(snapshot.records.len(), 1);
    assert_eq!(snapshot.records[0].type_name, "Item");
}

#[tokio::test]
async fn run_replay_inner_threads_inputs_through_the_engine() {
    // Recipe takes a `term` input and substitutes it into the URL.
    // The fixture's URL must match the substituted shape for the
    // ReplayTransport to find it.
    let source = r#"
        recipe "input-substitution"
        engine http
        input term: String
        type Item { id: String }
        step search {
            method "GET"
            url "https://api.example.com/search?q={$input.term}"
        }
        for $i in $search.items[*] {
            emit Item { id ← $i.id }
        }
    "#;
    let jsonl = capture_line(
        "https://api.example.com/search?q=OT22",
        r#"{"items":[{"id":"a"}]}"#,
    );
    let mut inputs: IndexMap<String, EvalValue> = IndexMap::new();
    inputs.insert("term".into(), EvalValue::String("OT22".into()));

    let snapshot = run_replay_inner(source, &[], &jsonl, inputs, IndexMap::new())
        .await
        .expect("replay run with input succeeds");
    assert_eq!(snapshot.records.len(), 1);
}

#[tokio::test]
async fn run_replay_inner_rejects_a_recipe_in_decls_slot() {
    // The decls list is meant for header-less declarations files only.
    // Passing a full recipe in the decls slot is a caller bug; surface
    // it as a structured error rather than silently merging anything.
    let decls = vec![DeclFile {
        name: "rogue.forage".into(),
        source: "recipe \"r\" engine http".into(),
    }];
    let source = r#"
        recipe "smoke"
        engine http
        type Item { id: String }
        step list {
            method "GET"
            url "https://api.example.com/items"
        }
        emit Item { id ← "x" }
    "#;
    let jsonl = capture_line(
        "https://api.example.com/items",
        r#"{"items":[]}"#,
    );
    let err = run_replay_inner(source, &decls, &jsonl, IndexMap::new(), IndexMap::new())
        .await
        .expect_err("recipe in decls slot rejected");
    assert!(matches!(err, ReplayError::NotADeclFile { .. }));
}

#[tokio::test]
async fn run_replay_inner_surfaces_validation_errors() {
    // Recipe references an undeclared type. The pre-run validator
    // catches it; the engine never starts. The error variant carries
    // the per-issue messages so the hub IDE can surface them.
    let source = r#"
        recipe "missing-type"
        engine http
        step list {
            method "GET"
            url "https://api.example.com/items"
        }
        emit DoesNotExist { id ← "x" }
    "#;
    let jsonl = capture_line(
        "https://api.example.com/items",
        r#"{"items":[]}"#,
    );
    let err = run_replay_inner(source, &[], &jsonl, IndexMap::new(), IndexMap::new())
        .await
        .expect_err("validation should fail");
    assert!(matches!(err, ReplayError::Validation(_)));
}
