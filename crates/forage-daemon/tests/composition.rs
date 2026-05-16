//! End-to-end composition: deploy a chain of recipes, trigger the
//! composed recipe, verify that records flow upstream → downstream
//! and the final snapshot lands in the daemon's output store.

use forage_daemon::{Cadence, Daemon, Outcome, OutputFormat, RunConfig, RunFlags};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

mod common;
use common::{deploy_disk_recipe, init_workspace};

/// Stage 1: HTTP scrape that emits two Products with ids "a" and "b".
const UPSTREAM: &str = r#"recipe "scrape-products"
engine http

share type Product { id: String }

emits Product

step list {
    method "GET"
    url    "MOCK_BASE_URL/items"
}

for $i in $list.items[*] {
    emit Product { id ← $i.id }
}
"#;

/// Stage 2: takes a `[Product]` input and re-emits a Product per input
/// with the id prefixed by "enriched-". The recipe has no HTTP step
/// — its job is to transform the upstream stream.
const ENRICHER: &str = r#"recipe "enrich-products"
engine http

share type Product { id: String }

input prior: [Product]

emits Product

for $p in $input.prior {
    emit Product { id ← $p.id }
}
"#;

/// Composition: `scrape-products | enrich-products`. The composition
/// itself has no inputs (stage 1 doesn't take any) and declares its
/// output as Product, the type both inner stages produce.
const COMPOSITION: &str = r#"recipe "composed"
engine http

share type Product { id: String }

emits Product

compose "scrape-products" | "enrich-products"
"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn composition_runs_chain_and_emits_downstream_records() {
    let tmp = tempfile::tempdir().unwrap();
    let ws_root = tmp.path().to_path_buf();

    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/items"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "items": [{"id": "a"}, {"id": "b"}],
        })))
        .mount(&mock)
        .await;

    let upstream_src = UPSTREAM.replace("MOCK_BASE_URL", &mock.uri());
    init_workspace(&ws_root, "scrape-products", &upstream_src);
    // Add the enricher + the composition as sibling files.
    std::fs::write(ws_root.join("enrich-products.forage"), ENRICHER).unwrap();
    std::fs::write(ws_root.join("composed.forage"), COMPOSITION).unwrap();

    let daemon = Daemon::open(ws_root.clone()).expect("open daemon");
    deploy_disk_recipe(&daemon, &ws_root, "scrape-products");
    deploy_disk_recipe(&daemon, &ws_root, "enrich-products");
    deploy_disk_recipe(&daemon, &ws_root, "composed");

    let cfg = RunConfig {
        cadence: Cadence::Manual,
        output: ws_root.join(".forage").join("data").join("composed.sqlite"),
        enabled: true,
        inputs: indexmap::IndexMap::new(),
        output_format: OutputFormat::default(),
    };
    let run = daemon
        .configure_run("composed", cfg)
        .expect("configure_run");
    let sr = daemon
        .trigger_run(&run.id, RunFlags::prod())
        .await
        .expect("trigger_run");
    assert_eq!(sr.outcome, Outcome::Ok, "stall: {:?}", sr.stall);
    // The enricher emits one Product per upstream record.
    assert_eq!(sr.counts.get("Product").copied(), Some(2));

    // Records are persisted under the composition's output store, not
    // the inner stages' — the daemon writes only the final snapshot.
    let records = daemon
        .load_records(&sr.id, "Product", 100)
        .expect("load records");
    let ids: Vec<&str> = records
        .iter()
        .filter_map(|r| r.get("id").and_then(|v| v.as_str()))
        .collect();
    assert_eq!(ids, vec!["a", "b"]);
}

/// A composition is itself a recipe; it can be composed in turn. Pin
/// the typed-function-closed-under-composition invariant by deploying
/// a composition that references another composition.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn nested_composition_chains_three_stages() {
    let tmp = tempfile::tempdir().unwrap();
    let ws_root = tmp.path().to_path_buf();

    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/items"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "items": [{"id": "a"}, {"id": "b"}],
        })))
        .mount(&mock)
        .await;

    let upstream_src = UPSTREAM.replace("MOCK_BASE_URL", &mock.uri());
    init_workspace(&ws_root, "scrape-products", &upstream_src);
    std::fs::write(ws_root.join("enrich-products.forage"), ENRICHER).unwrap();
    std::fs::write(ws_root.join("composed.forage"), COMPOSITION).unwrap();

    // Wrap the inner composition with another enrich pass.
    let outer_composition = r#"recipe "double-composed"
engine http

share type Product { id: String }

emits Product

compose "composed" | "enrich-products"
"#;
    std::fs::write(ws_root.join("double-composed.forage"), outer_composition).unwrap();

    let daemon = Daemon::open(ws_root.clone()).expect("open daemon");
    deploy_disk_recipe(&daemon, &ws_root, "scrape-products");
    deploy_disk_recipe(&daemon, &ws_root, "enrich-products");
    deploy_disk_recipe(&daemon, &ws_root, "composed");
    deploy_disk_recipe(&daemon, &ws_root, "double-composed");

    let cfg = RunConfig {
        cadence: Cadence::Manual,
        output: ws_root.join(".forage").join("data").join("double.sqlite"),
        enabled: true,
        inputs: indexmap::IndexMap::new(),
        output_format: OutputFormat::default(),
    };
    let run = daemon
        .configure_run("double-composed", cfg)
        .expect("configure_run");
    let sr = daemon
        .trigger_run(&run.id, RunFlags::prod())
        .await
        .expect("trigger_run");
    assert_eq!(sr.outcome, Outcome::Ok, "stall: {:?}", sr.stall);
    assert_eq!(sr.counts.get("Product").copied(), Some(2));
}
