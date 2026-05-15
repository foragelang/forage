//! End-to-end roundtrips against a wiremock-faked hub-api:
//!
//! - `sync_from_hub` materializes a `PackageVersion` into the workspace,
//!   writing recipe + decls + fixtures + snapshot + the sidecar, and
//!   bumps the download counter.
//! - `assemble_publish_request` walks the on-disk shape back into the
//!   wire artifact, picking up `base_version` from the sidecar.
//! - 409 stale-base from the publish endpoint surfaces as
//!   `HubError::StaleBase` with the version numbers preserved.
//! - `fork_from_hub` POSTs the fork endpoint and then syncs the new
//!   fork's v1 artifact into the destination workspace.

use forage_hub::{
    ForageMeta, HubClient, HubError, PackageFile, PackageFixture, PackageMetadata, PackageVersion,
    assemble_publish_request, fork_from_hub, publish_from_workspace, read_meta, sync_from_hub,
};
use indexmap::IndexMap;
use serde_json::json;
use wiremock::matchers::{body_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn artifact_v1(author: &str, slug: &str) -> PackageVersion {
    PackageVersion {
        author: author.into(),
        slug: slug.into(),
        version: 1,
        recipe: format!(
            "recipe \"{slug}\"\nengine http\n\nstep s {{ method \"GET\" url \"https://example.test\" }}\n"
        ),
        decls: vec![PackageFile {
            name: "shared.forage".into(),
            source: "type Shared { id: String }\n".into(),
        }],
        fixtures: vec![PackageFixture {
            name: "captures.jsonl".into(),
            content: "{\"kind\":\"http\",\"url\":\"https://example.test\",\"method\":\"GET\",\"status\":200,\"body\":\"{}\"}\n".into(),
        }],
        snapshot: None,
        base_version: None,
        published_at: 1_700_000_000_000,
        published_by: author.into(),
    }
}

fn package_meta(author: &str, slug: &str) -> PackageMetadata {
    PackageMetadata {
        author: author.into(),
        slug: slug.into(),
        description: "test pkg".into(),
        category: "scrape".into(),
        tags: vec!["test".into()],
        forked_from: None,
        created_at: 0,
        latest_version: 1,
        stars: 0,
        downloads: 0,
        fork_count: 0,
        owner_login: author.into(),
    }
}

/// Mount the GET endpoints `sync_from_hub` calls on the fake hub.
async fn mount_read_package(server: &MockServer, art: &PackageVersion) {
    let author = art.author.clone();
    let slug = art.slug.clone();
    Mock::given(method("GET"))
        .and(path(format!("/v1/packages/{author}/{slug}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(package_meta(&author, &slug)))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/packages/{author}/{slug}/versions/latest")))
        .respond_with(ResponseTemplate::new(200).set_body_json(art))
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("/v1/packages/{author}/{slug}/downloads")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "downloads": 1 })))
        .mount(server)
        .await;
}

#[tokio::test]
async fn sync_materializes_workspace_with_sidecar() {
    let server = MockServer::start().await;
    let art = artifact_v1("alice", "zen-leaf");
    mount_read_package(&server, &art).await;

    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path();
    let client = HubClient::new(server.uri());
    let outcome = sync_from_hub(&client, ws, "alice", "zen-leaf", None)
        .await
        .expect("sync should succeed");
    assert_eq!(outcome.version, 1);
    assert_eq!(outcome.meta.author, "alice");
    assert_eq!(outcome.meta.slug, "zen-leaf");
    assert_eq!(outcome.meta.base_version, 1);

    // Recipe + decls + fixtures land on disk where the workspace
    // loader expects them.
    assert!(ws.join("zen-leaf").join("recipe.forage").is_file());
    assert!(ws.join("shared.forage").is_file());
    assert!(
        ws.join("zen-leaf")
            .join("fixtures")
            .join("captures.jsonl")
            .is_file()
    );

    // The sidecar carries the publish-back base_version.
    let meta = read_meta(&ws.join("zen-leaf")).unwrap().unwrap();
    assert_eq!(meta.base_version, 1);
    assert_eq!(meta.origin, "@alice/zen-leaf@v1");
}

