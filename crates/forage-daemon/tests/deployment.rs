//! Deploy pipeline: the daemon accepts validated source + catalog,
//! writes both to disk under `<daemon_dir>/deployments/<recipe_name>/v<n>/`,
//! records the metadata row, and rejects parse / validation failures
//! before anything is written. The Run pointer (`run.deployed_version`)
//! advances atomically with the deploy when a Run row exists.

use std::path::Path;

use forage_core::SerializableCatalog;
use forage_daemon::{
    Cadence, Daemon, DeployError, Outcome, RunConfig,
};

mod common;
use common::init_workspace;

const RECIPE: &str = r#"recipe "fixture-ok"
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

const RECIPE_REFERENCES_UNDECLARED_TYPE: &str = r#"recipe "broken-validate"
engine http

step list {
    method "GET"
    url    "https://example.test/items"
}

for $i in $list.items[*] {
    emit Ghost {
        id ← $i.id
    }
}
"#;

fn catalog_for(source: &str, ws_root: &Path) -> SerializableCatalog {
    let recipe = forage_core::parse(source).expect("parse");
    let workspace = forage_core::load(ws_root).expect("load workspace");
    let cat = workspace
        .catalog(&recipe, |p| std::fs::read_to_string(p))
        .expect("catalog");
    SerializableCatalog::from(cat)
}

#[test]
fn daemon_opens_in_empty_tempdir_without_forage_toml() {
    // Workspace marker is the host's concern now; the daemon doesn't
    // require one to open.
    let tmp = tempfile::tempdir().unwrap();
    Daemon::open(tmp.path().to_path_buf()).expect("open daemon in bare tempdir");
}

#[test]
fn deploy_writes_filesystem_and_db_row() {
    let tmp = tempfile::tempdir().unwrap();
    let ws_root = tmp.path().to_path_buf();
    let recipe_name = "fixture-ok";
    init_workspace(&ws_root, recipe_name, RECIPE);
    let daemon = Daemon::open(ws_root.clone()).unwrap();

    let catalog = catalog_for(RECIPE, &ws_root);
    let dv = daemon
        .deploy(recipe_name, RECIPE.to_string(), catalog.clone())
        .expect("deploy");
    assert_eq!(dv.version, 1);
    assert_eq!(dv.recipe_name, recipe_name);

    let v1_dir = ws_root
        .join(".forage")
        .join("deployments")
        .join(recipe_name)
        .join("v1");
    let recipe_path = v1_dir.join("recipe.forage");
    let catalog_path = v1_dir.join("catalog.json");
    assert!(recipe_path.exists(), "recipe.forage must be written");
    assert!(catalog_path.exists(), "catalog.json must be written");
    assert_eq!(std::fs::read_to_string(&recipe_path).unwrap(), RECIPE);

    let on_disk_catalog: SerializableCatalog =
        serde_json::from_str(&std::fs::read_to_string(&catalog_path).unwrap()).unwrap();
    assert_eq!(on_disk_catalog, catalog);

    let current = daemon.current_deployed(recipe_name).unwrap();
    assert_eq!(current.map(|c| c.version), Some(1));
}

#[test]
fn deploy_rejects_broken_source() {
    let tmp = tempfile::tempdir().unwrap();
    let ws_root = tmp.path().to_path_buf();
    let recipe_name = "broken";
    init_workspace(&ws_root, recipe_name, "recipe \"broken\"\nengine http\n");
    let daemon = Daemon::open(ws_root.clone()).unwrap();

    // Use an empty catalog — the parse failure short-circuits before
    // validation cares about it.
    let err = daemon
        .deploy(
            recipe_name,
            "for in {{ }}\n".to_string(),
            SerializableCatalog::default(),
        )
        .expect_err("parse failure rejects the deploy");
    assert!(matches!(err, DeployError::Parse(_)), "got {err:?}");

    let v1_dir = ws_root
        .join(".forage")
        .join("deployments")
        .join(recipe_name)
        .join("v1");
    assert!(!v1_dir.exists(), "no version directory must be written");
    assert!(daemon.current_deployed(recipe_name).unwrap().is_none());
}

