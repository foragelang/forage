//! Recipe-name keying: every daemon path/key anchors on the header
//! name in `recipe "<name>"`, not on the file basename or any
//! path-derived slug. A workspace whose source file is
//! `amazon-scrape.forage` but whose header reads `recipe
//! "amazon-products"` deploys, configures, and writes output under
//! `amazon-products`.

mod common;

use forage_daemon::{Cadence, Daemon, RunConfig};
use rusqlite::Connection;

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
        inputs: indexmap::IndexMap::new(),
    };
    let updated = daemon
        .configure_run(recipe_name, cfg)
        .expect("configure_run");
    assert_eq!(updated.id, run.id, "configure is an update, not an insert");
    assert_eq!(updated.recipe_name, "amazon-products");
    assert_eq!(updated.output, expected_output);
}

/// A v2 daemon DB plus pre-Phase-4 disk layout (SQLite output store
/// named after the slug, `deployments/<slug>/`, `runs.recipe_name`
/// holding the slug) opens cleanly through the v3 schema bump and the
/// data-layer reconciliation: every row, file, and directory keyed on
/// the legacy path-derived slug moves to the recipe's header name.
#[test]
fn opening_legacy_v2_state_migrates_slug_to_header_name() {
    let tmp = tempfile::tempdir().unwrap();
    let ws_root = tmp.path().to_path_buf();
    let legacy_slug = "zen-leaf-elkridge";
    let header_name = "zen-leaf-products";

    // Workspace marker + a `<slug>/recipe.forage` file whose header
    // name differs from the folder slug. This is the exact shape the
    // user's local recipes have today.
    std::fs::write(
        ws_root.join("forage.toml"),
        "description = \"\"\ncategory = \"\"\ntags = []\n",
    )
    .unwrap();
    let recipe_dir = ws_root.join(legacy_slug);
    std::fs::create_dir_all(&recipe_dir).unwrap();
    std::fs::write(
        recipe_dir.join("recipe.forage"),
        format!("recipe \"{header_name}\"\nengine http\n"),
    )
    .unwrap();

    // Pre-create the pre-Phase-4 daemon state: a v2 SQLite DB with a
    // slug-keyed `runs` row, a slug-keyed deployments dir, a slug-keyed
    // output store. Mirrors what an in-place upgrade has to handle.
    let daemon_dir = ws_root.join(".forage");
    std::fs::create_dir_all(&daemon_dir).unwrap();
    let data_dir = daemon_dir.join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let output_store = data_dir.join(format!("{legacy_slug}.sqlite"));
    std::fs::write(&output_store, b"sentinel output store").unwrap();
    let deployments_dir = daemon_dir.join("deployments");
    let legacy_deployment = deployments_dir.join(legacy_slug).join("v1");
    std::fs::create_dir_all(&legacy_deployment).unwrap();
    std::fs::write(legacy_deployment.join("recipe.forage"), b"deployed src").unwrap();
    std::fs::write(legacy_deployment.join("catalog.json"), b"{}").unwrap();
    let db_path = daemon_dir.join("daemon.sqlite");
    {
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE _meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            INSERT INTO _meta(key, value) VALUES ('schema_version', '2');
            CREATE TABLE runs (
                id              TEXT PRIMARY KEY,
                recipe_slug     TEXT NOT NULL,
                workspace_root  TEXT NOT NULL,
                enabled         INTEGER NOT NULL,
                cadence_json    TEXT NOT NULL,
                output_path     TEXT NOT NULL,
                health          TEXT NOT NULL,
                next_run        INTEGER,
                deployed_version INTEGER
            );
            CREATE UNIQUE INDEX runs_recipe_slug ON runs(recipe_slug);
            CREATE TABLE scheduled_runs (
                id              TEXT PRIMARY KEY,
                run_id          TEXT NOT NULL,
                at              INTEGER NOT NULL,
                trigger         TEXT NOT NULL,
                outcome         TEXT NOT NULL,
                duration_s      REAL NOT NULL,
                counts_json     TEXT NOT NULL,
                diagnostics     INTEGER NOT NULL,
                stall           TEXT,
                recipe_version  INTEGER,
                FOREIGN KEY (run_id) REFERENCES runs(id) ON DELETE CASCADE
            );
            CREATE TABLE deployed_versions (
                slug         TEXT NOT NULL,
                version      INTEGER NOT NULL,
                deployed_at  INTEGER NOT NULL,
                PRIMARY KEY (slug, version)
            );
            "#,
        )
        .unwrap();
        conn.execute(
            "INSERT INTO runs(id, recipe_slug, workspace_root, enabled, cadence_json, output_path, health, next_run, deployed_version)
             VALUES (?1, ?2, ?3, 1, '{\"kind\":\"manual\"}', ?4, 'unknown', NULL, 1)",
            rusqlite::params![
                "run-legacy",
                legacy_slug,
                ws_root.to_string_lossy(),
                output_store.to_string_lossy(),
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO deployed_versions(slug, version, deployed_at) VALUES (?1, 1, 0)",
            rusqlite::params![legacy_slug],
        )
        .unwrap();
    }

    // Open through the current schema. The v2 → v3 transition runs
    // the schema bump (column rename) and the data migration.
    let daemon = Daemon::open(ws_root.clone()).expect("open daemon under v3 migration");

    // The runs row now keys on the header name and its output path
    // points at the renamed SQLite file.
    let runs = daemon.list_runs().expect("list runs");
    assert_eq!(runs.len(), 1);
    let run = &runs[0];
    assert_eq!(run.id, "run-legacy");
    assert_eq!(run.recipe_name, header_name);
    let expected_output = data_dir.join(format!("{header_name}.sqlite"));
    assert_eq!(run.output, expected_output);

    // The output-store file moved to the header-name basename.
    assert!(expected_output.is_file(), "renamed output store exists");
    assert!(
        !output_store.exists(),
        "old slug-keyed output store must be gone after rename"
    );

    // The deployments dir moved to the header-name basename.
    assert!(
        deployments_dir.join(header_name).join("v1").is_dir(),
        "renamed deployments dir exists",
    );
    assert!(
        !deployments_dir.join(legacy_slug).exists(),
        "old slug-keyed deployments dir is gone",
    );

    // The deployed-versions row keys on the header name.
    let dvs = daemon.deployed_versions(header_name).expect("deployed_versions");
    assert_eq!(dvs.len(), 1);
    assert_eq!(dvs[0].recipe_name, header_name);

    // Re-opening is idempotent: a second open finds the schema is
    // already current and does no further renaming work.
    drop(daemon);
    let daemon2 = Daemon::open(ws_root.clone()).expect("re-open after migration");
    let runs2 = daemon2.list_runs().unwrap();
    assert_eq!(runs2.len(), 1);
    assert_eq!(runs2[0].recipe_name, header_name);
    assert_eq!(runs2[0].output, expected_output);
}