#[tokio::test]
async fn publish_succeeds_when_base_matches() {
    let server = MockServer::start().await;
    let author = "alice";
    let slug = "zen-leaf";
    Mock::given(method("POST"))
        .and(path(format!("/v1/packages/{author}/{slug}/versions")))
        .and(header("authorization", "Bearer test-token"))
        .and(body_json(json!({
            "description": "test pkg",
            "category": "scrape",
            "tags": ["test"],
            "recipe": "recipe \"zen-leaf\"\nengine http\nstep s { method \"GET\" url \"x\" }\n",
            "decls": [],
            "fixtures": [],
            "snapshot": null,
            "base_version": 1,
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "author": author,
            "slug": slug,
            "version": 2,
            "latest_version": 2,
        })))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path();
    std::fs::create_dir_all(ws.join(slug)).unwrap();
    std::fs::write(
        ws.join(slug).join("recipe.forage"),
        "recipe \"zen-leaf\"\nengine http\nstep s { method \"GET\" url \"x\" }\n",
    )
    .unwrap();
    // Sidecar carries the synced base version (1); the publish path
    // should send it as `base_version: 1`.
    forage_hub::write_meta(
        &ws.join(slug),
        &ForageMeta {
            origin: ForageMeta::pretty_origin(author, slug, 1),
            author: author.into(),
            slug: slug.into(),
            base_version: 1,
            forked_from: None,
        },
    )
    .unwrap();

    let client = HubClient::new(server.uri()).with_token("test-token");
    let resp = publish_from_workspace(
        &client,
        ws,
        author,
        slug,
        "test pkg".into(),
        "scrape".into(),
        vec!["test".into()],
    )
    .await
    .expect("publish should succeed");
    assert_eq!(resp.version, 2);
    assert_eq!(resp.latest_version, 2);

    // Sidecar reflects the new base after a successful publish.
    let meta = forage_hub::read_meta(&ws.join(slug)).unwrap().unwrap();
    assert_eq!(meta.base_version, 2);
    assert_eq!(meta.origin, "@alice/zen-leaf@v2");
}

#[tokio::test]
async fn publish_surfaces_stale_base_409() {
    let server = MockServer::start().await;
    let author = "alice";
    let slug = "zen-leaf";
    Mock::given(method("POST"))
        .and(path(format!("/v1/packages/{author}/{slug}/versions")))
        .respond_with(ResponseTemplate::new(409).set_body_json(json!({
            "error": {
                "code": "stale_base",
                "message": "base is stale, rebase to v5 and retry",
                "latest_version": 5,
                "your_base": 3,
            }
        })))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path();
    std::fs::create_dir_all(ws.join(slug)).unwrap();
    std::fs::write(
        ws.join(slug).join("recipe.forage"),
        "recipe \"zen-leaf\"\nengine http\nstep s { method \"GET\" url \"x\" }\n",
    )
    .unwrap();
    forage_hub::write_meta(
        &ws.join(slug),
        &ForageMeta {
            origin: ForageMeta::pretty_origin(author, slug, 3),
            author: author.into(),
            slug: slug.into(),
            base_version: 3,
            forked_from: None,
        },
    )
    .unwrap();

    let client = HubClient::new(server.uri()).with_token("test-token");
    let err = publish_from_workspace(
        &client,
        ws,
        author,
        slug,
        "test pkg".into(),
        "scrape".into(),
        vec![],
    )
    .await
    .expect_err("must fail with StaleBase");
    match err {
        HubError::StaleBase {
            latest_version,
            your_base,
            message,
        } => {
            assert_eq!(latest_version, 5);
            assert_eq!(your_base, Some(3));
            assert!(message.contains("rebase"), "message: {message}");
        }
        other => panic!("expected StaleBase, got {other:?}"),
    }
}