#[test]
fn deploy_rejects_invalid_recipe() {
    let tmp = tempfile::tempdir().unwrap();
    let ws_root = tmp.path().to_path_buf();
    let recipe_name = "validates";
    init_workspace(&ws_root, recipe_name, RECIPE_REFERENCES_UNDECLARED_TYPE);
    let daemon = Daemon::open(ws_root.clone()).unwrap();

    let catalog = catalog_for(RECIPE_REFERENCES_UNDECLARED_TYPE, &ws_root);
    let err = daemon
        .deploy(
            recipe_name,
            RECIPE_REFERENCES_UNDECLARED_TYPE.to_string(),
            catalog,
        )
        .expect_err("validation failure rejects the deploy");
    assert!(matches!(err, DeployError::Validate(_)), "got {err:?}");

    let v1_dir = ws_root
        .join(".forage")
        .join("deployments")
        .join(recipe_name)
        .join("v1");
    assert!(
        !v1_dir.exists(),
        "no version directory must be written on validation failure"
    );
    assert!(daemon.current_deployed(recipe_name).unwrap().is_none());
}

#[test]
fn deploy_bumps_version() {
    let tmp = tempfile::tempdir().unwrap();
    let ws_root = tmp.path().to_path_buf();
    let recipe_name = "fixture-ok";
    init_workspace(&ws_root, recipe_name, RECIPE);
    let daemon = Daemon::open(ws_root.clone()).unwrap();
    let catalog = catalog_for(RECIPE, &ws_root);

    let v1 = daemon
        .deploy(recipe_name, RECIPE.to_string(), catalog.clone())
        .unwrap();
    let v2 = daemon
        .deploy(recipe_name, RECIPE.to_string(), catalog)
        .unwrap();
    assert_eq!(v1.version, 1);
    assert_eq!(v2.version, 2);

    let deployments_root = ws_root
        .join(".forage")
        .join("deployments")
        .join(recipe_name);
    assert!(deployments_root.join("v1").is_dir());
    assert!(deployments_root.join("v2").is_dir());

    let listed = daemon.deployed_versions(recipe_name).unwrap();
    assert_eq!(listed.len(), 2);
    // Newest first.
    assert_eq!(listed[0].version, 2);
    assert_eq!(listed[1].version, 1);
}

