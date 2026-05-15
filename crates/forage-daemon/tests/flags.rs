//! `RunFlags` semantics — sampling, replay, ephemeral persistence —
//! exercised end-to-end through the daemon's `trigger_run` path.

use std::path::PathBuf;

use forage_daemon::{Cadence, Daemon, Outcome, OutputFormat, RunConfig, RunFlags};
use forage_replay::{Capture, HttpExchange, write_jsonl};
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

mod common;
use common::{deploy_disk_recipe, init_workspace};

/// A recipe with one paginated step that returns a 100-item array. The
/// engine emits one record per top-level for-loop iteration, so
/// `sample_limit = Some(5)` should yield 5 records.
const SAMPLE_RECIPE: &str = r#"recipe "sample-flag"
engine http

type Item {
    id: String
}

step list {
    method "GET"
    url    "STAND_IN_URL"
}

for $i in $list[*] {
    emit Item {
        id ← $i.id
    }
}
"#;

/// Items the live mock should respond with. Each test plants a path
/// match against `/items` and the response is the items list
/// serialized as JSON.
fn items_mock_for_path() -> wiremock::MockBuilder {
    Mock::given(method("GET")).and(wiremock::matchers::path("/items"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sample_limit_caps_recorded_emit_counts() {
    let tmp = tempfile::tempdir().unwrap();
    let ws_root = tmp.path().to_path_buf();
    let recipe_name = "sample-flag";

    let server = MockServer::start().await;
    let body: Vec<serde_json::Value> = (0..100)
        .map(|i| serde_json::json!({ "id": format!("r-{i}") }))
        .collect();
    items_mock_for_path()
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;

    let source = SAMPLE_RECIPE.replace("STAND_IN_URL", &format!("{}/items", server.uri()));
    init_workspace(&ws_root, recipe_name, &source);

    let daemon = Daemon::open(ws_root.clone()).expect("open daemon");
    deploy_disk_recipe(&daemon, &ws_root, recipe_name);

    let cfg = RunConfig {
        cadence: Cadence::Manual,
        output: ws_root.join(".forage").join("data").join("sample.sqlite"),
        enabled: true,
        inputs: indexmap::IndexMap::new(),
        output_format: OutputFormat::default(),
    };
    let run = daemon.configure_run(recipe_name, cfg).expect("configure_run");

    let flags = RunFlags {
        sample_limit: Some(5),
        replay: None,
        ephemeral: false,
    };
    let sr = daemon
        .trigger_run(&run.id, flags)
        .await
        .expect("trigger_run");
    assert_eq!(sr.outcome, Outcome::Ok, "stall: {:?}", sr.stall);
    assert_eq!(sr.counts.get("Item").copied(), Some(5));
}

/// `--replay` swaps in the recorded fixtures. The mock server is
/// configured to fail any live hit; the recipe still emits because
/// `RunFlags::replay` carries the captures path.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn replay_flag_plays_recipe_against_fixtures() {
    let tmp = tempfile::tempdir().unwrap();
    let ws_root = tmp.path().to_path_buf();
    let recipe_name = "sample-flag";

    // Live network would 500 — proves the replay path doesn't hit it.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    let live_url = format!("{}/items", server.uri());
    let source = SAMPLE_RECIPE.replace("STAND_IN_URL", &live_url);
    init_workspace(&ws_root, recipe_name, &source);

    let captures_path: PathBuf = ws_root.join("captures.jsonl");
    write_jsonl(
        &captures_path,
        &[Capture::Http(HttpExchange {
            url: live_url.clone(),
            method: "GET".into(),
            request_headers: indexmap::IndexMap::new(),
            request_body: None,
            status: 200,
            response_headers: indexmap::IndexMap::new(),
            body: r#"[{"id":"f-1"},{"id":"f-2"},{"id":"f-3"}]"#.into(),
        })],
    )
    .expect("write captures");

    let daemon = Daemon::open(ws_root.clone()).expect("open daemon");
    deploy_disk_recipe(&daemon, &ws_root, recipe_name);

    let cfg = RunConfig {
        cadence: Cadence::Manual,
        output: ws_root.join(".forage").join("data").join("replay.sqlite"),
        enabled: true,
        inputs: indexmap::IndexMap::new(),
        output_format: OutputFormat::default(),
    };
    let run = daemon.configure_run(recipe_name, cfg).expect("configure_run");

    let flags = RunFlags {
        sample_limit: None,
        replay: Some(captures_path),
        ephemeral: false,
    };
    let sr = daemon
        .trigger_run(&run.id, flags)
        .await
        .expect("trigger_run");
    assert_eq!(sr.outcome, Outcome::Ok, "stall: {:?}", sr.stall);
    assert_eq!(sr.counts.get("Item").copied(), Some(3));
}

/// Ephemeral runs route writes to an in-memory store. The persistent
/// file at `Run.output` stays absent — the directory may not even
/// exist after the run.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ephemeral_flag_skips_persistent_output_store() {
    let tmp = tempfile::tempdir().unwrap();
    let ws_root = tmp.path().to_path_buf();
    let recipe_name = "sample-flag";

    let server = MockServer::start().await;
    let body = serde_json::json!([{"id": "x"}, {"id": "y"}]);
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;
    let source = SAMPLE_RECIPE.replace("STAND_IN_URL", &format!("{}/items", server.uri()));
    init_workspace(&ws_root, recipe_name, &source);

    let daemon = Daemon::open(ws_root.clone()).expect("open daemon");
    deploy_disk_recipe(&daemon, &ws_root, recipe_name);

    let persistent = ws_root.join(".forage").join("data").join("ephemeral.sqlite");
    let cfg = RunConfig {
        cadence: Cadence::Manual,
        output: persistent.clone(),
        enabled: true,
        inputs: indexmap::IndexMap::new(),
        output_format: OutputFormat::default(),
    };
    let run = daemon.configure_run(recipe_name, cfg).expect("configure_run");

    let flags = RunFlags {
        sample_limit: None,
        replay: None,
        ephemeral: true,
    };
    let sr = daemon
        .trigger_run(&run.id, flags)
        .await
        .expect("trigger_run");
    assert_eq!(sr.outcome, Outcome::Ok, "stall: {:?}", sr.stall);
    assert_eq!(sr.counts.get("Item").copied(), Some(2));

    // The persistent file at the configured output path must not
    // exist: ephemeral runs land in :memory: and drop on completion.
    assert!(
        !persistent.exists(),
        "ephemeral run wrote to {}",
        persistent.display(),
    );
    // load_records reads from the configured output path; ephemeral
    // writes evaporate, so a follow-up read finds nothing.
    let records = daemon
        .load_records(&sr.id, "Item", 10)
        .expect("load_records");
    assert!(records.is_empty(), "ephemeral records leaked: {records:?}");
}

/// The dev preset bundles all three flags. The "Run" button passes it
/// straight through; the persistent store remains untouched and the
/// sample cap clamps to the preset's value.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dev_preset_applies_sample_and_ephemeral_together() {
    let tmp = tempfile::tempdir().unwrap();
    let ws_root = tmp.path().to_path_buf();
    let recipe_name = "sample-flag";

    let server = MockServer::start().await;
    let body: Vec<serde_json::Value> = (0..100)
        .map(|i| serde_json::json!({ "id": format!("r-{i}") }))
        .collect();
    items_mock_for_path()
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;
    let source = SAMPLE_RECIPE.replace("STAND_IN_URL", &format!("{}/items", server.uri()));
    init_workspace(&ws_root, recipe_name, &source);

    let daemon = Daemon::open(ws_root.clone()).expect("open daemon");
    deploy_disk_recipe(&daemon, &ws_root, recipe_name);

    let persistent = ws_root.join(".forage").join("data").join("dev.sqlite");
    let cfg = RunConfig {
        cadence: Cadence::Manual,
        output: persistent.clone(),
        enabled: true,
        inputs: indexmap::IndexMap::new(),
        output_format: OutputFormat::default(),
    };
    let run = daemon.configure_run(recipe_name, cfg).expect("configure_run");

    // The dev preset's `replay` field is left at None: the caller
    // (Studio / CLI) fills it in if a fixture exists at
    // `_fixtures/<recipe>.jsonl`. This test exercises the no-fixture
    // dev path — sampled live HTTP, ephemeral persistence.
    let sr = daemon
        .trigger_run(&run.id, RunFlags::dev())
        .await
        .expect("trigger_run");
    assert_eq!(sr.outcome, Outcome::Ok, "stall: {:?}", sr.stall);
    assert_eq!(sr.counts.get("Item").copied(), Some(10));
    assert!(!persistent.exists(), "dev preset must not write persistent store");
}