#[tokio::test]
async fn fork_then_sync_round_trip() {
    let server = MockServer::start().await;
    let upstream_author = "alice";
    let upstream_slug = "zen-leaf";
    let fork_author = "bob";

    // Fork response carries the new package's metadata; the fork
    // operation then GETs the v1 artifact + the package metadata to
    // populate the sidecar's forked_from.
    Mock::given(method("POST"))
        .and(path(format!(
            "/v1/packages/{upstream_author}/{upstream_slug}/fork"
        )))
        .respond_with(ResponseTemplate::new(201).set_body_json({
            let mut m = package_meta(fork_author, upstream_slug);
            m.forked_from = Some(forage_hub::ForkedFrom {
                author: upstream_author.into(),
                slug: upstream_slug.into(),
                version: 1,
            });
            m
        }))
        .mount(&server)
        .await;

    let mut art = artifact_v1(fork_author, upstream_slug);
    art.recipe = format!(
        "recipe \"{upstream_slug}\"\nengine http\nstep s {{ method \"GET\" url \"x\" }}\n"
    );
    // The fork creates a v1 under @bob/zen-leaf — the sync path uses
    // the versioned endpoint (not /latest) because sync_from_hub is
    // called with Some(1) when fork_from_hub dispatches.
    Mock::given(method("GET"))
        .and(path(format!("/v1/packages/{fork_author}/{upstream_slug}/versions/1")))
        .respond_with(ResponseTemplate::new(200).set_body_json(&art))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/packages/{fork_author}/{upstream_slug}")))
        .respond_with(ResponseTemplate::new(200).set_body_json({
            let mut m = package_meta(fork_author, upstream_slug);
            m.forked_from = Some(forage_hub::ForkedFrom {
                author: upstream_author.into(),
                slug: upstream_slug.into(),
                version: 1,
            });
            m
        }))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("/v1/packages/{fork_author}/{upstream_slug}/downloads")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "downloads": 1 })))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path();
    let client = HubClient::new(server.uri()).with_token("test-token");
    let outcome = fork_from_hub(&client, ws, upstream_author, upstream_slug, None)
        .await
        .expect("fork should succeed");
    assert_eq!(outcome.meta.author, fork_author);
    assert_eq!(outcome.meta.slug, upstream_slug);
    assert_eq!(outcome.meta.base_version, 1);
    assert!(outcome.meta.forked_from.is_some());

    let meta = read_meta(&ws.join(upstream_slug)).unwrap().unwrap();
    assert_eq!(
        meta.forked_from.as_ref().map(|f| f.author.as_str()),
        Some(upstream_author)
    );
}

#[tokio::test]
async fn assemble_request_matches_disk_state() {
    // Walks the assembly path directly: write recipe + decls +
    // fixtures + snapshot + sidecar, then assert the request shape.
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path();
    let slug = "zen-leaf";
    std::fs::create_dir_all(ws.join(slug).join("fixtures")).unwrap();
    std::fs::write(
        ws.join(slug).join("recipe.forage"),
        "recipe \"zen-leaf\"\nengine http\nstep s { method \"GET\" url \"x\" }\n",
    )
    .unwrap();
    std::fs::write(ws.join("shared.forage"), "type Shared { id: String }\n").unwrap();
    std::fs::write(
        ws.join(slug).join("fixtures").join("captures.jsonl"),
        "{\"line\":1}\n",
    )
    .unwrap();
    // Drop a forage_core::Snapshot on disk so the assembly converts
    // it to the wire shape.
    let core_snapshot = forage_core::Snapshot {
        records: vec![forage_core::Record {
            id: "rec-0".into(),
            type_name: "Item".into(),
            fields: IndexMap::new(),
        }],
        diagnostic: forage_core::DiagnosticReport::default(),
    };
    std::fs::write(
        ws.join(slug).join("snapshot.json"),
        serde_json::to_string(&core_snapshot).unwrap(),
    )
    .unwrap();

    forage_hub::write_meta(
        &ws.join(slug),
        &ForageMeta {
            origin: ForageMeta::pretty_origin("alice", slug, 2),
            author: "alice".into(),
            slug: slug.into(),
            base_version: 2,
            forked_from: None,
        },
    )
    .unwrap();

    let req = assemble_publish_request(
        ws,
        slug,
        "test pkg".into(),
        "scrape".into(),
        vec!["test".into()],
    )
    .unwrap();
    assert!(req.recipe.contains("zen-leaf"));
    assert_eq!(req.base_version, Some(2));
    assert!(req.decls.iter().any(|d| d.name == "shared.forage"));
    assert!(req.fixtures.iter().any(|f| f.name == "captures.jsonl"));
    let snap = req.snapshot.expect("snapshot should round-trip");
    assert_eq!(snap.counts.get("Item").copied(), Some(1));
    assert_eq!(snap.records.get("Item").map(|v| v.len()), Some(1));
}

