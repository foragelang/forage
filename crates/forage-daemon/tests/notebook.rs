//! Notebook composition: `Daemon::run_composition` against a chain
//! of deployed recipes. Mirrors the editor's run flow but goes through
//! the synthetic in-memory composition path Studio's notebook surface
//! exposes.

use forage_daemon::{Daemon, RunFlags};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

mod common;
use common::{deploy_disk_recipe, init_workspace};

const UPSTREAM: &str = r#"recipe "scrape-products"
engine http

share type Product { id: String }

output Product

step list {
    method "GET"
    url    "MOCK_BASE_URL/items"
}

for $i in $list.items[*] {
    emit Product { id ← $i.id }
}
"#;

const ENRICHER: &str = r#"recipe "enrich-products"
engine http

share type Product { id: String }

input prior: [Product]

output Product

for $p in $input.prior {
    emit Product { id ← $p.id }
}
"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn notebook_runs_two_stage_chain_without_persisting_a_run() {
    let tmp = tempfile::tempdir().unwrap();
    let ws_root = tmp.path().to_path_buf();

    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/items"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [{"id": "a"}, {"id": "b"}, {"id": "c"}],
            })),
        )
        .mount(&mock)
        .await;

    let upstream_src = UPSTREAM.replace("MOCK_BASE_URL", &mock.uri());
    init_workspace(&ws_root, "scrape-products", &upstream_src);
    std::fs::write(ws_root.join("enrich-products.forage"), ENRICHER).unwrap();

    let daemon = Daemon::open(ws_root.clone()).expect("open daemon");
    deploy_disk_recipe(&daemon, &ws_root, "scrape-products");
    deploy_disk_recipe(&daemon, &ws_root, "enrich-products");

    let snapshot = daemon
        .run_composition(
            "notebook-preview",
            vec!["scrape-products".into(), "enrich-products".into()],
            indexmap::IndexMap::new(),
            RunFlags::prod(),
        )
        .await
        .expect("run_composition succeeds");

    // The chain emits one record per upstream item — the enricher
    // is a pass-through that re-emits each `Product` it sees.
    assert_eq!(snapshot.records.len(), 3);
    let ids: Vec<String> = snapshot
        .records
        .iter()
        .map(|r| match r.fields.get("id") {
            Some(forage_core::ast::JSONValue::String(s)) => s.clone(),
            other => panic!("expected String id, got {other:?}"),
        })
        .collect();
    assert_eq!(ids, vec!["a", "b", "c"]);

    // The notebook surface never creates a Run row — the user
    // publishes (which creates a deployed recipe → eventual Run) or
    // runs ephemerally without persistence. `list_runs` is empty
    // after the composition fires.
    let runs = daemon.list_runs().expect("list_runs");
    let composition_runs: Vec<_> = runs
        .iter()
        .filter(|r| r.recipe_name == "notebook-preview")
        .collect();
    assert!(
        composition_runs.is_empty(),
        "notebook should not have created a Run row: {composition_runs:?}",
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn notebook_run_threads_sample_limit_through_to_engine() {
    let tmp = tempfile::tempdir().unwrap();
    let ws_root = tmp.path().to_path_buf();

    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/items"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [
                    {"id": "a"}, {"id": "b"}, {"id": "c"},
                    {"id": "d"}, {"id": "e"}, {"id": "f"},
                ],
            })),
        )
        .mount(&mock)
        .await;

    let upstream_src = UPSTREAM.replace("MOCK_BASE_URL", &mock.uri());
    init_workspace(&ws_root, "scrape-products", &upstream_src);
    std::fs::write(ws_root.join("enrich-products.forage"), ENRICHER).unwrap();

    let daemon = Daemon::open(ws_root.clone()).expect("open daemon");
    deploy_disk_recipe(&daemon, &ws_root, "scrape-products");
    deploy_disk_recipe(&daemon, &ws_root, "enrich-products");

    // sample_limit caps every stage's top-level `for`-loop. The
    // upstream's loop is over the 6-item array; with sample_limit=2
    // it stops after 2 emits, and the downstream pass-through sees
    // only those 2.
    let flags = RunFlags {
        sample_limit: Some(2),
        replay: None,
        ephemeral: true,
    };
    let snapshot = daemon
        .run_composition(
            "notebook-preview",
            vec!["scrape-products".into(), "enrich-products".into()],
            indexmap::IndexMap::new(),
            flags,
        )
        .await
        .expect("run_composition succeeds");

    assert_eq!(snapshot.records.len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn notebook_run_surfaces_engine_failure_per_stage() {
    let tmp = tempfile::tempdir().unwrap();
    let ws_root = tmp.path().to_path_buf();

    // Don't init the workspace — referencing a non-deployed stage
    // should fail with a clear per-stage message rather than
    // silently emitting nothing.
    std::fs::write(
        ws_root.join("forage.toml"),
        "description = \"\"\ncategory = \"\"\ntags = []\n",
    )
    .unwrap();

    let daemon = Daemon::open(ws_root.clone()).expect("open daemon");
    let err = daemon
        .run_composition(
            "notebook-preview",
            vec!["missing-stage".into()],
            indexmap::IndexMap::new(),
            RunFlags::prod(),
        )
        .await
        .expect_err("undeployed stage must surface a failure");
    let msg = format!("{err}");
    assert!(
        msg.contains("missing-stage"),
        "error must name the missing stage: {msg}",
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn notebook_run_rejects_empty_chain() {
    let tmp = tempfile::tempdir().unwrap();
    let ws_root = tmp.path().to_path_buf();
    std::fs::write(
        ws_root.join("forage.toml"),
        "description = \"\"\ncategory = \"\"\ntags = []\n",
    )
    .unwrap();
    let daemon = Daemon::open(ws_root.clone()).expect("open daemon");
    let err = daemon
        .run_composition(
            "notebook-preview",
            Vec::new(),
            indexmap::IndexMap::new(),
            RunFlags::prod(),
        )
        .await
        .expect_err("empty composition must be rejected");
    assert!(format!("{err}").contains("zero stages"));
}
