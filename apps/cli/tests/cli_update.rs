//! `forage update` resolves the manifest's `[deps]` recipes, fetches
//! each recipe artifact + every type it references into the on-disk
//! cache, and writes a lockfile that pins both citizens. Subsequent
//! workspace loads pull types out of the cache via the lockfile's
//! `[types]` table.

use assert_cmd::Command;
use forage_hub::{
    PackageFixture, PackageVersion, TypeFieldAlignment, TypeRef, TypeVersion,
};
use std::fs;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn artifact(author: &str, slug: &str) -> PackageVersion {
    PackageVersion {
        author: author.into(),
        slug: slug.into(),
        version: 1,
        recipe: format!(
            "recipe \"{slug}\"\nengine http\nstep s {{ method \"GET\" url \"x\" }}\n"
        ),
        type_refs: vec![TypeRef {
            author: author.into(),
            name: "Product".into(),
            version: 4,
        }],
        input_type_refs: Vec::new(),
        output_type_refs: Vec::new(),
        fixtures: vec![PackageFixture {
            name: "captures.jsonl".into(),
            content: String::new(),
        }],
        snapshot: None,
        base_version: None,
        published_at: 0,
        published_by: author.into(),
    }
}

fn type_version() -> TypeVersion {
    TypeVersion {
        author: "alice".into(),
        name: "Product".into(),
        version: 4,
        source: "share type Product {\n    id: String\n    name: String\n}\n".into(),
        alignments: Vec::new(),
        field_alignments: vec![
            TypeFieldAlignment {
                field: "id".into(),
                alignment: None,
            },
            TypeFieldAlignment {
                field: "name".into(),
                alignment: None,
            },
        ],
        base_version: None,
        published_at: 0,
        published_by: "alice".into(),
    }
}

/// `forage update` walks `[deps]`, fetches each recipe (which also
/// pulls every referenced type into the parallel type cache), and
/// writes a lockfile that pins both the recipe and every type it
/// references. The lockfile carries `[recipes]` + `[types]` tables;
/// subsequent workspace loads use the `[types]` pins to resolve the
/// catalog against the type cache.
#[tokio::test]
async fn forage_update_writes_recipes_and_types_to_lockfile() {
    let server = MockServer::start().await;
    let art = artifact("alice", "scrape-products");
    let tv = type_version();
    Mock::given(method("GET"))
        .and(path("/v1/packages/alice/scrape-products/versions/1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&art))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/types/alice/Product/versions/4"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&tv))
        .mount(&server)
        .await;
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("ws");
    let cache = tmp.path().join("cache");
    fs::create_dir_all(&ws).unwrap();
    fs::write(
        ws.join("forage.toml"),
        "name = \"bob/uses-scraper\"\n\
         description = \"\"\n\
         category = \"scrape\"\n\
         tags = []\n\
         [deps]\n\
         \"alice/scrape-products\" = 1\n",
    )
    .unwrap();

    Command::cargo_bin("forage")
        .unwrap()
        .env("FORAGE_HUB_CACHE", &cache)
        .arg("update")
        .arg(&ws)
        .arg("--hub")
        .arg(server.uri())
        .assert()
        .success()
        .stdout(predicates::str::contains("alice/scrape-products@1"))
        .stdout(predicates::str::contains("forage.lock"));

    // Lockfile carries both the recipe pin and the transitive type pin.
    let lock_body = fs::read_to_string(ws.join("forage.lock")).unwrap();
    assert!(
        lock_body.contains("[recipes.\"alice/scrape-products\"]"),
        "lockfile missing recipe pin:\n{lock_body}",
    );
    assert!(
        lock_body.contains("version = 1"),
        "lockfile missing recipe version:\n{lock_body}",
    );
    assert!(
        lock_body.contains("[types.\"alice/Product\"]"),
        "lockfile missing type pin:\n{lock_body}",
    );
    assert!(
        lock_body.contains("version = 4"),
        "lockfile missing type version:\n{lock_body}",
    );

    // The type cache holds the type source so the workspace loader
    // can resolve it. The recipe cache holds the recipe source.
    assert!(
        cache
            .join("types")
            .join("alice")
            .join("Product")
            .join("4.forage")
            .is_file(),
    );
    assert!(
        cache
            .join("alice")
            .join("scrape-products")
            .join("1")
            .join("scrape-products.forage")
            .is_file(),
    );
}