#[tokio::test]
async fn sync_refuses_to_clobber_local_recipe() {
    // A workspace that already has `<slug>/recipe.forage` but no
    // sidecar means local edits exist that the sync would overwrite.
    // sync_from_hub returns an error instead of clobbering.
    let server = MockServer::start().await;
    let art = artifact_v1("alice", "zen-leaf");
    mount_read_package(&server, &art).await;

    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path();
    std::fs::create_dir_all(ws.join("zen-leaf")).unwrap();
    std::fs::write(
        ws.join("zen-leaf").join("recipe.forage"),
        "local edits not from hub\n",
    )
    .unwrap();

    let client = HubClient::new(server.uri());
    let err = sync_from_hub(&client, ws, "alice", "zen-leaf", None)
        .await
        .expect_err("must refuse to clobber local recipe");
    assert!(
        format!("{err}").contains("already holds local recipe files"),
        "unexpected error: {err}"
    );
    // The local file is untouched.
    let body = std::fs::read_to_string(ws.join("zen-leaf").join("recipe.forage")).unwrap();
    assert!(body.contains("local edits"));
}

#[tokio::test]
async fn sync_into_same_workspace_at_higher_version_succeeds() {
    // Drop a sidecar pinning base_version=1, then sync a version-3
    // artifact. The sidecar's base_version is older than the
    // incoming artifact's version, so the sync succeeds and
    // overwrites — the guard only fires when the existing sidecar
    // already covers the version being pulled.
    let server = MockServer::start().await;
    let mut v3 = artifact_v1("alice", "zen-leaf");
    v3.version = 3;
    v3.recipe = "recipe \"zen-leaf\"\nengine http\nstep s_new { method \"GET\" url \"https://example.test/new\" }\n".into();
    Mock::given(method("GET"))
        .and(path("/v1/packages/alice/zen-leaf"))
        .respond_with(ResponseTemplate::new(200).set_body_json(package_meta("alice", "zen-leaf")))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/packages/alice/zen-leaf/versions/latest"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&v3))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/packages/alice/zen-leaf/downloads"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "downloads": 2 })))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path();
    // Stage a prior sidecar at v1 to mimic a previous sync.
    std::fs::create_dir_all(ws.join("zen-leaf")).unwrap();
    std::fs::write(
        ws.join("zen-leaf").join("recipe.forage"),
        "recipe \"zen-leaf\"\nengine http\nstep s_old { method \"GET\" url \"x\" }\n",
    )
    .unwrap();
    forage_hub::write_meta(
        &ws.join("zen-leaf"),
        &ForageMeta {
            origin: ForageMeta::pretty_origin("alice", "zen-leaf", 1),
            author: "alice".into(),
            slug: "zen-leaf".into(),
            base_version: 1,
            forked_from: None,
        },
    )
    .unwrap();

    let client = HubClient::new(server.uri());
    let outcome = sync_from_hub(&client, ws, "alice", "zen-leaf", None)
        .await
        .expect("re-sync at higher version should succeed");
    assert_eq!(outcome.version, 3);
    let recipe_body =
        std::fs::read_to_string(ws.join("zen-leaf").join("recipe.forage")).unwrap();
    assert!(recipe_body.contains("s_new"));
}

#[tokio::test]
async fn fetch_to_cache_writes_decls_under_cache_root() {
    let server = MockServer::start().await;
    let art = artifact_v1("alice", "zen-leaf");
    Mock::given(method("GET"))
        .and(path("/v1/packages/alice/zen-leaf/versions/1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&art))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    let cache = tmp.path();
    let client = HubClient::new(server.uri());
    let fetched = forage_hub::fetch_to_cache(&client, cache, "alice", "zen-leaf", 1)
        .await
        .unwrap();
    assert_eq!(fetched.dir, cache.join("alice").join("zen-leaf").join("1"));
    assert!(fetched.dir.join("recipe.forage").is_file());
    // Decls land alongside the recipe folder so the workspace loader
    // picks them up — same layout `Workspace::catalog`'s
    // `scan_package_declarations` walks.
    assert!(cache.join("alice").join("zen-leaf").join("shared.forage").is_file());
    assert!(!fetched.sha256.is_empty(), "sha256 must be populated");
}
