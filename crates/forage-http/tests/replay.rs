//! End-to-end: plant a `_fixtures/<recipe>.jsonl` in a workspace
//! tempdir, replay against it, and verify the emitted snapshot
//! matches the captured fixture. The path resolution flows through
//! `forage_core::workspace::fixtures_path` so every consumer of the
//! `_fixtures/` layout exercises the same code path.

use std::fs;

use forage_core::workspace::fixtures_path;
use forage_core::{RunOptions, TypeCatalog, parse};
use forage_http::Engine;
use forage_http::transport::ReplayTransport;
use forage_replay::{Capture, HttpExchange, read_jsonl, write_jsonl};
use indexmap::IndexMap;

const RECIPE_NAME: &str = "underscore-fixtures-replay";
const RECIPE_SOURCE: &str = r#"recipe "underscore-fixtures-replay"
engine http

type Item {
    id: String
    weight: Double
}

step list {
    method "GET"
    url    "https://example.test/items"
}

for $i in $list.items[*] {
    emit Item {
        id ← $i.id,
        weight ← $i.weight
    }
}
"#;

const RESPONSE_BODY: &str = r#"{"items":[{"id":"a","weight":1.5},{"id":"b","weight":2.0}]}"#;

/// Round-trip a JSONL fixture through the `_fixtures/` layout: the
/// writer puts the captures at `<root>/_fixtures/<recipe>.jsonl`, the
/// reader picks them back up via the same helper, and the replay
/// engine produces the snapshot the recipe declares.
#[tokio::test]
async fn replay_reads_underscore_fixtures_layout() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    // Plant a workspace marker so `discover` resolves this tempdir,
    // matching what the CLI / Studio see in production.
    fs::write(
        root.join("forage.toml"),
        "description = \"\"\ncategory = \"\"\ntags = []\n",
    )
    .unwrap();
    fs::write(root.join(format!("{RECIPE_NAME}.forage")), RECIPE_SOURCE).unwrap();

    let captures = vec![Capture::Http(HttpExchange {
        url: "https://example.test/items".into(),
        method: "GET".into(),
        request_headers: IndexMap::new(),
        request_body: None,
        status: 200,
        response_headers: IndexMap::new(),
        body: RESPONSE_BODY.into(),
    })];
    let path = fixtures_path(root, RECIPE_NAME);
    write_jsonl(&path, &captures).expect("write captures");
    assert!(path.exists(), "captures landed at {}", path.display());

    let recipe = parse(RECIPE_SOURCE).expect("parse recipe");
    let catalog = TypeCatalog::from_file(&recipe);
    let loaded = read_jsonl(&path).expect("read captures");
    assert_eq!(loaded.len(), 1);

    let transport = ReplayTransport::new(loaded);
    let engine = Engine::new(&transport);
    let snapshot = engine
        .run(
            &recipe,
            &catalog,
            IndexMap::new(),
            IndexMap::new(),
            &RunOptions::default(),
        )
        .await
        .expect("engine run");

    assert_eq!(snapshot.records.len(), 2);
    let ids: Vec<&str> = snapshot
        .records
        .iter()
        .filter_map(|r| match r.fields.get("id") {
            Some(forage_core::ast::JSONValue::String(s)) => Some(s.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(ids, vec!["a", "b"]);
}
