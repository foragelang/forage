//! Replay-mode browser engine.
//!
//! Reads `captures.jsonl`, walks each `captures.match` rule against the
//! matching captures, then runs `captures.document` (if any) against the
//! recorded HTML. Bodies execute via `forage_core::Evaluator` — emit
//! blocks accumulate into the shared `Snapshot`.

use indexmap::IndexMap;
use regex::Regex;

use forage_core::ast::*;
use forage_core::eval::default_registry;
use forage_core::{EvalValue, Evaluator, Record, Scope, Snapshot};
use forage_replay::{BrowserCapture, Capture};

use crate::error::{BrowserError, BrowserResult};

pub struct ReplayEngine<'r> {
    pub recipe: &'r Recipe,
    pub captures: &'r [Capture],
}

impl<'r> ReplayEngine<'r> {
    pub fn new(recipe: &'r Recipe, captures: &'r [Capture]) -> Self {
        Self { recipe, captures }
    }

    pub fn run(
        &self,
        inputs: IndexMap<String, EvalValue>,
        secrets: IndexMap<String, String>,
    ) -> BrowserResult<Snapshot> {
        let cfg = self
            .recipe
            .browser
            .as_ref()
            .ok_or(BrowserError::MissingBrowserConfig)?;
        let registry = default_registry();
        let evaluator = Evaluator::new(registry);
        let mut scope = Scope::new().with_inputs(inputs).with_secrets(secrets);
        let mut snapshot = Snapshot::new();

        // Top-level body (e.g. Jane's `emit Dispensary` before captures).
        for s in &self.recipe.body {
            run_statement(s, &evaluator, &mut scope, &mut snapshot)?;
        }

        // For each captures.match rule: filter the capture list, run the
        // body with `$<iter_var>` bound to the captured body.
        for cap_rule in &cfg.captures {
            let re = Regex::new(&cap_rule.url_pattern)
                .map_err(|e| BrowserError::Regex(e.to_string()))?;
            for c in self.captures {
                if let Capture::Browser(BrowserCapture::Match { url, body, .. }) = c {
                    if !re.is_match(url) {
                        continue;
                    }
                    let parsed = parse_body(body);
                    scope.push_frame();
                    scope.bind(&cap_rule.iter_var, parsed.clone());
                    let saved = scope.current.clone();
                    scope.current = Some(parsed);

                    // Evaluate the iter_path against the current scope, then
                    // for-loop the body over the resulting array.
                    let collection = evaluator.eval_extraction(&cap_rule.iter_path, &scope)?;
                    run_for_each_item(
                        collection,
                        &cap_rule.body,
                        &cap_rule.iter_var,
                        &evaluator,
                        &mut scope,
                        &mut snapshot,
                    )?;

                    scope.current = saved;
                    scope.pop_frame();
                }
            }
        }

        // captures.document — runs against the recorded document HTML.
        if let Some(doc_rule) = &cfg.document_capture {
            for c in self.captures {
                if let Capture::Browser(BrowserCapture::Document { html, .. }) = c {
                    let parsed = EvalValue::Node(html.clone());
                    scope.push_frame();
                    scope.bind(&doc_rule.iter_var, parsed.clone());
                    let saved = scope.current.clone();
                    scope.current = Some(parsed);

                    let collection = evaluator.eval_extraction(&doc_rule.iter_path, &scope)?;
                    run_for_each_item(
                        collection,
                        &doc_rule.body,
                        &doc_rule.iter_var,
                        &evaluator,
                        &mut scope,
                        &mut snapshot,
                    )?;

                    scope.current = saved;
                    scope.pop_frame();
                    // Document fires once per run.
                    break;
                }
            }
        }

        snapshot.evaluate_expectations(&self.recipe.expectations);
        Ok(snapshot)
    }
}

pub fn run_browser_replay(
    recipe: &Recipe,
    captures: &[Capture],
    inputs: IndexMap<String, EvalValue>,
    secrets: IndexMap<String, String>,
) -> BrowserResult<Snapshot> {
    ReplayEngine::new(recipe, captures).run(inputs, secrets)
}

fn run_statement(
    s: &Statement,
    evaluator: &Evaluator<'_>,
    scope: &mut Scope,
    snapshot: &mut Snapshot,
) -> BrowserResult<()> {
    match s {
        Statement::Step(_) => Ok(()),
        Statement::Emit(em) => run_emit(em, evaluator, scope, snapshot),
        Statement::ForLoop {
            variable,
            collection,
            body,
            ..
        } => {
            let collection_val = evaluator.eval_extraction(collection, scope)?;
            run_for_each_item(collection_val, body, variable, evaluator, scope, snapshot)
        }
    }
}

