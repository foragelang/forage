//! HTTP engine: walks a `Recipe`'s body against a `Transport`, evaluating
//! emit blocks and accumulating records into a `Snapshot`.
//!
//! Live and replay flows share the same Engine code; only the Transport
//! differs.

use indexmap::IndexMap;
use tracing::debug;

use crate::auth::{AuthState, apply_request_headers, run_session_login};
use crate::body::render_body;
use crate::error::{HttpError, HttpResult};
use crate::paginate::{NextPage, PaginationDriver};
use crate::transport::{HttpRequest, HttpResponse, Transport};

use forage_core::ast::*;
use forage_core::eval::default_registry;
use forage_core::{EvalValue, Evaluator, Record, Scope, Snapshot};

/// Engine knobs.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// Stop after this many total step requests (safety net).
    pub max_requests: u32,
    /// User-Agent for live requests.
    pub user_agent: String,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            max_requests: 500,
            user_agent: format!(
                "Forage/{} (+https://foragelang.com)",
                env!("CARGO_PKG_VERSION")
            ),
        }
    }
}

pub struct Engine<'t> {
    pub transport: &'t dyn Transport,
    pub config: EngineConfig,
}

impl<'t> Engine<'t> {
    pub fn new(transport: &'t dyn Transport) -> Self {
        Self {
            transport,
            config: EngineConfig::default(),
        }
    }

    pub fn with_config(mut self, config: EngineConfig) -> Self {
        self.config = config;
        self
    }

    pub async fn run(
        &self,
        recipe: &Recipe,
        inputs: IndexMap<String, EvalValue>,
        secrets: IndexMap<String, String>,
    ) -> HttpResult<Snapshot> {
        let registry = default_registry();
        let evaluator = Evaluator::new(registry);
        let mut scope = Scope::new().with_inputs(inputs).with_secrets(secrets);
        let mut snapshot = Snapshot::new();

        // Run session-auth login flow if declared. Cookies thread via
        // the Transport's cookie jar; bearer tokens flow through AuthState.
        let auth_state = run_session_login(
            recipe.auth.as_ref(),
            self.transport,
            &evaluator,
            &scope,
            &self.config.user_agent,
        )
        .await?;

        let mut requests_made: u32 = 0;
        self.run_statements(
            &recipe.body,
            recipe,
            &auth_state,
            &evaluator,
            &mut scope,
            &mut snapshot,
            &mut requests_made,
        )
        .await?;
        snapshot.evaluate_expectations(&recipe.expectations);
        Ok(snapshot)
    }

    #[allow(clippy::too_many_arguments)]
    fn run_statements<'a>(
        &'a self,
        body: &'a [Statement],
        recipe: &'a Recipe,
        auth_state: &'a AuthState,
        evaluator: &'a Evaluator<'a>,
        scope: &'a mut Scope,
        snapshot: &'a mut Snapshot,
        requests_made: &'a mut u32,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = HttpResult<()>> + Send + 'a>> {
        Box::pin(async move {
            for s in body {
                match s {
                    Statement::Step(step) => {
                        self.run_step(step, recipe, auth_state, evaluator, scope, requests_made)
                            .await?;
                    }
                    Statement::Emit(em) => {
                        self.run_emit(em, evaluator, scope, snapshot)?;
                    }
                    Statement::ForLoop {
                        variable,
                        collection,
                        body,
                    } => {
                        let collection_val = evaluator.eval_extraction(collection, scope)?;
                        let items = match collection_val {
                            EvalValue::Array(xs) => xs,
                            EvalValue::NodeList(xs) => {
                                xs.into_iter().map(EvalValue::Node).collect()
                            }
                            EvalValue::Null => Vec::new(),
                            other => vec![other],
                        };
                        for item in items {
                            scope.push_frame();
                            scope.bind(variable, item.clone());
                            let saved_current = scope.current.clone();
                            scope.current = Some(item);
                            self.run_statements(
                                body,
                                recipe,
                                auth_state,
                                evaluator,
                                scope,
                                snapshot,
                                requests_made,
                            )
                            .await?;
                            scope.current = saved_current;
                            scope.pop_frame();
                        }
                    }
                }
            }
            Ok(())
        })
    }

    async fn run_step(
        &self,
        step: &HTTPStep,
        recipe: &Recipe,
        auth_state: &AuthState,
        evaluator: &Evaluator<'_>,
        scope: &mut Scope,
        requests_made: &mut u32,
    ) -> HttpResult<()> {
        let mut driver = step.pagination.as_ref().map(PaginationDriver::new);
        let mut extra_query: Vec<(String, String)> = Vec::new();

        loop {
            if *requests_made >= self.config.max_requests {
                return Err(HttpError::Generic(format!(
                    "exceeded max_requests ({})",
                    self.config.max_requests
                )));
            }
            *requests_made += 1;

            let req =
                self.build_request(step, recipe, auth_state, evaluator, scope, &extra_query)?;
            debug!(method = %req.method, url = %req.url, "step request");

            let resp = self.transport.fetch(req.clone()).await?;
            if !(200..400).contains(&resp.status) {
                return Err(HttpError::Status {
                    status: resp.status,
                    url: req.url,
                });
            }

            let body_val = parse_response_body(&req.url, &resp)?;

            // Bind `$<stepName>` to the response body for downstream eval.
            scope.bind(&step.name, body_val.clone());
            scope.current = Some(body_val.clone());

            // extract.regex { pattern, groups } — bind each group name from
            // the response body. Used by Leafbridge-style auth.htmlPrime.
            if let Some(ex) = &step.extract {
                let body_str = resp.body_str();
                apply_regex_extract(ex, body_str, scope)?;
            }

            match driver.as_mut() {
                Some(d) => match d.advance(evaluator, scope)? {
                    NextPage::Stop => return Ok(()),
                    NextPage::Continue(params) => {
                        extra_query = params;
                    }
                },
                None => return Ok(()),
            }
        }
    }