#[test]
fn deploy_updates_run_pointer_when_run_exists() {
    let tmp = tempfile::tempdir().unwrap();
    let ws_root = tmp.path().to_path_buf();
    let recipe_name = "fixture-ok";
    init_workspace(&ws_root, recipe_name, RECIPE);
    let daemon = Daemon::open(ws_root.clone()).unwrap();

    let cfg = RunConfig {
        cadence: Cadence::Manual,
        output: ws_root.join(".forage").join("data").join("ok.sqlite"),
        enabled: true,
    };
    let run = daemon.configure_run(recipe_name, cfg).unwrap();
    assert!(run.deployed_version.is_none());

    let catalog = catalog_for(RECIPE, &ws_root);
    daemon
        .deploy(recipe_name, RECIPE.to_string(), catalog)
        .unwrap();

    let refreshed = daemon.get_run(&run.id).unwrap().unwrap();
    assert_eq!(refreshed.deployed_version, Some(1));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_once_without_deployment_fails_cleanly() {
    let tmp = tempfile::tempdir().unwrap();
    let ws_root = tmp.path().to_path_buf();
    let recipe_name = "fixture-ok";
    init_workspace(&ws_root, recipe_name, RECIPE);
    let daemon = Daemon::open(ws_root.clone()).unwrap();

    let cfg = RunConfig {
        cadence: Cadence::Manual,
        output: ws_root.join(".forage").join("data").join("ok.sqlite"),
        enabled: true,
    };
    let run = daemon.configure_run(recipe_name, cfg).unwrap();

    let sr = daemon.trigger_run(&run.id).await.expect("trigger_run");
    assert_eq!(sr.outcome, Outcome::Fail);
    assert_eq!(sr.stall.as_deref(), Some("recipe not deployed"));
    // Short-circuit fired before any version was resolved.
    assert_eq!(sr.recipe_version, None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_once_uses_deployed_source() {
    // After deploy, edits to the on-disk draft must not affect the
    // run's emit counts — the daemon executes the frozen version,
    // not whatever's on disk at fire time.
    let tmp = tempfile::tempdir().unwrap();
    let ws_root = tmp.path().to_path_buf();
    let recipe_name = "fixture-ok";
    init_workspace(&ws_root, recipe_name, RECIPE);

    let mock = common::http_mock::server_returning_items(&[("a", 0.1), ("b", 0.2), ("c", 0.3)])
        .await;
    let recipe_path = ws_root.join(recipe_name).join("recipe.forage");
    let src = std::fs::read_to_string(&recipe_path).unwrap();
    let deployed_src = src.replace("https://example.test/items", &mock.url("/items"));
    std::fs::write(&recipe_path, &deployed_src).unwrap();

    let daemon = Daemon::open(ws_root.clone()).unwrap();
    let catalog = catalog_for(&deployed_src, &ws_root);
    daemon.deploy(recipe_name, deployed_src, catalog).unwrap();

    // Now mangle the draft so a re-read would parse but emit zero
    // records. If the scheduler reads from disk we'll see the count
    // collapse; if it reads from the deployed payload we see 3.
    let mangled_draft = format!(
        "recipe \"{recipe_name}\"\nengine http\n\
         type Item {{ id: String }}\n\
         step list {{\n    method \"GET\"\n    url    \"{}\"\n}}\n",
        mock.url("/items")
    );
    std::fs::write(&recipe_path, mangled_draft).unwrap();

    let cfg = RunConfig {
        cadence: Cadence::Manual,
        output: ws_root.join(".forage").join("data").join("ok.sqlite"),
        enabled: true,
    };
    let run = daemon.configure_run(recipe_name, cfg).unwrap();
    assert_eq!(run.deployed_version, Some(1));

    let sr = daemon.trigger_run(&run.id).await.expect("trigger_run");
    assert_eq!(sr.outcome, Outcome::Ok, "stall: {:?}", sr.stall);
    assert_eq!(sr.counts.get("Item").copied(), Some(3));
    // The row records which deployed version executed; without it,
    // count history goes incoherent across deploys.
    assert_eq!(sr.recipe_version, Some(1));
}

/// A stray `v<n>/` directory on disk with no matching `deployed_versions`
/// row is the documented recovery state when an FS write succeeded but
/// the SQLite txn rolled back. The next `deploy()` must bump past the
/// stray dir rather than overwriting it — the on-disk source from the
/// failed attempt stays visible to the user, and the new deploy lands
/// at the next version.
#[test]
fn deploy_skips_past_stray_version_directories() {
    let tmp = tempfile::tempdir().unwrap();
    let ws_root = tmp.path().to_path_buf();
    let recipe_name = "fixture-ok";
    init_workspace(&ws_root, recipe_name, RECIPE);
    let daemon = Daemon::open(ws_root.clone()).unwrap();

    // Plant a stray v1 directory before any real deploy lands. The
    // deployments dir lives at <ws_root>/.forage/deployments/<recipe_name>/.
    let stray = ws_root
        .join(".forage")
        .join("deployments")
        .join(recipe_name)
        .join("v1");
    std::fs::create_dir_all(&stray).unwrap();
    std::fs::write(stray.join("recipe.forage"), "STRAY").unwrap();

    let catalog = catalog_for(RECIPE, &ws_root);
    let dv = daemon
        .deploy(recipe_name, RECIPE.to_string(), catalog)
        .expect("deploy must succeed past the stray dir");
    assert_eq!(dv.version, 2, "must bump past stray v1, not overwrite it");

    // Stray dir is untouched — that's the whole point of the bump.
    assert_eq!(
        std::fs::read_to_string(stray.join("recipe.forage")).unwrap(),
        "STRAY"
    );

    // The new deploy materialized at v2 with the real content.
    let v2 = ws_root
        .join(".forage")
        .join("deployments")
        .join(recipe_name)
        .join("v2")
        .join("recipe.forage");
    assert_eq!(std::fs::read_to_string(&v2).unwrap(), RECIPE);
}

/// Two concurrent `deploy(recipe_name, ...)` calls must land at distinct
/// versions. Without the deploy lock, both would read the same
/// `latest_deployed_version` outside the txn and race on `fs::rename`
/// — one would land at v1, the other would either trip the
/// `(slug, version)` PRIMARY KEY or hit `ENOTEMPTY` on rename.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_deploys_land_at_distinct_versions() {
    let tmp = tempfile::tempdir().unwrap();
    let ws_root = tmp.path().to_path_buf();
    let recipe_name = "fixture-ok";
    init_workspace(&ws_root, recipe_name, RECIPE);
    let daemon = Daemon::open(ws_root.clone()).unwrap();
    let catalog = catalog_for(RECIPE, &ws_root);

    let d1 = daemon.clone();
    let d2 = daemon.clone();
    let cat1 = catalog.clone();
    let cat2 = catalog.clone();
    let name1 = recipe_name.to_string();
    let name2 = recipe_name.to_string();
    let src1 = RECIPE.to_string();
    let src2 = RECIPE.to_string();

    let (r1, r2) = tokio::join!(
        tokio::task::spawn_blocking(move || d1.deploy(&name1, src1, cat1)),
        tokio::task::spawn_blocking(move || d2.deploy(&name2, src2, cat2)),
    );
    let dv1 = r1.expect("join 1").expect("deploy 1");
    let dv2 = r2.expect("join 2").expect("deploy 2");

    let mut versions = [dv1.version, dv2.version];
    versions.sort();
    assert_eq!(versions, [1, 2], "two concurrent deploys must produce v1 and v2");

    let listed = daemon.deployed_versions(recipe_name).unwrap();
    assert_eq!(listed.len(), 2);
}


/// — `None` for the no-deployment short-circuit, `Some(v)` for runs
/// where the engine resolved a deployed version. Without this, the
/// column drops on the SELECT projection silently turn every row's
/// version into `None`, which corrupts the "which recipe shape
/// produced this row?" invariant.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scheduled_run_recipe_version_round_trips() {
    let tmp = tempfile::tempdir().unwrap();
    let ws_root = tmp.path().to_path_buf();
    let recipe_name = "fixture-ok";
    init_workspace(&ws_root, recipe_name, RECIPE);

    let mock = common::http_mock::server_returning_items(&[("a", 0.1), ("b", 0.2)]).await;
    let recipe_path = ws_root.join(recipe_name).join("recipe.forage");
    let src = std::fs::read_to_string(&recipe_path).unwrap();
    let deployed_src = src.replace("https://example.test/items", &mock.url("/items"));
    std::fs::write(&recipe_path, &deployed_src).unwrap();

    let daemon = Daemon::open(ws_root.clone()).unwrap();

    let cfg = RunConfig {
        cadence: Cadence::Manual,
        output: ws_root.join(".forage").join("data").join("ok.sqlite"),
        enabled: true,
    };
    let run = daemon.configure_run(recipe_name, cfg).unwrap();

    // First fire: no deployed version → row carries `recipe_version: None`.
    let pre_deploy = daemon.trigger_run(&run.id).await.expect("trigger pre-deploy");
    assert_eq!(pre_deploy.outcome, Outcome::Fail);
    assert_eq!(pre_deploy.recipe_version, None);

    // Deploy then fire again: row carries `recipe_version: Some(1)`.
    let catalog = catalog_for(&deployed_src, &ws_root);
    daemon.deploy(recipe_name, deployed_src, catalog).unwrap();
    let post_deploy = daemon
        .trigger_run(&run.id)
        .await
        .expect("trigger post-deploy");
    assert_eq!(post_deploy.outcome, Outcome::Ok, "stall: {:?}", post_deploy.stall);
    assert_eq!(post_deploy.recipe_version, Some(1));

    // Read back through the same query path Studio uses. Both
    // versions of the field must survive the SQL round-trip.
    let history = daemon.list_scheduled_runs(&run.id, 10, None).unwrap();
    assert_eq!(history.len(), 2);
    // Newest first.
    let none_count = history
        .iter()
        .filter(|sr| sr.recipe_version.is_none())
        .count();
    let some_count = history
        .iter()
        .filter(|sr| sr.recipe_version == Some(1))
        .count();
    assert_eq!(none_count, 1, "one row should round-trip as None");
    assert_eq!(some_count, 1, "one row should round-trip as Some(1)");

    // Pin the exact rows so a regression that always returns Some/None
    // both gets caught — counts above could pass by coincidence.
    let by_id: std::collections::HashMap<String, Option<u32>> = history
        .iter()
        .map(|sr| (sr.id.clone(), sr.recipe_version))
        .collect();
    assert_eq!(by_id.get(&pre_deploy.id).copied(), Some(None));
    assert_eq!(by_id.get(&post_deploy.id).copied(), Some(Some(1)));
}

/// A v1 daemon DB (schema_version = 1) opens cleanly under the v2
/// migration: the new `deployed_versions` table is created and the
/// `deployed_version` column is added to `runs` without losing prior
/// rows.
#[test]
fn opening_a_v1_database_runs_the_migration() {
    use rusqlite::Connection;

    let tmp = tempfile::tempdir().unwrap();
    let ws_root = tmp.path().to_path_buf();
    let daemon_dir = ws_root.join(".forage");
    std::fs::create_dir_all(&daemon_dir).unwrap();
    let db_path = daemon_dir.join("daemon.sqlite");

    {
        // Hand-build a v1 schema. Mirrors the legacy migration in
        // `db::apply_migrations` — the contract under test is that
        // opening this DB through v2 doesn't drop or corrupt the row.
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE _meta (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            INSERT INTO _meta(key, value) VALUES ('schema_version', '1');
            CREATE TABLE runs (
                id              TEXT PRIMARY KEY,
                recipe_slug     TEXT NOT NULL,
                workspace_root  TEXT NOT NULL,
                enabled         INTEGER NOT NULL,
                cadence_json    TEXT NOT NULL,
                output_path     TEXT NOT NULL,
                health          TEXT NOT NULL,
                next_run        INTEGER
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
                FOREIGN KEY (run_id) REFERENCES runs(id) ON DELETE CASCADE
            );
            CREATE INDEX scheduled_runs_run_id_at ON scheduled_runs(run_id, at DESC);
            "#,
        )
        .unwrap();
        conn.execute(
            "INSERT INTO runs(id, recipe_slug, workspace_root, enabled, cadence_json, output_path, health, next_run)
             VALUES ('legacy', 'old-slug', '/tmp', 1, '{\"kind\":\"manual\"}', '/tmp/x.sqlite', 'unknown', NULL)",
            [],
        )
        .unwrap();
    }

    let daemon = Daemon::open(ws_root.clone()).expect("open v1 db under v2");
    // The migration must have added the column without dropping the
    // legacy row.
    let runs = daemon.list_runs().expect("list runs after migration");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].id, "legacy");
    assert!(runs[0].deployed_version.is_none());

    // The new table is queryable (returns empty for a never-deployed
    // slug). If the migration didn't add it, this would fail at the
    // SQL prepare step.
    assert!(daemon.deployed_versions("old-slug").unwrap().is_empty());

    // `scheduled_runs.recipe_version` was added in the same v2 block —
    // querying through the standard path must succeed against a
    // migrated DB (zero rows here, but the projection has to bind).
    assert!(daemon.list_scheduled_runs("legacy", 10, None).unwrap().is_empty());
}
