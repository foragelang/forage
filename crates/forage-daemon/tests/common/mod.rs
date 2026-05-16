//! Shared helpers for daemon integration tests. Drops the workspace
//! marker + per-recipe directory under a tempdir, wires up a wiremock
//! HTTP fixture, exposes a tweakable clock for the scheduler tests.
//!
//! Each `tests/*.rs` is its own binary, so any single test may use only
//! a subset of these helpers. The `dead_code` allow suppresses
//! per-binary "unused" warnings; the workspace as a whole exercises
//! everything here.
#![allow(dead_code)]

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use forage_daemon::{Clock, Daemon, DeployedVersion};

/// Plant a workspace marker plus a flat-shape recipe file at
/// `<ws_root>/<name>.forage`. `name` is the on-disk basename; the
/// recipe's identity is whatever the source declares in its header.
pub fn init_workspace(ws_root: &Path, name: &str, recipe_source: &str) {
    std::fs::create_dir_all(ws_root).unwrap();
    std::fs::write(
        ws_root.join("forage.toml"),
        // Minimal valid manifest: required fields present, no name.
        "description = \"\"\ncategory = \"\"\ntags = []\n",
    )
    .unwrap();
    std::fs::write(ws_root.join(format!("{name}.forage")), recipe_source).unwrap();
}

/// Link the recipe at `ws_root/<name>.forage` through the workspace
/// and deploy the resulting closure. Returns the `DeployedVersion` so
/// tests can pin the expected version number.
pub fn deploy_disk_recipe(daemon: &Daemon, ws_root: &Path, name: &str) -> DeployedVersion {
    let workspace = forage_core::load(ws_root).expect("load workspace");
    let outcome = forage_core::link(&workspace, name).expect("link");
    assert!(
        !outcome.report.has_errors(),
        "link errors for {name}: {:?}",
        outcome
            .report
            .issues
            .iter()
            .map(|i| (i.code, &i.message))
            .collect::<Vec<_>>(),
    );
    let module = outcome.module.expect("linker produces module");
    daemon.deploy(name, module).expect("deploy")
}

pub fn set_secret(name: &str, value: &str) {
    // SAFETY: tests run with --test-threads=1 inside Cargo, and these
    // helpers are only used by one test at a time. Real concurrent
    // env-var mutation is undefined behavior across libc.
    unsafe { std::env::set_var(format!("FORAGE_SECRET_{}", name.to_uppercase()), value) };
}

pub mod http_mock {
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    pub struct Server(pub MockServer);
    impl Server {
        pub fn url(&self, path: &str) -> String {
            format!("{}{}", self.0.uri(), path)
        }
    }

    pub async fn server_returning_items(items: &[(&str, f64)]) -> Server {
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "items": items
                .iter()
                .map(|(id, weight)| serde_json::json!({ "id": id, "weight": weight }))
                .collect::<Vec<_>>(),
        });
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;
        Server(server)
    }

    pub async fn server_failing(status: u16) -> Server {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(status).set_body_string("nope"))
            .mount(&server)
            .await;
        Server(server)
    }
}

/// A clock that can be advanced by `set_now` / `advance`, for
/// deterministic scheduler tests. `sleep_until_ms` polls the stored
/// time on a tight interval — every `set_now` / `advance` is picked up
/// within a few ms, with no lost-wakeup window.
pub struct StubClock {
    now: AtomicI64,
}

impl StubClock {
    pub fn new(initial_ms: i64) -> Arc<Self> {
        Arc::new(Self {
            now: AtomicI64::new(initial_ms),
        })
    }

    pub fn set_now(&self, ms: i64) {
        self.now.store(ms, Ordering::SeqCst);
    }

    pub fn advance(&self, by_ms: i64) {
        self.now.fetch_add(by_ms, Ordering::SeqCst);
    }
}

#[async_trait::async_trait]
impl Clock for StubClock {
    fn now_ms(&self) -> i64 {
        self.now.load(Ordering::SeqCst)
    }
    async fn sleep_until_ms(&self, deadline_ms: i64) {
        // Polling rather than a Notify avoids any lost-wakeup window
        // between the test thread's `set_now` and the scheduler task's
        // wait registration. 2ms is fine for tests — we are not
        // simulating microsecond-scale time.
        while self.now_ms() < deadline_ms {
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        }
    }
}
