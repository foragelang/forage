//! `forage publish <recipe-name>` keys the published artifact on the
//! recipe header name. The workspace's `forage.toml` contributes only
//! the author segment; the slug-portion is ignored in favor of the
//! header name.

use assert_cmd::Command;
use forage_hub::{PublishRequest, PublishTypeRequest};
use std::fs;
use std::sync::Arc;
use tokio::sync::Mutex;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const MANIFEST: &str = "name = \"alice/legacy-slug\"\n\
description = \"test pkg\"\n\
category = \"scrape\"\n\
tags = []\n";

/// `wiremock` responder that captures the request body so the test can
/// inspect what the CLI POSTed.
struct CapturingResponder {
    captured: Arc<Mutex<Option<Vec<u8>>>>,
    response: ResponseTemplate,
}

impl Respond for CapturingResponder {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        // The mock server runs in a tokio runtime; lock-and-write
        // can't await, so use `try_lock` and panic on contention —
        // every test has exactly one call in flight per endpoint.
        let mut guard = self.captured.try_lock().expect("captured already locked");
        *guard = Some(request.body.clone());
        self.response.clone()
    }
}

/// `forage publish amazon-products` finds the recipe by header name —
/// not by file basename, not by the slug portion of `forage.toml.name`
/// — extracts every `share`d type into its own publishable unit, posts
/// the types first under `@<author>/<TypeName>`, then posts the recipe
/// pinning the resolved type versions. The author segment of `name`
/// IS used; the slug segment is decorative.
#[tokio::test]
async fn publish_keys_artifact_on_recipe_header_name() {
    let server = MockServer::start().await;
    let captured_recipe: Arc<Mutex<Option<Vec<u8>>>> = Arc::new(Mutex::new(None));
    let captured_type: Arc<Mutex<Option<Vec<u8>>>> = Arc::new(Mutex::new(None));

    // First publish: the type doesn't exist yet, so the GET returns 404.
    Mock::given(method("GET"))
        .and(path("/v1/types/alice/Dispensary"))
        .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
            "error": { "code": "not_found", "message": "unknown type" }
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/types/alice/Dispensary/versions"))
        .respond_with(CapturingResponder {
            captured: Arc::clone(&captured_type),
            response: ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "author": "alice",
                "name": "Dispensary",
                "version": 1,
                "latest_version": 1,
                "deduped": false,
            })),
        })
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/packages/alice/amazon-products/versions"))
        .respond_with(CapturingResponder {
            captured: Arc::clone(&captured_recipe),
            response: ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "author": "alice",
                "slug": "amazon-products",
                "version": 1,
                "latest_version": 1,
            })),
        })
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path();
    fs::write(ws.join("forage.toml"), MANIFEST).unwrap();
    // The header name (`amazon-products`) is what `forage publish`
    // resolves and sends, NOT the file basename (`scrape-amazon`).
    fs::write(
        ws.join("scrape-amazon.forage"),
        "recipe \"amazon-products\"\nengine http\n",
    )
    .unwrap();
    // Sibling with a `share`d declaration: must publish as a separate
    // type artifact and ride into the recipe as a `type_refs` entry.
    fs::write(
        ws.join("cannabis.forage"),
        "share type Dispensary { id: String }\n",
    )
    .unwrap();
    // Sibling with only file-local declarations: must NOT ship.
    fs::write(
        ws.join("local-only.forage"),
        "type Private { id: String }\n",
    )
    .unwrap();
    // Per-recipe fixtures + snapshot: must ride along.
    fs::create_dir_all(ws.join("_fixtures")).unwrap();
    fs::write(
        ws.join("_fixtures").join("amazon-products.jsonl"),
        "{\"kind\":\"http\",\"url\":\"https://example.test\",\"method\":\"GET\",\"status\":200,\"body\":\"{}\"}\n",
    )
    .unwrap();
    fs::create_dir_all(ws.join("_snapshots")).unwrap();
    fs::write(
        ws.join("_snapshots").join("amazon-products.json"),
        "{\"records\":[],\"diagnostic\":{\"stall_reason\":null,\"unmet_expectations\":[],\
         \"unfired_capture_rules\":[],\"unmatched_captures\":[],\"unhandled_affordances\":[]},\
         \"record_types\":{}}",
    )
    .unwrap();

    Command::cargo_bin("forage")
        .unwrap()
        .current_dir(ws)
        .arg("publish")
        .arg("amazon-products")
        .arg("--publish")
        .arg("--hub")
        .arg(server.uri())
        .arg("--token")
        .arg("test-token")
        .assert()
        .success()
        .stdout(predicates::str::contains("alice/amazon-products"));

    // The type POST received the standalone `share type Dispensary`
    // source as its own artifact.
    let type_body = captured_type
        .lock()
        .await
        .clone()
        .expect("type publish must have POSTed");
    let type_req: PublishTypeRequest =
        serde_json::from_slice(&type_body).expect("type publish body is JSON");
    assert!(
        type_req.source.starts_with("share type Dispensary"),
        "type publish source must be the standalone share-type fragment: {:?}",
        type_req.source,
    );

    let recipe_body = captured_recipe
        .lock()
        .await
        .clone()
        .expect("recipe publish must have POSTed");
    let req: PublishRequest = serde_json::from_slice(&recipe_body).expect("publish body is JSON");
    assert!(
        req.recipe.contains("recipe \"amazon-products\""),
        "POSTed recipe body must be the resolved file's content",
    );
    assert_eq!(req.type_refs.len(), 1);
    assert_eq!(req.type_refs[0].author, "alice");
    assert_eq!(req.type_refs[0].name, "Dispensary");
    assert_eq!(req.type_refs[0].version, 1);
    assert!(
        !req.recipe.contains("Private"),
        "file-local types do not appear in the recipe body's `type_refs` or in the source",
    );
    assert_eq!(req.fixtures.len(), 1, "_fixtures content rides along");
    assert!(req.snapshot.is_some(), "_snapshots content rides along");
}

/// Dry-run prints the would-POST artifact and exits zero without
/// hitting the network. `forage publish` resolves the recipe by
/// header name, not by workspace directory.
#[tokio::test]
async fn publish_dry_run_resolves_recipe_by_name_without_post() {
    let server = MockServer::start().await;
    // No mock for the publish endpoint — a real POST attempt would
    // fail with a connection error AND surface a wiremock-unmatched
    // warning, both of which would fail the test.

    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path();
    fs::write(ws.join("forage.toml"), MANIFEST).unwrap();
    fs::write(
        ws.join("scrape-amazon.forage"),
        "recipe \"amazon-products\"\nengine http\n",
    )
    .unwrap();

    Command::cargo_bin("forage")
        .unwrap()
        .current_dir(ws)
        .arg("publish")
        .arg("amazon-products")
        .arg("--hub")
        .arg(server.uri())
        .assert()
        .success()
        .stdout(predicates::str::contains("dry-run"))
        .stdout(predicates::str::contains("alice/amazon-products"));
}
