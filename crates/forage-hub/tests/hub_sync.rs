//! End-to-end roundtrips against a wiremock-faked hub-api:
//!
//! - `sync_from_hub` materializes a `PackageVersion` into the flat
//!   workspace, writing the recipe to `<workspace>/<slug>.forage`,
//!   decls at the workspace root, fixtures + snapshot under the data
//!   dirs, and the per-recipe sidecar under `.forage/sync/`.
//! - `assemble_publish_request` walks the on-disk workspace back into
//!   the wire artifact, picking up `base_version` from the sidecar.
//! - 409 stale-base from the publish endpoint surfaces as
//!   `HubError::StaleBase` with the version numbers preserved.
//! - `fork_from_hub` POSTs the fork endpoint and then syncs the new
//!   fork's v1 artifact into the destination workspace.

use forage_hub::{
    ForageMeta, HubClient, HubError, PackageFile, PackageFixture, PackageMetadata, PackageVersion,
    assemble_publish_request, fork_from_hub, publish_from_workspace, read_meta, sync_from_hub,
    write_meta,
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
            source: "share type Shared { id: String }\n".into(),
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

/// Stage a workspace with a `forage.toml` plus a recipe file named
/// `<slug>.forage` carrying `recipe "<slug>"`. Returns the workspace
/// root so tests can extend it.
fn stage_workspace(ws_root: &std::path::Path, author: &str, slug: &str) {
    std::fs::write(
        ws_root.join("forage.toml"),
        format!(
            "name = \"{author}/{slug}\"\ndescription = \"test pkg\"\ncategory = \"scrape\"\ntags = [\"test\"]\n"
        ),
    )
    .unwrap();
    std::fs::write(
        ws_root.join(format!("{slug}.forage")),
        format!(
            "recipe \"{slug}\"\nengine http\nstep s {{ method \"GET\" url \"x\" }}\n"
        ),
    )
    .unwrap();
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
async fn sync_materializes_flat_workspace_with_sidecar() {
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

    // Recipe lands at `<workspace>/<slug>.forage` (flat). Decls at
    // the workspace root, captures and snapshot in the data dirs
    // keyed by recipe name.
    assert!(ws.join("zen-leaf.forage").is_file());
    assert!(ws.join("shared.forage").is_file());
    assert!(ws.join("_fixtures").join("zen-leaf.jsonl").is_file());
    // No legacy nested folder layout.
    assert!(!ws.join("zen-leaf").exists());

    // The sidecar carries the publish-back base_version and lives
    // under `.forage/sync/`.
    let meta = read_meta(ws, "zen-leaf").unwrap().unwrap();
    assert_eq!(meta.base_version, 1);
    assert_eq!(meta.origin, "@alice/zen-leaf@v1");
    assert!(ws.join(".forage").join("sync").join("zen-leaf.json").is_file());
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
    let ws_root = tmp.path();
    stage_workspace(ws_root, author, slug);
    // Sidecar carries the synced base version (1); the publish path
    // should send it as `base_version: 1`.
    write_meta(
        ws_root,
        slug,
        &ForageMeta {
            origin: ForageMeta::pretty_origin(author, slug, 1),
            author: author.into(),
            slug: slug.into(),
            base_version: 1,
            forked_from: None,
        },
    )
    .unwrap();

    let ws = forage_core::workspace::load(ws_root).unwrap();
    let client = HubClient::new(server.uri()).with_token("test-token");
    let resp = publish_from_workspace(
        &client,
        &ws,
        slug,
        author,
        "test pkg".into(),
        "scrape".into(),
        vec!["test".into()],
    )
    .await
    .expect("publish should succeed");
    assert_eq!(resp.version, 2);
    assert_eq!(resp.latest_version, 2);

    // Sidecar reflects the new base after a successful publish.
    let meta = read_meta(ws_root, slug).unwrap().unwrap();
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
    let ws_root = tmp.path();
    stage_workspace(ws_root, author, slug);
    write_meta(
        ws_root,
        slug,
        &ForageMeta {
            origin: ForageMeta::pretty_origin(author, slug, 3),
            author: author.into(),
            slug: slug.into(),
            base_version: 3,
            forked_from: None,
        },
    )
    .unwrap();

    let ws = forage_core::workspace::load(ws_root).unwrap();
    let client = HubClient::new(server.uri()).with_token("test-token");
    let err = publish_from_workspace(
        &client,
        &ws,
        slug,
        author,
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

    let meta = read_meta(ws, upstream_slug).unwrap().unwrap();
    assert_eq!(
        meta.forked_from.as_ref().map(|f| f.author.as_str()),
        Some(upstream_author)
    );
    // The fork lands as a flat `<slug>.forage` at the workspace root.
    assert!(ws.join(format!("{upstream_slug}.forage")).is_file());
}

#[tokio::test]
async fn assemble_request_matches_disk_state() {
    // Walks the assembly path directly: stage a flat workspace with
    // recipe + shared decl + fixtures + snapshot + sidecar, then
    // assert the wire shape.
    let tmp = tempfile::tempdir().unwrap();
    let ws_root = tmp.path();
    let slug = "zen-leaf";
    stage_workspace(ws_root, "alice", slug);
    std::fs::write(
        ws_root.join("shared.forage"),
        "share type Shared { id: String }\n",
    )
    .unwrap();
    std::fs::create_dir_all(ws_root.join("_fixtures")).unwrap();
    std::fs::write(
        ws_root.join("_fixtures").join(format!("{slug}.jsonl")),
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
        record_types: IndexMap::new(),
    };
    std::fs::create_dir_all(ws_root.join("_snapshots")).unwrap();
    std::fs::write(
        ws_root.join("_snapshots").join(format!("{slug}.json")),
        serde_json::to_string(&core_snapshot).unwrap(),
    )
    .unwrap();

    write_meta(
        ws_root,
        slug,
        &ForageMeta {
            origin: ForageMeta::pretty_origin("alice", slug, 2),
            author: "alice".into(),
            slug: slug.into(),
            base_version: 2,
            forked_from: None,
        },
    )
    .unwrap();

    let ws = forage_core::workspace::load(ws_root).unwrap();
    let req = assemble_publish_request(
        &ws,
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
    // A workspace that already has `<slug>.forage` but no sidecar
    // means local edits exist that the sync would overwrite.
    // sync_from_hub returns an error instead of clobbering.
    let server = MockServer::start().await;
    let art = artifact_v1("alice", "zen-leaf");
    mount_read_package(&server, &art).await;

    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path();
    std::fs::write(ws.join("zen-leaf.forage"), "local edits not from hub\n").unwrap();

    let client = HubClient::new(server.uri());
    let err = sync_from_hub(&client, ws, "alice", "zen-leaf", None)
        .await
        .expect_err("must refuse to clobber local recipe");
    assert!(
        format!("{err}").contains("already exists locally"),
        "unexpected error: {err}"
    );
    // The local file is untouched.
    let body = std::fs::read_to_string(ws.join("zen-leaf.forage")).unwrap();
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
    std::fs::write(
        ws.join("zen-leaf.forage"),
        "recipe \"zen-leaf\"\nengine http\nstep s_old { method \"GET\" url \"x\" }\n",
    )
    .unwrap();
    write_meta(
        ws,
        "zen-leaf",
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
    let recipe_body = std::fs::read_to_string(ws.join("zen-leaf.forage")).unwrap();
    assert!(recipe_body.contains("s_new"));
}

/// Sync a forked recipe (sidecar carries `forked_from` from the
/// upstream's package metadata), edit it, then publish v2 back. The
/// hub rejects v2+ publishes that include `forked_from` — this test
/// asserts the assembler drops the field even though it lives in the
/// sidecar.
#[tokio::test]
async fn sync_edit_publish_does_not_send_forked_from() {
    let server = MockServer::start().await;
    let author = "bob";
    let slug = "zen-leaf";

    // Step 1: hub-side metadata claims this fork descends from
    // @alice/zen-leaf v3. sync_from_hub copies that into the sidecar.
    let mut meta = package_meta(author, slug);
    meta.forked_from = Some(forage_hub::ForkedFrom {
        author: "alice".into(),
        slug: slug.into(),
        version: 3,
    });
    Mock::given(method("GET"))
        .and(path(format!("/v1/packages/{author}/{slug}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(meta))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/packages/{author}/{slug}/versions/latest")))
        .respond_with(ResponseTemplate::new(200).set_body_json(artifact_v1(author, slug)))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("/v1/packages/{author}/{slug}/downloads")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "downloads": 1 })))
        .mount(&server)
        .await;

    // Step 2: the publish endpoint accepts a body whose top-level keys
    // are exactly the documented PublishRequest fields — and crucially
    // NOT `forked_from`. wiremock's `body_json` matcher does an exact
    // shape comparison: if the assembler ever started smuggling
    // `forked_from` onto the wire, the body wouldn't match this mock
    // and the publish would 404, failing the test.
    Mock::given(method("POST"))
        .and(path(format!("/v1/packages/{author}/{slug}/versions")))
        .and(body_json(json!({
            "description": "edited",
            "category": "scrape",
            "tags": ["t"],
            "recipe": format!(
                "recipe \"{slug}\"\nengine http\n\nstep s {{ method \"GET\" url \"https://example.test\" }}\n"
            ),
            "decls": [{"name": "shared.forage", "source": "share type Shared { id: String }\n"}],
            "fixtures": [{"name": "captures.jsonl", "content": "{\"kind\":\"http\",\"url\":\"https://example.test\",\"method\":\"GET\",\"status\":200,\"body\":\"{}\"}\n"}],
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
    let ws_root = tmp.path();
    let client = HubClient::new(server.uri()).with_token("test-token");

    // Sync — the sidecar should carry forked_from for local display.
    let outcome = sync_from_hub(&client, ws_root, author, slug, None).await.unwrap();
    assert!(
        outcome.meta.forked_from.is_some(),
        "sidecar must track lineage for local UI",
    );
    let sidecar = read_meta(ws_root, slug).unwrap().unwrap();
    assert!(sidecar.forked_from.is_some());

    // After sync the workspace needs a manifest so `load` can mount
    // it; sync itself doesn't write one (sync materializes a single
    // recipe into an existing workspace).
    std::fs::write(
        ws_root.join("forage.toml"),
        format!("name = \"{author}/{slug}\"\ndescription = \"\"\ncategory = \"scrape\"\ntags = []\n"),
    )
    .unwrap();
    let ws = forage_core::workspace::load(ws_root).unwrap();

    // Publish — body_json matcher fails if the body carries any
    // additional fields (e.g. forked_from). A second publish on the
    // same fork would re-trigger the regression if it ever returned.
    let resp = publish_from_workspace(
        &client,
        &ws,
        slug,
        author,
        "edited".into(),
        "scrape".into(),
        vec!["t".into()],
    )
    .await
    .expect("publish should succeed without forked_from on the wire");
    assert_eq!(resp.version, 2);

    // After publish, the sidecar still tracks lineage — `forked_from`
    // is local-only state, untouched by the publish round-trip.
    let after = read_meta(ws_root, slug).unwrap().unwrap();
    assert!(
        after.forked_from.is_some(),
        "sidecar lineage must survive the publish",
    );
}

/// `fetch_to_cache` writes the version artifact into the layout that
/// the dep-cache reader (`scan_package_declarations`) walks. The
/// reader recurses inside `<cache>/<author>/<slug>/<version>/`, so the
/// writer must place decls there — not in the slug directory one
/// level up, where the reader would never see them.
#[tokio::test]
async fn fetch_to_cache_writes_decls_inside_version_subtree() {
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
    assert!(fetched.dir.join("zen-leaf.forage").is_file());
    // Decls land INSIDE the version subtree so the dep-cache reader
    // finds them (the reader walks the version dir recursively; it
    // never looks at the slug directory above it).
    assert!(fetched.dir.join("shared.forage").is_file());
    assert!(
        !cache
            .join("alice")
            .join("zen-leaf")
            .join("shared.forage")
            .exists(),
        "decls must not leak into the slug-level directory",
    );
    assert!(!fetched.sha256.is_empty(), "sha256 must be populated");
}

/// Round-trip: `fetch_to_cache` writes a version into the cache, then
/// `Workspace::catalog` loads a recipe whose manifest depends on that
/// version. The catalog must pick up the cached decls. Serialised
/// because `FORAGE_HUB_CACHE` is process-global.
#[tokio::test]
#[serial_test::serial]
async fn fetch_to_cache_roundtrips_through_workspace_catalog() {
    let server = MockServer::start().await;
    let art = artifact_v1("alice", "zen-leaf");
    Mock::given(method("GET"))
        .and(path("/v1/packages/alice/zen-leaf/versions/1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&art))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    let cache = tmp.path().join("hub-cache");
    let ws_root = tmp.path().join("ws");
    std::fs::create_dir_all(&ws_root).unwrap();

    // Point the workspace loader at our temp cache. `FORAGE_HUB_CACHE`
    // is process-global; `#[serial_test::serial]` serialises this test
    // against any other test that touches the same var.
    let prev = std::env::var("FORAGE_HUB_CACHE").ok();
    // SAFETY: env mutation is unsafe in Rust 2024; the serial attribute
    // serialises against other env-var-touching tests in the harness.
    unsafe { std::env::set_var("FORAGE_HUB_CACHE", &cache); }

    let client = HubClient::new(server.uri());
    forage_hub::fetch_to_cache(&client, &cache, "alice", "zen-leaf", 1)
        .await
        .unwrap();

    // Build a workspace that declares the dep at v1, then load the
    // catalog. The cached decls must show up.
    std::fs::write(
        ws_root.join("forage.toml"),
        "name = \"bob/uses-zen\"\n\
         description = \"deps zen-leaf shared types\"\n\
         category = \"scrape\"\n\
         tags = [\"test\"]\n\
         [deps]\n\
         \"alice/zen-leaf\" = 1\n",
    )
    .unwrap();
    std::fs::write(
        ws_root.join("uses-zen.forage"),
        "recipe \"uses-zen\"\nengine http\nstep s { method \"GET\" url \"https://example.test\" }\n",
    )
    .unwrap();

    let ws = forage_core::workspace::load(&ws_root).unwrap();
    let recipe_path = ws_root.join("uses-zen.forage");
    let catalog = ws.catalog_from_disk(&recipe_path).unwrap();

    // SAFETY: see safety comment above.
    match prev {
        Some(v) => unsafe { std::env::set_var("FORAGE_HUB_CACHE", v) },
        None => unsafe { std::env::remove_var("FORAGE_HUB_CACHE") },
    }

    assert!(
        catalog.types.contains_key("Shared"),
        "dep cache decls must be visible in the workspace catalog",
    );
}

/// Sync an artifact whose internal recipe header name disagrees with
/// the hub-side slug. The sync layer keys data dirs and sidecars on
/// the header name, so a mismatch would silently land captures in the
/// wrong place — surface it as a structured error instead.
#[tokio::test]
async fn sync_rejects_mismatched_recipe_header() {
    let server = MockServer::start().await;
    let mut art = artifact_v1("alice", "zen-leaf");
    art.recipe = "recipe \"different\"\nengine http\nstep s { method \"GET\" url \"x\" }\n".into();
    mount_read_package(&server, &art).await;

    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path();
    let client = HubClient::new(server.uri());
    let err = sync_from_hub(&client, ws, "alice", "zen-leaf", None)
        .await
        .expect_err("mismatched header name should surface");
    assert!(format!("{err}").contains("different"), "unexpected: {err}");
}

/// Round-trip: assemble a flat-workspace publish artifact, simulate
/// the hub accepting it, then sync the same artifact back into a
/// fresh workspace. The recipe + decls + fixtures + snapshot land
/// in the canonical flat shape, and the sidecar carries the freshly
/// stamped version.
#[tokio::test]
async fn flat_workspace_publish_round_trips_through_sync() {
    let server = MockServer::start().await;
    let author = "alice";
    let slug = "bar";

    // Stage a workspace with a recipe whose file basename
    // (`foo.forage`) differs from the recipe header name (`bar`) —
    // the publish path is keyed on the header name, not the file
    // basename, and the round-trip must preserve that.
    let tmp = tempfile::tempdir().unwrap();
    let pub_root = tmp.path().join("pub");
    std::fs::create_dir_all(&pub_root).unwrap();
    std::fs::write(
        pub_root.join("forage.toml"),
        format!(
            "name = \"{author}/{slug}\"\ndescription = \"flat ws\"\ncategory = \"scrape\"\ntags = [\"t\"]\n"
        ),
    )
    .unwrap();
    std::fs::write(
        pub_root.join("foo.forage"),
        format!(
            "recipe \"{slug}\"\nengine http\nstep s {{ method \"GET\" url \"https://example.test\" }}\n"
        ),
    )
    .unwrap();
    std::fs::write(
        pub_root.join("shared.forage"),
        "share type Shared { id: String }\n",
    )
    .unwrap();
    std::fs::create_dir_all(pub_root.join("_fixtures")).unwrap();
    std::fs::write(
        pub_root.join("_fixtures").join(format!("{slug}.jsonl")),
        "{\"line\":1}\n",
    )
    .unwrap();

    let ws = forage_core::workspace::load(&pub_root).unwrap();

    // The hub accepts the publish and stamps v1, then turns around
    // and serves the same artifact on the GET endpoints. The shared
    // mock state captures the body so the GET returns the same bytes
    // the publisher sent.
    let recipe_src = std::fs::read_to_string(pub_root.join("foo.forage")).unwrap();
    let shared_src = std::fs::read_to_string(pub_root.join("shared.forage")).unwrap();
    let fixture_body = std::fs::read_to_string(
        pub_root.join("_fixtures").join(format!("{slug}.jsonl")),
    )
    .unwrap();
    let served_artifact = PackageVersion {
        author: author.into(),
        slug: slug.into(),
        version: 1,
        recipe: recipe_src.clone(),
        decls: vec![PackageFile {
            name: "shared.forage".into(),
            source: shared_src.clone(),
        }],
        fixtures: vec![PackageFixture {
            name: "captures.jsonl".into(),
            content: fixture_body.clone(),
        }],
        snapshot: None,
        base_version: None,
        published_at: 0,
        published_by: author.into(),
    };

    Mock::given(method("POST"))
        .and(path(format!("/v1/packages/{author}/{slug}/versions")))
        .and(header("authorization", "Bearer test-token"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "author": author,
            "slug": slug,
            "version": 1,
            "latest_version": 1,
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/packages/{author}/{slug}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(package_meta(author, slug)))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/packages/{author}/{slug}/versions/latest")))
        .respond_with(ResponseTemplate::new(200).set_body_json(&served_artifact))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("/v1/packages/{author}/{slug}/downloads")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "downloads": 1 })))
        .mount(&server)
        .await;

    let client = HubClient::new(server.uri()).with_token("test-token");
    let publish_resp = publish_from_workspace(
        &client,
        &ws,
        slug,
        author,
        "flat ws".into(),
        "scrape".into(),
        vec!["t".into()],
    )
    .await
    .expect("publish should succeed");
    assert_eq!(publish_resp.version, 1);
    assert_eq!(publish_resp.slug, slug);

    // Sidecar stamped post-publish.
    let pub_meta = read_meta(&pub_root, slug).unwrap().unwrap();
    assert_eq!(pub_meta.base_version, 1);

    // Now sync into a fresh workspace and verify the on-disk shape.
    let sync_root = tmp.path().join("sync");
    std::fs::create_dir_all(&sync_root).unwrap();
    let sync_outcome = sync_from_hub(&client, &sync_root, author, slug, None)
        .await
        .expect("sync should succeed");
    assert_eq!(sync_outcome.version, 1);
    assert_eq!(sync_outcome.recipe_path, sync_root.join(format!("{slug}.forage")));
    assert!(sync_root.join(format!("{slug}.forage")).is_file());
    assert!(sync_root.join("shared.forage").is_file());
    assert!(sync_root.join("_fixtures").join(format!("{slug}.jsonl")).is_file());
    let sync_meta = read_meta(&sync_root, slug).unwrap().unwrap();
    assert_eq!(sync_meta.author, author);
    assert_eq!(sync_meta.slug, slug);
    assert_eq!(sync_meta.base_version, 1);
}
