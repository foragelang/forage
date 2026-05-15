//! CLI integration: `forage sync` and `forage fork` shells out to the
//! shared `forage-hub::operations` surface. The subprocess test
//! validates the wire-shape end-to-end against a wiremock-faked hub,
//! confirming the subcommands, args, and exit codes the user sees.

use assert_cmd::Command;
use forage_hub::{
    PackageFixture, PackageMetadata, PackageVersion, TypeFieldAlignment, TypeRef, TypeVersion,
};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn artifact(author: &str, slug: &str) -> PackageVersion {
    PackageVersion {
        author: author.into(),
        slug: slug.into(),
        version: 1,
        recipe: format!(
            "recipe \"{slug}\"\nengine http\nstep s {{ method \"GET\" url \"https://example.test\" }}\n"
        ),
        type_refs: vec![TypeRef {
            author: author.into(),
            name: "Shared".into(),
            version: 1,
        }],
        input_type_refs: Vec::new(),
        output_type_refs: Vec::new(),
        fixtures: vec![PackageFixture {
            name: "captures.jsonl".into(),
            content: "{\"x\":1}\n".into(),
        }],
        snapshot: None,
        base_version: None,
        published_at: 0,
        published_by: author.into(),
    }
}

fn type_version(author: &str, name: &str) -> TypeVersion {
    TypeVersion {
        author: author.into(),
        name: name.into(),
        version: 1,
        source: format!("share type {name} {{\n    id: String\n}}\n"),
        alignments: Vec::new(),
        field_alignments: vec![TypeFieldAlignment {
            field: "id".into(),
            alignment: None,
        }],
        base_version: None,
        published_at: 0,
        published_by: author.into(),
    }
}

fn package_meta(author: &str, slug: &str) -> PackageMetadata {
    PackageMetadata {
        author: author.into(),
        slug: slug.into(),
        description: "test pkg".into(),
        category: "scrape".into(),
        tags: vec![],
        forked_from: None,
        created_at: 0,
        latest_version: 1,
        stars: 0,
        downloads: 0,
        fork_count: 0,
        owner_login: author.into(),
    }
}

#[tokio::test]
async fn forage_sync_materializes_recipe_and_types_in_cwd() {
    let server = MockServer::start().await;
    let art = artifact("alice", "zen-leaf");
    let tv = type_version("alice", "Shared");
    Mock::given(method("GET"))
        .and(path("/v1/packages/alice/zen-leaf"))
        .respond_with(ResponseTemplate::new(200).set_body_json(package_meta("alice", "zen-leaf")))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/packages/alice/zen-leaf/versions/latest"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&art))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/types/alice/Shared/versions/1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&tv))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/packages/alice/zen-leaf/downloads"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "downloads": 1 })))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path();
    let cache = tempfile::tempdir().unwrap();

    Command::cargo_bin("forage")
        .unwrap()
        .env("FORAGE_HUB_CACHE", cache.path())
        .arg("sync")
        .arg("@alice/zen-leaf")
        .arg(ws)
        .arg("--hub")
        .arg(server.uri())
        .assert()
        .success()
        .stdout(predicates::str::contains("@alice/zen-leaf@v1"));

    assert!(ws.join("zen-leaf.forage").is_file());
    assert!(ws.join(".forage").join("sync").join("zen-leaf.json").is_file());
    assert!(ws.join("Shared.forage").is_file());
    // Type also lands in the hub-type cache so the workspace loader's
    // lockfile resolution can pick it up.
    assert!(
        cache
            .path()
            .join("types")
            .join("alice")
            .join("Shared")
            .join("1.forage")
            .is_file(),
    );
}

#[tokio::test]
async fn forage_sync_with_explicit_version_arg() {
    let server = MockServer::start().await;
    let mut art = artifact("alice", "zen-leaf");
    art.version = 7;
    let tv = type_version("alice", "Shared");
    Mock::given(method("GET"))
        .and(path("/v1/packages/alice/zen-leaf"))
        .respond_with(ResponseTemplate::new(200).set_body_json(package_meta("alice", "zen-leaf")))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/packages/alice/zen-leaf/versions/7"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&art))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/types/alice/Shared/versions/1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&tv))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/packages/alice/zen-leaf/downloads"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "downloads": 1 })))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path();
    let cache = tempfile::tempdir().unwrap();

    Command::cargo_bin("forage")
        .unwrap()
        .env("FORAGE_HUB_CACHE", cache.path())
        .arg("sync")
        .arg("@alice/zen-leaf")
        .arg(ws)
        .arg("--version")
        .arg("7")
        .arg("--hub")
        .arg(server.uri())
        .assert()
        .success()
        .stdout(predicates::str::contains("@alice/zen-leaf@v7"));
}

#[tokio::test]
async fn forage_sync_rejects_invalid_spec() {
    let tmp = tempfile::tempdir().unwrap();
    Command::cargo_bin("forage")
        .unwrap()
        .arg("sync")
        .arg("bare-slug-no-author")
        .arg(tmp.path())
        .assert()
        .failure()
        .stderr(predicates::str::contains("expected `@author/slug`"));
}

/// `forage sync alice/zen-leaf` (without the leading `@`) must
/// round-trip identically to `forage sync @alice/zen-leaf`. The CLI
/// accepts either form because hub URLs paste in without the `@`;
/// regressing the bare form would surface only when a user copies
/// a slug out of a URL bar.
#[tokio::test]
async fn forage_sync_accepts_bare_slug_without_at_prefix() {
    let server = MockServer::start().await;
    let art = artifact("alice", "zen-leaf");
    let tv = type_version("alice", "Shared");
    Mock::given(method("GET"))
        .and(path("/v1/packages/alice/zen-leaf"))
        .respond_with(ResponseTemplate::new(200).set_body_json(package_meta("alice", "zen-leaf")))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/packages/alice/zen-leaf/versions/latest"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&art))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/types/alice/Shared/versions/1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&tv))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/packages/alice/zen-leaf/downloads"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "downloads": 1 })))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path();
    let cache = tempfile::tempdir().unwrap();

    Command::cargo_bin("forage")
        .unwrap()
        .env("FORAGE_HUB_CACHE", cache.path())
        .arg("sync")
        .arg("alice/zen-leaf") // no `@` prefix
        .arg(ws)
        .arg("--hub")
        .arg(server.uri())
        .assert()
        .success()
        .stdout(predicates::str::contains("@alice/zen-leaf@v1"));

    assert!(ws.join("zen-leaf.forage").is_file());
}
