//! `forage record <recipe-name>` runs the recipe against the network
//! and writes captured HTTP exchanges to `_fixtures/<recipe>.jsonl`.
//! The integration test points the recipe at a wiremock-faked host
//! and asserts the file appears at the recipe-name-keyed path with
//! the exchange faithfully serialized.

use assert_cmd::Command;
use forage_replay::Capture;
use std::fs;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const MANIFEST: &str = "description = \"test\"\ncategory = \"scrape\"\ntags = []\n";

#[tokio::test]
async fn record_writes_captures_to_underscore_fixtures() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/items"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"items": [{"id": "a"}]})),
        )
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path();
    fs::write(ws.join("forage.toml"), MANIFEST).unwrap();
    let recipe = format!(
        "recipe \"items\"\nengine http\nstep list {{\n    method \"GET\"\n    url \"{}/items\"\n}}\n",
        server.uri()
    );
    fs::write(ws.join("items.forage"), recipe).unwrap();

    Command::cargo_bin("forage")
        .unwrap()
        .current_dir(ws)
        .arg("record")
        .arg("items")
        .assert()
        .success()
        .stdout(predicates::str::contains("recorded"));

    let path = ws.join("_fixtures").join("items.jsonl");
    assert!(path.is_file(), "record must write the JSONL stream");
    let captures = forage_replay::read_jsonl(&path).unwrap();
    assert_eq!(captures.len(), 1, "exactly one HTTP exchange captured");
    let Capture::Http(ex) = &captures[0] else {
        panic!("expected an HTTP capture, got {:?}", captures[0]);
    };
    assert_eq!(ex.method, "GET");
    assert!(ex.url.ends_with("/items"));
    assert_eq!(ex.status, 200);
    assert!(ex.body.contains("\"id\""));
}
