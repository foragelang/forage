//! `forage run` against a composition recipe.
//!
//! Before the linked-runtime refactor, the CLI's `run` only handled
//! scraping bodies — composition recipes silently produced zero
//! records because no composition path existed in `forage_http::Engine`.
//! After the refactor, `forage_core::run_recipe` dispatches on body
//! kind and walks the chain; the CLI uses the same entry point as
//! the daemon, so a composition recipe runs end-to-end.

use assert_cmd::Command;
use predicates::str::contains;
use std::fs;

const MANIFEST: &str = "description = \"\"\ncategory = \"\"\ntags = []\n";

const PRODUCT_DECLS: &str = "share type Product { id: String }\n";

const UPSTREAM: &str = r#"recipe "scrape"
engine http

emits Product

step list {
    method "GET"
    url    "https://api.example.com/items"
}

for $i in $list[*] {
    emit Product {
        id ← $i.id
    }
}
"#;

const ENRICH: &str = r#"recipe "enrich"
engine http

input prior: [Product]

emits Product

for $p in $input.prior {
    emit Product {
        id ← $p.id
    }
}
"#;

const COMPOSED: &str = r#"recipe "pipeline"
engine http

compose "scrape" | "enrich"
"#;

fn workspace() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path();
    fs::write(ws.join("forage.toml"), MANIFEST).unwrap();
    fs::write(ws.join("Product.forage"), PRODUCT_DECLS).unwrap();
    fs::write(ws.join("scrape.forage"), UPSTREAM).unwrap();
    fs::write(ws.join("enrich.forage"), ENRICH).unwrap();
    fs::write(ws.join("pipeline.forage"), COMPOSED).unwrap();
    tmp
}

/// Plant a captures file for the scraping stage. The composition
/// runtime threads the same captures into every stage's HTTP driver,
/// so only the upstream stage hits the network; the downstream pass-
/// through consumes the upstream record stream.
fn plant_fixtures(workspace: &std::path::Path) {
    let exchange = serde_json::json!({
        "kind": "http",
        "url": "https://api.example.com/items",
        "method": "GET",
        "request_headers": {},
        "request_body": null,
        "status": 200,
        "response_headers": {},
        "body": r#"[{"id":"a"},{"id":"b"},{"id":"c"}]"#,
    });
    let path = workspace.join("_fixtures").join("pipeline.jsonl");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        path,
        format!("{}\n", serde_json::to_string(&exchange).unwrap()),
    )
    .unwrap();
}

#[test]
fn forage_run_executes_composition_recipe_end_to_end() {
    let tmp = workspace();
    let ws = tmp.path();
    plant_fixtures(ws);

    // Run the composition recipe by its header name.  Each stage's
    // emitted records thread into the next stage; the snapshot
    // surfaces the final stage's emissions.
    Command::cargo_bin("forage")
        .unwrap()
        .current_dir(ws)
        .args(["run", "pipeline", "--replay", "--format", "json"])
        .assert()
        .success()
        // Three records flow through the pipeline: upstream emits a/b/c,
        // enrich re-emits each one.
        .stdout(contains(r#""typeName": "Product""#).count(3));
}