    fn run_emit(
        &self,
        em: &Emission,
        evaluator: &Evaluator<'_>,
        scope: &Scope,
        snapshot: &mut Snapshot,
    ) -> HttpResult<()> {
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

    fn build_request(
        &self,
        step: &HTTPStep,
        recipe: &Recipe,
        auth_state: &AuthState,
        evaluator: &Evaluator<'_>,
        scope: &Scope,
        extra_query: &[(String, String)],
    ) -> HttpResult<HttpRequest> {
        let mut url = evaluator.render_template(&step.request.url, scope)?;
        if !extra_query.is_empty() {
            let sep = if url.contains('?') { '&' } else { '?' };
            let qs: Vec<String> = extra_query
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect();
            url.push(sep);
            url.push_str(&qs.join("&"));
        }
        let mut headers: IndexMap<String, String> = IndexMap::new();
        for (k, t) in &step.request.headers {
            let v = evaluator.render_template(t, scope)?;
            headers.insert(k.clone(), v);
        }
        headers
            .entry("User-Agent".into())
            .or_insert(self.config.user_agent.clone());
        apply_request_headers(
            recipe.auth.as_ref(),
            auth_state,
            evaluator,
            scope,
            &mut headers,
        )?;

        let body = if let Some(b) = &step.request.body {
            let (content_type, bytes) = render_body(b, evaluator, scope)?;
            headers.entry("Content-Type".into()).or_insert(content_type);
            Some(bytes)
        } else {
            None
        };

        Ok(HttpRequest {
            method: step.request.method.clone(),
            url,
            headers,
            body,
        })
    }
}

fn apply_regex_extract(ex: &RegexExtract, body: &str, scope: &mut Scope) -> HttpResult<()> {
    let re = regex::Regex::new(&ex.pattern)
        .map_err(|e| HttpError::Generic(format!("regex compile: {e}")))?;
    if let Some(caps) = re.captures(body) {
        for (i, group_name) in ex.groups.iter().enumerate() {
            let v = caps
                .get(i + 1)
                .map(|m| EvalValue::String(m.as_str().to_string()))
                .unwrap_or(EvalValue::Null);
            scope.bind(group_name, v);
        }
    }
    Ok(())
}

fn parse_response_body(url: &str, resp: &HttpResponse) -> HttpResult<EvalValue> {
    let body = resp.body_str();
    // Try JSON first; if it doesn't parse, fall back to string.
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(body) {
        return Ok((&v).into());
    }
    // Empty body → null. Otherwise treat as raw string (HTML, plain text).
    if body.is_empty() {
        return Ok(EvalValue::Null);
    }
    let _ = url;
    Ok(EvalValue::String(body.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::ReplayTransport;
    use forage_core::parse;
    use forage_replay::{Capture, HttpExchange};

    #[tokio::test]
    async fn runs_one_step_recipe_against_replay() {
        let src = r#"
            recipe "tiny" {
                engine http
                type Item { id: String }
                step list {
                    method "GET"
                    url "https://api.example.com/items"
                }
                for $i in $list.items[*] {
                    emit Item { id ← $i.id }
                }
            }
        "#;
        let recipe = parse(src).unwrap();
        let exchange = Capture::Http(HttpExchange {
            url: "https://api.example.com/items".into(),
            method: "GET".into(),
            request_headers: IndexMap::new(),
            request_body: None,
            status: 200,
            response_headers: IndexMap::new(),
            body: r#"{"items":[{"id":"a"},{"id":"b"}]}"#.into(),
        });
        let transport = ReplayTransport::new(vec![exchange]);
        let engine = Engine::new(&transport);
        let snap = engine
            .run(&recipe, IndexMap::new(), IndexMap::new())
            .await
            .unwrap();
        assert_eq!(snap.records.len(), 2);
        assert_eq!(snap.records[0].type_name, "Item");
        assert_eq!(
            snap.records[0].fields.get("id"),
            Some(&JSONValue::String("a".into()))
        );
    }

    #[tokio::test]
    async fn missing_fixture_errors() {
        let src = r#"
            recipe "tiny" {
                engine http
                type T { x: String }
                step go {
                    method "GET"
                    url "https://api.example.com/nope"
                }
                emit T { x ← "hi" }
            }
        "#;
        let recipe = parse(src).unwrap();
        let transport = ReplayTransport::new(vec![]);
        let engine = Engine::new(&transport);
        let err = engine
            .run(&recipe, IndexMap::new(), IndexMap::new())
            .await
            .unwrap_err();
        assert!(matches!(err, HttpError::NoFixture { .. }));
    }
}
