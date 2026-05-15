//! Recipe-name keying: every daemon path/key anchors on the header
//! name in `recipe "<name>"`, not on the file basename or any
//! path-derived slug. A workspace whose source file is
//! `amazon-scrape.forage` but whose header reads `recipe
//! "amazon-products"` deploys, configures, and writes output under
//! `amazon-products`.

mod common;

use forage_daemon::{Cadence, Daemon, RunConfig};

const RECIPE_AMAZON: &str = r#"recipe "amazon-products"
engine http

type Item {
    id: String
}

step list {
    method "GET"
    url    "https://example.test/items"
}

for $i in $list.items[*] {
    emit Item {
        id ← $i.id
    }
}
"#;

/// `default_output_path` keys on the recipe header name, not the file
/// basename. The host (Studio / CLI) resolves the header name from
/// the on-disk file before calling the daemon; the daemon never reads
/// the source path back into the output-store layout.
#[test]
fn default_output_path_uses_recipe_header_name() {
    let tmp = tempfile::tempdir().unwrap();
    let ws_root = tmp.path().to_path_buf();
    let daemon = Daemon::open(ws_root.clone()).expect("open daemon");

    let path = daemon.default_output_path("amazon-products");
    let expected = ws_root
        .join(".forage")
        .join("data")
        .join("amazon-products.sqlite");
    assert_eq!(path, expected);
}

/// Deploy + ensure_run + output-store path for a workspace whose
/// on-disk file basename differs from the header name. Pre-Phase-4 the
/// daemon keyed on a path-derived slug, so the output would have
/// landed at `data/amazon-scrape.sqlite`; the contract under test is
/// that it now lands at `data/amazon-products.sqlite`.
#[test]
fn deploy_and_default_output_anchor_on_header_name() {
    let tmp = tempfile::tempdir().unwrap();
    let ws_root = tmp.path().to_path_buf();
    // Workspace marker. The source file basename (`amazon-scrape`)
    // intentionally differs from the recipe header name
    // (`amazon-products`).
    std::fs::write(
        ws_root.join("forage.toml"),
        "description = \"\"\ncategory = \"\"\ntags = []\n",
    )
    .unwrap();
    std::fs::write(ws_root.join("amazon-scrape.forage"), RECIPE_AMAZON).unwrap();

    let daemon = Daemon::open(ws_root.clone()).expect("open daemon");
    let workspace = forage_core::load(&ws_root).expect("load workspace");
    let recipe = forage_core::parse(RECIPE_AMAZON).expect("parse");
    let catalog = workspace
        .catalog(&recipe, |p| std::fs::read_to_string(p))
        .expect("catalog");
    let wire = forage_core::SerializableCatalog::from(catalog);

    let recipe_name = recipe.recipe_name().expect("recipe header name");
    assert_eq!(recipe_name, "amazon-products");

    let dv = daemon
        .deploy(recipe_name, RECIPE_AMAZON.to_string(), wire)
        .expect("deploy");
    assert_eq!(dv.recipe_name, "amazon-products");

    // The on-disk deployment lives under the header name, not the
    // file basename.
    let deployments_dir = ws_root
        .join(".forage")
        .join("deployments")
        .join("amazon-products")
        .join("v1");
    assert!(
        deployments_dir.is_dir(),
        "deployment dir must be keyed by header name, got {deployments_dir:?}"
    );
    let stale_slug_dir = ws_root
        .join(".forage")
        .join("deployments")
        .join("amazon-scrape");
    assert!(
        !stale_slug_dir.exists(),
        "no deployment dir under the file basename"
    );

    // `ensure_run` keys its default output path on the header name.
    let run = daemon.ensure_run(recipe_name).expect("ensure_run");
    assert_eq!(run.recipe_name, "amazon-products");
    let expected_output = ws_root
        .join(".forage")
        .join("data")
        .join("amazon-products.sqlite");
    assert_eq!(run.output, expected_output);

    // `configure_run` overrides the output path, but the run still
    // keys on the header name end-to-end.
    let cfg = RunConfig {
        cadence: Cadence::Manual,
        output: daemon.default_output_path(recipe_name),
        enabled: true,
    };
    let updated = daemon
        .configure_run(recipe_name, cfg)
        .expect("configure_run");
    assert_eq!(updated.id, run.id, "configure is an update, not an insert");
    assert_eq!(updated.recipe_name, "amazon-products");
    assert_eq!(updated.output, expected_output);
}
