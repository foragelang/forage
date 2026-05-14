//! End-to-end shared-recipe parity. For each entry in
//! `Tests/shared-recipes/expected.json` that carries a `runSnapshot`,
//! parse the recipe, drive it through the HTTP engine against the
//! declared fixtures, and compare emitted records. The TS port runs
//! the same `runSnapshot` through its own runner, so divergence
//! between the Rust engine and the TS port fails one side first.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use forage_core::{EvalValue, parse};
use forage_http::engine::Engine;
use forage_http::transport::ReplayTransport;
use forage_replay::{Capture, HttpExchange};
use indexmap::IndexMap;
use serde::Deserialize;

#[derive(Deserialize)]
struct ExpectedFile {
    recipes: Vec<RecipeExpect>,
}

#[derive(Deserialize)]
struct RecipeExpect {
    file: String,
    #[serde(default, rename = "runSnapshot")]
    run_snapshot: Option<RunSnapshot>,
}

#[derive(Deserialize)]
struct RunSnapshot {
    #[serde(default)]
    inputs: HashMap<String, serde_json::Value>,
    #[serde(default, rename = "httpFixtures")]
    http_fixtures: Vec<HttpFixture>,
    records: Vec<ExpectedRecord>,
}

#[derive(Deserialize)]
struct HttpFixture {
    url: String,
    method: String,
    status: u16,
    body: String,
}

#[derive(Deserialize)]
struct ExpectedRecord {
    #[serde(rename = "typeName")]
    type_name: String,
    fields: serde_json::Map<String, serde_json::Value>,
}

fn shared_dir() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p.push("Tests");
    p.push("shared-recipes");
    p
}

#[tokio::test]
async fn shared_recipes_match_run_snapshots() {
    let dir = shared_dir();
    let raw = fs::read_to_string(dir.join("expected.json")).unwrap();
    let exp: ExpectedFile = serde_json::from_str(&raw).unwrap();

    let mut failures = Vec::<String>::new();
    let mut ran = 0;

    for r in &exp.recipes {
        let Some(snapshot) = &r.run_snapshot else {
            continue;
        };
        ran += 1;
        let src = fs::read_to_string(dir.join(&r.file)).unwrap();
        let recipe = match parse(&src) {
            Ok(r) => r,
            Err(e) => {
                failures.push(format!("{}: parse: {e}", r.file));
                continue;
            }
        };

        let inputs: IndexMap<String, EvalValue> = snapshot
            .inputs
            .iter()
            .map(|(k, v)| (k.clone(), EvalValue::from(v)))
            .collect();

        let captures: Vec<Capture> = snapshot
            .http_fixtures
            .iter()
            .map(|f| {
                Capture::Http(HttpExchange {
                    url: f.url.clone(),
                    method: f.method.clone(),
                    request_headers: IndexMap::new(),
                    request_body: None,
                    status: f.status,
                    response_headers: IndexMap::new(),
                    body: f.body.clone(),
                })
            })
            .collect();

        let transport = ReplayTransport::new(captures);
        let engine = Engine::new(&transport);
        let snap = match engine.run(&recipe, inputs, IndexMap::new()).await {
            Ok(s) => s,
            Err(e) => {
                failures.push(format!("{}: run: {e}", r.file));
                continue;
            }
        };

        if snap.records.len() != snapshot.records.len() {
            failures.push(format!(
                "{}: emitted {} records, expected {}",
                r.file,
                snap.records.len(),
                snapshot.records.len(),
            ));
            continue;
        }

        for (i, (got, want)) in snap.records.iter().zip(snapshot.records.iter()).enumerate() {
            if got.type_name != want.type_name {
                failures.push(format!(
                    "{}: record[{}] typeName {:?} != expected {:?}",
                    r.file, i, got.type_name, want.type_name,
                ));
            }
            for (k, want_v) in &want.fields {
                let got_v = got
                    .fields
                    .get(k)
                    .map(|v| serde_json::to_value(v).unwrap())
                    .unwrap_or(serde_json::Value::Null);
                if &got_v != want_v {
                    failures.push(format!(
                        "{}: record[{}].fields[{:?}] got {} expected {}",
                        r.file, i, k, got_v, want_v,
                    ));
                }
            }
        }
    }

    assert!(ran > 0, "no `runSnapshot` entries in expected.json");
    if !failures.is_empty() {
        for f in &failures {
            eprintln!("--- {f}");
        }
        panic!("{} shared-recipe runtime failures", failures.len());
    }
}
