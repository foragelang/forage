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

/// Closure deployment freezes every stage version at the moment the
/// composition itself is deployed. Redeploying a stage afterwards must
/// not change what the composition runs — the on-disk module carries
/// the resolved closure, and the runtime walks that closure rather than
/// re-resolving names at run time.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn composition_pins_stage_versions_against_subsequent_redeploys() {
    let tmp = tempfile::tempdir().unwrap();
    let ws_root = tmp.path().to_path_buf();

    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/items"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "items": [{"id": "seed"}],
        })))
        .mount(&mock)
        .await;

    // Scraper is the upstream stage — never redeployed. The runtime
    // drift this test guards against would show up in the downstream
    // `inner` stage, where the redeploy lands.
    let scraper = format!(
        r#"recipe "scraper"
engine http

share type Product {{ id: String }}

emits Product

step list {{
    method "GET"
    url    "{}/items"
}}

for $i in $list.items[*] {{
    emit Product {{ id ← $i.id }}
}}
"#,
        mock.uri(),
    );
    let inner_v1 = r#"recipe "inner"
engine http

share type Product { id: String }

input prior: [Product]

emits Product

for $p in $input.prior {
    emit Product { id ← "v1-marker" }
}
"#;
    let inner_v2 = r#"recipe "inner"
engine http

share type Product { id: String }

input prior: [Product]

emits Product

for $p in $input.prior {
    emit Product { id ← "v2-marker" }
}
"#;
    let outer = r#"recipe "outer"
engine http

share type Product { id: String }

emits Product

compose "scraper" | "inner"
"#;

    init_workspace(&ws_root, "scraper", &scraper);
    std::fs::write(ws_root.join("inner.forage"), inner_v1).unwrap();
    std::fs::write(ws_root.join("outer.forage"), outer).unwrap();

    let daemon = Daemon::open(ws_root.clone()).expect("open daemon");

    // Deploy scraper, then inner v1, then outer. The outer module's
    // closure freezes the chain — scraper@v1 + inner@v1.
    deploy_disk_recipe(&daemon, &ws_root, "scraper");
    deploy_disk_recipe(&daemon, &ws_root, "inner");
    deploy_disk_recipe(&daemon, &ws_root, "outer");

    // Redeploy inner with the v2 source. `current_deployed("inner")`
    // now reports v2; outer's frozen closure still references v1.
    std::fs::write(ws_root.join("inner.forage"), inner_v2).unwrap();
    deploy_disk_recipe(&daemon, &ws_root, "inner");

    // Run outer through the normal Run pathway. The runtime walks
    // outer's closure (inner@v1) — never re-resolves the stage by
    // name — so the records carry the v1 marker even though v2 is
    // now the latest deployment of inner.
    let cfg = RunConfig {
        cadence: Cadence::Manual,
        output: ws_root.join(".forage").join("data").join("outer.sqlite"),
        enabled: true,
        inputs: indexmap::IndexMap::new(),
        output_format: OutputFormat::default(),
    };
    let run = daemon.configure_run("outer", cfg).expect("configure_run");
    let sr = daemon
        .trigger_run(&run.id, RunFlags::prod())
        .await
        .expect("trigger_run");
    assert_eq!(sr.outcome, Outcome::Ok, "stall: {:?}", sr.stall);

    let records = daemon
        .load_records(&sr.id, "Product", 100)
        .expect("load records");
    let ids: Vec<&str> = records
        .iter()
        .filter_map(|r| r.get("id").and_then(|v| v.as_str()))
        .collect();
    assert_eq!(
        ids,
        vec!["v1-marker"],
        "outer's frozen closure pinned inner@v1; the v2 redeploy must \
         not drift through the composition",
    );
}

/// Notebook composition: when an explicit user-listed stage shares a
/// name with a transitive entry already pulled in from an earlier
/// stage's frozen closure, the explicit listing wins — i.e. the
/// stage resolves to its current deployment, not the older pin a
/// previous composition happens to carry.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn notebook_explicit_stage_overrides_earlier_stages_frozen_closure() {
    let tmp = tempfile::tempdir().unwrap();
    let ws_root = tmp.path().to_path_buf();

    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/items"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "items": [{"id": "seed"}],
        })))
        .mount(&mock)
        .await;

    // Seed scraper feeds into B. Its records flow into A (a
    // composition referencing B), and separately into the explicit
    // B stage in the notebook chain.
    let seed = format!(
        r#"recipe "seed"
engine http

share type Product {{ id: String }}

emits Product

step list {{
    method "GET"
    url    "{}/items"
}}

for $i in $list.items[*] {{
    emit Product {{ id ← $i.id }}
}}
"#,
        mock.uri(),
    );
    let b_v1 = r#"recipe "B"
engine http

share type Product { id: String }

input prior: [Product]

emits Product

for $p in $input.prior {
    emit Product { id ← "v1" }
}
"#;
    let b_v2 = r#"recipe "B"
engine http

share type Product { id: String }

input prior: [Product]

emits Product

for $p in $input.prior {
    emit Product { id ← "v2" }
}
"#;
    // A is a composition `seed | B`. After deploy at v1, A's frozen
    // closure pins B@v1.
    let a = r#"recipe "A"
engine http

share type Product { id: String }

emits Product

compose "seed" | "B"
"#;

    init_workspace(&ws_root, "seed", &seed);
    std::fs::write(ws_root.join("B.forage"), b_v1).unwrap();
    std::fs::write(ws_root.join("A.forage"), a).unwrap();

    let daemon = Daemon::open(ws_root.clone()).expect("open daemon");
    deploy_disk_recipe(&daemon, &ws_root, "seed");
    deploy_disk_recipe(&daemon, &ws_root, "B");
    deploy_disk_recipe(&daemon, &ws_root, "A");

    // Redeploy B with the v2 source. A's frozen closure still
    // pins B@v1; the latest deployment of B is now v2.
    std::fs::write(ws_root.join("B.forage"), b_v2).unwrap();
    deploy_disk_recipe(&daemon, &ws_root, "B");

    // Notebook run with stages = ["A", "B"]. Stage A walks its
    // frozen closure (B@v1 internally → "v1" markers). Stage B is
    // the explicit listing — it must resolve to the current
    // deployment (B@v2 → "v2" markers) regardless of A's pin.
    let snapshot = daemon
        .run_composition(
            "notebook-collision",
            vec!["A".into(), "B".into()],
            indexmap::IndexMap::new(),
            RunFlags::prod(),
        )
        .await
        .expect("run_composition succeeds");

    let ids: Vec<&str> = snapshot
        .records
        .iter()
        .filter_map(|r| match r.fields.get("id") {
            Some(forage_core::ast::JSONValue::String(s)) => Some(s.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        ids,
        vec!["v2"],
        "explicit B stage must run B@v2 (the current deployment) — \
         the earlier A stage's frozen closure pins B@v1 only inside A",
    );
}
