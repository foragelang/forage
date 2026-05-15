//! Daemon lifecycle: open the daemon DB, deploy a recipe, configure a
//! Run by recipe name, trigger it manually against a recorded
//! transport, then verify the ScheduledRun was persisted and the
//! Run's health is `Ok`. The daemon only executes deployed versions,
//! so the deploy step is part of every meaningful integration test.

use std::path::Path;

use forage_daemon::{Cadence, Daemon, Health, Outcome, RunConfig, RunFlags, Trigger};

mod common;
use common::{deploy_disk_recipe, init_workspace};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn open_configure_trigger_persist() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let ws_root = tmp.path().to_path_buf();
    let recipe_name = "fixture-ok";

    // Recipe + workspace marker.
    init_workspace(&ws_root, recipe_name, RECIPE_OK);

    let mock = common::http_mock::server_returning_items(&[("a", 1.5), ("b", 2.0)]).await;
    let recipe_path = ws_root.join(format!("{recipe_name}.forage"));
    rewrite_url(&recipe_path, &mock.url("/items"));

    let daemon = Daemon::open(ws_root.clone()).expect("open daemon");

    let output = ws_root.join(".forage").join("data").join("ok.sqlite");
    let cfg = RunConfig {
        cadence: Cadence::Manual,
        output: output.clone(),
        enabled: true,
        inputs: indexmap::IndexMap::new(),
    };
    let run = daemon
        .configure_run(recipe_name, cfg)
        .expect("configure_run");
    assert_eq!(run.recipe_name, recipe_name);
    assert_eq!(run.health, Health::Unknown);
    assert!(
        run.deployed_version.is_none(),
        "configure_run without a prior deploy should leave deployed_version unset"
    );

    deploy_disk_recipe(&daemon, &ws_root, recipe_name);

    // Trigger; expect Ok outcome with two emitted records.
    let sr = daemon
        .trigger_run(&run.id, RunFlags::prod())
        .await
        .expect("trigger_run");
    assert_eq!(sr.outcome, Outcome::Ok, "stall: {:?}", sr.stall);
    assert_eq!(sr.trigger, Trigger::Manual);
    assert_eq!(sr.counts.get("Item").copied(), Some(2));

    // The ScheduledRun row is queryable.
    let rows = daemon
        .list_scheduled_runs(&run.id, 10, None)
        .expect("list scheduled_runs");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, sr.id);

    // Health refreshes to Ok after a single successful run (no drift
    // history yet, but Ok is the post-run baseline).
    let refreshed = daemon
        .get_run(&run.id)
        .expect("get_run")
        .expect("run exists");
    assert_eq!(refreshed.health, Health::Ok);

    // Output store has the table + the rows.
    let records = daemon
        .load_records(&sr.id, "Item", 10)
        .expect("load_records");
    assert_eq!(records.len(), 2);
}

fn rewrite_url(path: &Path, url: &str) {
    let src = std::fs::read_to_string(path).unwrap();
    let replaced = src.replace("https://example.test/items", url);
    std::fs::write(path, replaced).unwrap();
}

const RECIPE_OK: &str = r#"recipe "fixture-ok"
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
