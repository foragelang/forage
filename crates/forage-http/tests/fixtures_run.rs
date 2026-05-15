//! End-to-end parity for the bundled fixtures. For each entry that
//! carries a `runSnapshot`, parse the recipe, drive it through the
//! HTTP engine against the declared captures, and compare emitted
//! records. Any future implementation that wants to claim parity has
//! to clear the same snapshots.

use forage_core::{EvalValue, parse};
use forage_http::engine::Engine;
use forage_http::transport::ReplayTransport;
use forage_replay::{Capture, HttpExchange};
use forage_test::ExpectedFile;
use indexmap::IndexMap;

#[tokio::test]
async fn fixtures_match_run_snapshots() {
    let exp: ExpectedFile = forage_test::load_expected();

    let mut failures = Vec::<String>::new();
    let mut ran = 0;

    for r in &exp.recipes {
        let Some(snapshot) = &r.run_snapshot else {
            continue;
        };
        ran += 1;
        let src = forage_test::load_recipe_source(&r.file);
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
        panic!("{} fixture runtime failures", failures.len());
    }
}
