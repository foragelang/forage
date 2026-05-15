//! CLI JSON-LD output: `forage run --format jsonld` writes a JSON-LD
//! document driven by the recipe's type alignments, and
//! `forage test --format jsonld` round-trips against a `.jsonld`
//! golden in `_snapshots/`.

use assert_cmd::Command;
use serde_json::Value;
use std::fs;

const MANIFEST: &str = "description = \"\"\ncategory = \"\"\ntags = []\n";

/// Two emit types: one with type-level + field-level alignments, one
/// without — so the JSON-LD output exercises both the aligned and
/// ride-through arms of the writer in one run.
const RECIPE: &str = r#"recipe "items"
engine http
emits Product | Note

type Product
    aligns schema.org/Product
{
    name: String aligns schema.org/name
    sku:  String aligns schema.org/gtin
}

type Note {
    label: String
}

step list {
    method "GET"
    url    "https://api.example.com/items"
}

for $i in $list[*] {
    emit Product {
        name ← $i.name
        sku  ← $i.sku
    }
    emit Note {
        label ← $i.note
    }
}
"#;

fn plant_fixtures(workspace: &std::path::Path) {
    let body = r#"[{"name":"Widget","sku":"W-1","note":"first"}]"#;
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
    fs::write(path, format!("{}\n", serde_json::to_string(&exchange).unwrap())).unwrap();
}

fn workspace() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path();
    fs::write(ws.join("forage.toml"), MANIFEST).unwrap();
    fs::write(ws.join("items.forage"), RECIPE).unwrap();
    plant_fixtures(ws);
    tmp
}

#[test]
fn run_output_jsonld_writes_context_and_graph() {
    let tmp = workspace();
    let ws = tmp.path();
    let assertion = Command::cargo_bin("forage")
        .unwrap()
        .current_dir(ws)
        .args(["run", "items", "--replay", "--format", "jsonld"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assertion.get_output().stdout).to_string();
    let doc: Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("CLI did not produce a valid JSON-LD document: {e}\n{stdout}"));

    // `Product` is aligned; its context entry resolves the type to
    // schema.org/Product and the `name` field to schema.org/name.
    let product_ctx = doc
        .pointer("/@context/Product")
        .expect("Product entry under @context");
    assert_eq!(
        product_ctx.pointer("/@id").and_then(Value::as_str),
        Some("https://schema.org/Product"),
    );
    assert_eq!(
        product_ctx
            .pointer("/@context/name")
            .and_then(Value::as_str),
        Some("https://schema.org/name"),
    );

    // `Note` carries no alignment — it must not appear in @context.
    assert!(doc.pointer("/@context/Note").is_none());

    // `@graph` carries both records with bare-name @type.
    let graph = doc
        .pointer("/@graph")
        .and_then(Value::as_array)
        .expect("@graph array");
    let types: Vec<&str> = graph
        .iter()
        .filter_map(|r| r.pointer("/@type").and_then(Value::as_str))
        .collect();
    assert!(types.contains(&"Product"), "got types: {types:?}");
    assert!(types.contains(&"Note"), "got types: {types:?}");
}

#[test]
fn test_format_jsonld_writes_and_reads_jsonld_snapshot() {
    let tmp = workspace();
    let ws = tmp.path();

    // First invocation: no snapshot on disk, so `forage test` writes
    // one at `_snapshots/items.jsonld`.
    Command::cargo_bin("forage")
        .unwrap()
        .current_dir(ws)
        .args(["test", "items", "--format", "jsonld"])
        .assert()
        .success();
    let snap_path = ws.join("_snapshots").join("items.jsonld");
    let written = fs::read_to_string(&snap_path).expect("snapshot file exists after first test");
    let doc: Value = serde_json::from_str(&written).expect("snapshot is valid JSON");
    assert!(doc.pointer("/@context/Product").is_some());
    assert!(doc.pointer("/@graph").is_some());

    // Second invocation: golden is on disk; the test must pass without
    // rewriting the file (timestamps drift across runs but the JSON-LD
    // shape doesn't).
    Command::cargo_bin("forage")
        .unwrap()
        .current_dir(ws)
        .args(["test", "items", "--format", "jsonld"])
        .assert()
        .success();
}
