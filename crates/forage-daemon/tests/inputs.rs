//! Per-Run inputs flow from `RunConfig.inputs` through the daemon
//! into the engine. The legacy on-disk auto-load (next to a
//! `<slug>/recipe.forage`) is gone; the only path is the explicit
//! field on the row.

use forage_daemon::{Cadence, Daemon, Outcome, RunConfig};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

mod common;
use common::{deploy_disk_recipe, init_workspace};

/// Recipe with an `input` that the URL interpolates against. The
/// engine fails to execute the request unless the input lands —
/// wiremock only matches a path containing the configured tenant.
const RECIPE: &str = r#"recipe "tenant-items"
engine http

type Item {
    id: String
}

input tenant: String

step list {
    method "GET"
    url    "TENANT_BASE_URL/{$input.tenant}/items"
}

for $i in $list.items[*] {
    emit Item {
        id ← $i.id
    }
}
"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn configure_run_with_inputs_passes_them_to_engine() {
    let tmp = tempfile::tempdir().unwrap();
    let ws_root = tmp.path().to_path_buf();
    let recipe_name = "tenant-items";

    // Mock answers only on `/acme/items` — any other path 404s, which
    // would surface as a runtime error in the recipe's `$list.items`
    // dereference. The presence of two emitted items is therefore
    // evidence that the `tenant=acme` input reached the engine.
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/acme/items"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [{"id": "a"}, {"id": "b"}],
            })),
        )
        .mount(&mock)
        .await;

    let recipe_src = RECIPE.replace("TENANT_BASE_URL", &mock.uri());
    init_workspace(&ws_root, recipe_name, &recipe_src);

    let daemon = Daemon::open(ws_root.clone()).expect("open daemon");
    deploy_disk_recipe(&daemon, &ws_root, recipe_name);

    let mut inputs = indexmap::IndexMap::new();
    inputs.insert("tenant".to_string(), serde_json::Value::String("acme".into()));
    let cfg = RunConfig {
        cadence: Cadence::Manual,
        output: ws_root.join(".forage").join("data").join("items.sqlite"),
        enabled: true,
        inputs,
    };
    let run = daemon.configure_run(recipe_name, cfg).expect("configure_run");

    let sr = daemon.trigger_run(&run.id).await.expect("trigger_run");
    assert_eq!(sr.outcome, Outcome::Ok, "stall: {:?}", sr.stall);
    assert_eq!(sr.counts.get("Item").copied(), Some(2));

    // The inputs round-trip on the row, so a follow-up scheduler tick
    // would see the same values without the user reconfiguring.
    let refreshed = daemon.get_run(&run.id).unwrap().unwrap();
    assert_eq!(
        refreshed.inputs.get("tenant").and_then(|v| v.as_str()),
        Some("acme"),
    );
}

/// Empty `RunConfig.inputs` means the engine sees no bindings. The
/// daemon must never fall back to reading a legacy
/// `<workspace>/<slug>/fixtures/inputs.json` file — if one happens to
/// exist on disk the run still gets an empty map. Recipes that
/// declare an `input` will surface the unbound reference as a normal
/// engine error.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn empty_inputs_do_not_fall_back_to_legacy_inputs_file() {
    let tmp = tempfile::tempdir().unwrap();
    let ws_root = tmp.path().to_path_buf();
    let recipe_name = "tenant-items";

    let mock = MockServer::start().await;
    // Any path 404s — if the daemon picks up `tenant=acme` from a
    // legacy file we'd hit `/acme/items` and the recipe would emit.
    // With explicit empty inputs the URL interpolates to
    // `/<empty>/items` (or fails earlier on the unbound reference),
    // and the outcome must be a clean Fail with zero records.
    Mock::given(method("GET"))
        .and(path("/acme/items"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [{"id": "leak"}],
            })),
        )
        .mount(&mock)
        .await;

    let recipe_src = RECIPE.replace("TENANT_BASE_URL", &mock.uri());
    init_workspace(&ws_root, recipe_name, &recipe_src);

    // Plant a legacy inputs.json at the pre-Phase-10 location to
    // confirm the daemon never reads it. The flat shape leaves
    // `<workspace>/<slug>/` empty, so we have to create the
    // directory by hand for the file to land.
    let legacy_dir = ws_root.join(recipe_name).join("fixtures");
    std::fs::create_dir_all(&legacy_dir).expect("plant legacy dir");
    std::fs::write(
        legacy_dir.join("inputs.json"),
        r#"{"tenant": "acme"}"#,
    )
    .expect("plant legacy inputs.json");

    let daemon = Daemon::open(ws_root.clone()).expect("open daemon");
    deploy_disk_recipe(&daemon, &ws_root, recipe_name);

    let cfg = RunConfig {
        cadence: Cadence::Manual,
        output: ws_root.join(".forage").join("data").join("items.sqlite"),
        enabled: true,
        inputs: indexmap::IndexMap::new(),
    };
    let run = daemon.configure_run(recipe_name, cfg).expect("configure_run");
    assert!(
        run.inputs.is_empty(),
        "fresh configure with empty inputs must persist an empty map",
    );

    let sr = daemon.trigger_run(&run.id).await.expect("trigger_run");
    // The engine fails on the unbound `$input.tenant`. The exact
    // message is the engine's concern — we only pin the row-level
    // outcome (no leak from the legacy file).
    assert_eq!(sr.outcome, Outcome::Fail, "stall: {:?}", sr.stall);
    assert!(
        sr.counts.is_empty(),
        "no records may be emitted; got {:?}",
        sr.counts,
    );

    // Sanity: the legacy file still exists on disk. If the daemon
    // had silently read it the assertions above would have flipped.
    assert!(
        ws_root
            .join(recipe_name)
            .join("fixtures")
            .join("inputs.json")
            .exists()
    );
}