fn run_for_each_item(
    collection: EvalValue,
    body: &[Statement],
    variable: &str,
    evaluator: &Evaluator<'_>,
    scope: &mut Scope,
    snapshot: &mut Snapshot,
) -> BrowserResult<()> {
    let items = match collection {
        EvalValue::Array(xs) => xs,
        EvalValue::NodeList(xs) => xs.into_iter().map(EvalValue::Node).collect(),
        EvalValue::Null => Vec::new(),
        other => vec![other],
    };
    for item in items {
        scope.push_frame();
        scope.bind(variable, item.clone());
        let saved = scope.current.clone();
        scope.current = Some(item);
        for s in body {
            run_statement(s, evaluator, scope, snapshot)?;
        }
        scope.current = saved;
        scope.pop_frame();
    }
    Ok(())
}

fn run_emit(
    em: &Emission,
    evaluator: &Evaluator<'_>,
    scope: &Scope,
    snapshot: &mut Snapshot,
) -> BrowserResult<()> {
    let mut fields: IndexMap<String, JSONValue> = IndexMap::new();
    for b in &em.bindings {
        let v = evaluator.eval_extraction(&b.expr, scope)?;
        fields.insert(b.field_name.clone(), v.into_json());
    }
    snapshot.emit(Record {
        type_name: em.type_name.clone(),
        fields,
    });
    Ok(())
}

fn parse_body(body: &str) -> EvalValue {
    // Try JSON; fall back to a string.
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(body) {
        return (&v).into();
    }
    EvalValue::String(body.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use forage_core::parse;
    use forage_replay::BrowserCapture;

    #[test]
    fn captures_match_emits_records() {
        let src = r#"
            recipe "demo" {
                engine browser
                type Item { id: String }
                browser {
                    initialURL: "https://example.com"
                    observe:    "example.com/api"
                    paginate browserPaginate.scroll {
                        until: noProgressFor(2)
                        maxIterations: 5
                    }
                    captures.match {
                        urlPattern: "example.com/api/items"
                        for $r in $.items[*] {
                            emit Item { id ← $r.id }
                        }
                    }
                }
            }
        "#;
        let recipe = parse(src).unwrap();
        let cap = Capture::Browser(BrowserCapture::Match {
            url: "https://example.com/api/items?p=1".into(),
            method: "GET".into(),
            status: 200,
            body: r#"{"items":[{"id":"a"},{"id":"b"},{"id":"c"}]}"#.into(),
        });
        let snap = run_browser_replay(&recipe, &[cap], IndexMap::new(), IndexMap::new()).unwrap();
        assert_eq!(snap.records.len(), 3);
    }

    #[test]
    fn captures_document_emits_records() {
        let src = r#"
            recipe "letterboxd" {
                engine browser
                type Film { title: String }
                browser {
                    initialURL: "https://letterboxd.com/films/popular"
                    observe:    "letterboxd.com"
                    paginate browserPaginate.scroll {
                        until: noProgressFor(2)
                        maxIterations: 5
                    }
                    captures.document {
                        for $poster in $ | select(".film-poster") {
                            emit Film {
                                title ← $poster | select(".frame-title") | text
                            }
                        }
                    }
                }
            }
        "#;
        let recipe = parse(src).unwrap();
        let cap = Capture::Browser(BrowserCapture::Document {
            url: "https://letterboxd.com/films/popular".into(),
            html: r#"
                <div class="film-poster"><span class="frame-title">Inception</span></div>
                <div class="film-poster"><span class="frame-title">The Matrix</span></div>
            "#
            .into(),
        });
        let snap = run_browser_replay(&recipe, &[cap], IndexMap::new(), IndexMap::new()).unwrap();
        assert_eq!(snap.records.len(), 2);
        let titles: Vec<String> = snap
            .records
            .iter()
            .filter_map(|r| match r.fields.get("title")? {
                JSONValue::String(s) => Some(s.clone()),
                _ => None,
            })
            .collect();
        assert!(titles.iter().any(|t| t.contains("Inception")));
        assert!(titles.iter().any(|t| t.contains("Matrix")));
    }
}