/// All three flags on at once: sampled, replayed, ephemeral. The
/// captures hold 100 records, sample_limit caps at 5, ephemeral
/// keeps the persistent store untouched. This exercises the full
/// "dev preset with a fixture file resolved" shape the CLI / Studio
/// build when they detect `_fixtures/<recipe>.jsonl` is present.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn all_three_flags_compose() {
    let tmp = tempfile::tempdir().unwrap();
    let ws_root = tmp.path().to_path_buf();
    let recipe_name = "sample-flag";

    // The live mock errors any request — the run survives because
    // replay short-circuits the live transport.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    let live_url = format!("{}/items", server.uri());
    let source = SAMPLE_RECIPE.replace("STAND_IN_URL", &live_url);
    init_workspace(&ws_root, recipe_name, &source);

    let captures_path = ws_root.join("captures.jsonl");
    let mut body = String::from("[");
    for i in 0..100 {
        if i > 0 {
            body.push(',');
        }
        body.push_str(&format!(r#"{{"id":"r-{i}"}}"#));
    }
    body.push(']');
    write_jsonl(
        &captures_path,
        &[Capture::Http(HttpExchange {
            url: live_url.clone(),
            method: "GET".into(),
            request_headers: indexmap::IndexMap::new(),
            request_body: None,
            status: 200,
            response_headers: indexmap::IndexMap::new(),
            body,
        })],
    )
    .expect("write captures");

    let daemon = Daemon::open(ws_root.clone()).expect("open daemon");
    deploy_disk_recipe(&daemon, &ws_root, recipe_name);

    let persistent = ws_root.join(".forage").join("data").join("triple.sqlite");
    let cfg = RunConfig {
        cadence: Cadence::Manual,
        output: persistent.clone(),
        enabled: true,
        inputs: indexmap::IndexMap::new(),
        output_format: OutputFormat::default(),
    };
    let run = daemon.configure_run(recipe_name, cfg).expect("configure_run");

    let flags = RunFlags {
        sample_limit: Some(5),
        replay: Some(captures_path),
        ephemeral: true,
    };
    let sr = daemon
        .trigger_run(&run.id, flags)
        .await
        .expect("trigger_run");
    assert_eq!(sr.outcome, Outcome::Ok, "stall: {:?}", sr.stall);
    assert_eq!(sr.counts.get("Item").copied(), Some(5));
    assert!(
        !persistent.exists(),
        "ephemeral run with all three flags must not persist",
    );
}