/// A `data/<orphan>.sqlite` file with no matching recipe in the
/// workspace stays untouched: the daemon doesn't delete the user's
/// data and doesn't have enough information to rename it. The
/// migration logs a warn and moves on.
#[tracing_test::traced_test]
#[test]
fn legacy_state_with_no_matching_recipe_stays_in_place() {
    let tmp = tempfile::tempdir().unwrap();
    let ws_root = tmp.path().to_path_buf();
    std::fs::write(
        ws_root.join("forage.toml"),
        "description = \"\"\ncategory = \"\"\ntags = []\n",
    )
    .unwrap();
    // Workspace has one valid recipe.
    let kept = ws_root.join("kept.forage");
    std::fs::write(&kept, "recipe \"kept\"\nengine http\n").unwrap();

    // Pre-Phase-4 state: an orphan output store + DB row whose recipe
    // is no longer on disk.
    let daemon_dir = ws_root.join(".forage");
    let data_dir = daemon_dir.join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let orphan_store = data_dir.join("orphan.sqlite");
    std::fs::write(&orphan_store, b"orphan").unwrap();
    let db_path = daemon_dir.join("daemon.sqlite");
    {
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE _meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            INSERT INTO _meta(key, value) VALUES ('schema_version', '2');
            CREATE TABLE runs (
                id              TEXT PRIMARY KEY,
                recipe_slug     TEXT NOT NULL,
                workspace_root  TEXT NOT NULL,
                enabled         INTEGER NOT NULL,
                cadence_json    TEXT NOT NULL,
                output_path     TEXT NOT NULL,
                health          TEXT NOT NULL,
                next_run        INTEGER,
                deployed_version INTEGER
            );
            CREATE UNIQUE INDEX runs_recipe_slug ON runs(recipe_slug);
            CREATE TABLE scheduled_runs (
                id              TEXT PRIMARY KEY,
                run_id          TEXT NOT NULL,
                at              INTEGER NOT NULL,
                trigger         TEXT NOT NULL,
                outcome         TEXT NOT NULL,
                duration_s      REAL NOT NULL,
                counts_json     TEXT NOT NULL,
                diagnostics     INTEGER NOT NULL,
                stall           TEXT,
                recipe_version  INTEGER,
                FOREIGN KEY (run_id) REFERENCES runs(id) ON DELETE CASCADE
            );
            CREATE TABLE deployed_versions (
                slug         TEXT NOT NULL,
                version      INTEGER NOT NULL,
                deployed_at  INTEGER NOT NULL,
                PRIMARY KEY (slug, version)
            );
            "#,
        )
        .unwrap();
        conn.execute(
            "INSERT INTO runs(id, recipe_slug, workspace_root, enabled, cadence_json, output_path, health, next_run)
             VALUES (?1, ?2, ?3, 1, '{\"kind\":\"manual\"}', ?4, 'unknown', NULL)",
            rusqlite::params![
                "run-orphan",
                "orphan",
                ws_root.to_string_lossy(),
                orphan_store.to_string_lossy(),
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO deployed_versions(slug, version, deployed_at) VALUES (?1, 1, 0)",
            rusqlite::params!["orphan"],
        )
        .unwrap();
    }

    let daemon = Daemon::open(ws_root.clone()).expect("open daemon");

    // The orphan row keeps its old `recipe_name` value — the migration
    // had nothing to translate to.
    let runs = daemon.list_runs().unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].recipe_name, "orphan");
    assert_eq!(runs[0].output, orphan_store);

    // The orphan output store file is still on disk.
    assert!(orphan_store.is_file(), "orphan store stays on disk");

    // The migration emits a warn log so the user can see the
    // unmigratable rows and decide how to clean up. The deployed_versions
    // row is also an orphan and surfaces its own warn line.
    assert!(
        logs_contain("no workspace recipe matches"),
        "expected warn log for orphan runs/deployed_versions row",
    );
}
