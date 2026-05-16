//! HTTP engine: walks a `Recipe`'s body against a `Transport`, evaluating
//! emit blocks and accumulating records into a `Snapshot`.
//!
//! Live and replay flows share the same Engine code; only the Transport
//! differs.

use indexmap::IndexMap;
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, trace};

use crate::auth::{AuthState, apply_request_headers, run_session_login};
use crate::body::render_body;
use crate::debug::{
    BODY_CAPTURE_MAX, DebugScope, Debugger, EmitPause, ForLoopPause, IterationPause, ResumeAction,
    StepPause, StepResponse,
};
use crate::error::{HttpError, HttpResult};
use crate::paginate::{NextPage, PaginationDriver};
use crate::progress::{NoopSink, ProgressSink, RunEvent};
use crate::transport::{EngineTransportContext, HttpRequest, HttpResponse, Transport};

use forage_core::ast::*;
use forage_core::eval::{TransformRegistry, default_registry};
use forage_core::{
    EvalValue, Evaluator, LineMap, PriorRecords, Record, RunOptions, Scope, Snapshot, TypeCatalog,
};

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
    pub progress: Arc<dyn ProgressSink>,
    /// Optional step debugger. When set, the engine awaits a `ResumeAction`
    /// before each step. Absent → run uninterrupted (same as a no-op impl).
    pub debugger: Option<Arc<dyn Debugger>>,
}

impl<'t> Engine<'t> {
    pub fn new(transport: &'t dyn Transport) -> Self {
        Self {
            transport,
            config: EngineConfig::default(),
            progress: Arc::new(NoopSink),
            debugger: None,
        }
    }

    pub fn with_config(mut self, config: EngineConfig) -> Self {
        self.config = config;
        self
    }

    pub fn with_progress(mut self, p: Arc<dyn ProgressSink>) -> Self {
        self.progress = p;
        self
    }

    pub fn with_debugger(mut self, d: Arc<dyn Debugger>) -> Self {
        self.debugger = Some(d);
        self
    }

    fn emit(&self, event: RunEvent) {
        self.progress.emit(event);
    }

    pub async fn run(
        &self,
        recipe: &ForageFile,
        catalog: &TypeCatalog,
        inputs: IndexMap<String, EvalValue>,
        secrets: IndexMap<String, String>,
        options: &RunOptions,
    ) -> HttpResult<Snapshot> {
        self.run_with_prior(
            recipe,
            catalog,
            inputs,
            secrets,
            options,
            PriorRecords::default(),
        )
        .await
    }

    /// Same as `run`, but seeded with a stream of upstream records.
    /// Composition stages 2+ feed the prior stage's records here; the
    /// engine binds them to the recipe's input slot whose declared
    /// type matches `prior.type_name` (either `[T]` for batched
    /// consumption or `T` for single-record consumption).
    ///
    /// When `prior.records` is empty the engine behaves identically
    /// to `run` — composition stage 1 (with no upstream) and every
    /// non-composed recipe call land here.
    pub async fn run_with_prior(
        &self,
        recipe: &ForageFile,
        catalog: &TypeCatalog,
        mut inputs: IndexMap<String, EvalValue>,
        secrets: IndexMap<String, String>,
        options: &RunOptions,
        prior: PriorRecords,
    ) -> HttpResult<Snapshot> {
        let started = Instant::now();
        // The HTTP engine only runs recipe-bearing files; the validator
        // (`RecipeContextWithoutHeader`) makes sure every caller has
        // already rejected header-less files. Pull the name once here.
        let recipe_name = recipe
            .recipe_name()
            .expect("HTTP engine called with a header-less file");
        debug!(recipe = %recipe_name, "▶ run started");
        self.emit(RunEvent::RunStarted {
            recipe: recipe_name.to_string(),
            replay: false,
        });

        // Bind the prior records into the input slot whose declared
        // type matches the upstream output. The validator's
        // `IncompatiblePipeStage` rule should have caught the
        // mismatch before we get here — but a missing slot at run
        // time is still a real error, so fail loudly rather than
        // silently dropping records.
        if !prior.records.is_empty() {
            bind_prior_records(recipe, &prior, &mut inputs)?;
        }

        self.run_inner(recipe, catalog, inputs, secrets, options)
            .await
            .inspect(|snap| {
                debug!(
                    recipe = %recipe_name,
                    records = snap.records.len(),
                    duration_ms = started.elapsed().as_millis() as u64,
                    "✓ run succeeded"
                );
                self.emit(RunEvent::RunSucceeded {
                    records: snap.records.len(),
                    duration_ms: started.elapsed().as_millis() as u64,
                });
            })
            .inspect_err(|e| {
                debug!(
                    recipe = %recipe_name,
                    error = %e,
                    duration_ms = started.elapsed().as_millis() as u64,
                    "✗ run failed"
                );
                self.emit(RunEvent::RunFailed {
                    error: e.to_string(),
                    duration_ms: started.elapsed().as_millis() as u64,
                });
            })
    }

    async fn run_inner(
        &self,
        recipe: &ForageFile,
        catalog: &TypeCatalog,
        inputs: IndexMap<String, EvalValue>,
        secrets: IndexMap<String, String>,
        options: &RunOptions,
    ) -> HttpResult<Snapshot> {
        let registry =
            TransformRegistry::with_user_fns(default_registry(), recipe.functions.clone());
        let evaluator = Evaluator::new(&registry);
        let transport_ctx =
            EngineTransportContext::new(self.transport, self.config.user_agent.clone());
        let mut scope = Scope::new().with_inputs(inputs).with_secrets(secrets);
        let mut snapshot = Snapshot::new();
        // Stamp every type the recipe could emit onto the snapshot at
        // run boundary so JSON-LD serialization and hub indexing read
        // alignment metadata for workspace-shared and hub-dep types
        // too — not just the ones declared in the recipe file itself.
        snapshot.set_record_types(catalog.types_sorted_effective().iter());
        // Default `$page` so recipes that use `{$page}` outside a paginated
        // step (or before the first request) still have it bound. The
        // engine overwrites this inside each `run_step` iteration.
        scope.bind("page", EvalValue::Int(1));

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
        if let Some(a) = recipe.auth.as_ref() {
            self.emit(RunEvent::Auth {
                flavor: auth_flavor(a).into(),
                status: "ok".into(),
            });
        }

        let mut requests_made: u32 = 0;
        let mut emit_counts: IndexMap<String, usize> = IndexMap::new();
        let mut step_index: usize = 0;
        let mut emit_index: usize = 0;
        let mut step_responses: IndexMap<String, StepResponse> = IndexMap::new();
        // Build the line map once per run. Pause sites resolve each
        // statement's byte span to a 0-based line via this; that's what
        // the studio's gutter clicks key on. `recipe.source` is `""`
        // for hand-constructed / JSON-round-tripped ASTs, in which case
        // every span maps to line 0 — see ForageFile docs.
        let line_map = LineMap::new(&recipe.source);
        self.run_statements(
            recipe.body.statements(),
            recipe,
            &auth_state,
            &evaluator,
            &transport_ctx,
            &mut scope,
            &mut snapshot,
            &mut requests_made,
            &mut emit_counts,
            &mut step_index,
            &mut emit_index,
            &mut step_responses,
            &line_map,
            options,
            // Top-level body: outermost for-loops here are the sample
            // unit. Inside `run_statements`, a recursive call inside a
            // for-loop's body flips this to false.
            true,
        )
        .await?;
        // No source on hand at engine boundary — line annotations get
        // filled in by callers that have both the recipe and its
        // source text (CLI, Studio commands).
        snapshot.evaluate_expectations(&recipe.expectations, None);
        Ok(snapshot)
    }

    #[allow(clippy::too_many_arguments)]
    fn run_statements<'a>(
        &'a self,
        body: &'a [Statement],
        recipe: &'a ForageFile,
        auth_state: &'a AuthState,
        evaluator: &'a Evaluator<'a>,
        transport_ctx: &'a EngineTransportContext<'a>,
        scope: &'a mut Scope,
        snapshot: &'a mut Snapshot,
        requests_made: &'a mut u32,
        emit_counts: &'a mut IndexMap<String, usize>,
        step_index: &'a mut usize,
        emit_index: &'a mut usize,
        step_responses: &'a mut IndexMap<String, StepResponse>,
        line_map: &'a LineMap,
        options: &'a RunOptions,
        top_level: bool,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = HttpResult<()>> + Send + 'a>> {
        Box::pin(async move {
            for s in body {
                match s {
                    Statement::Step(step) => {
                        let start_line = line_map.range(step.span.clone()).start.line;
                        if let Some(dbg) = self.debugger.clone() {
                            let pause = StepPause {
                                step: step.name.clone(),
                                step_index: *step_index,
                                start_line,
                                scope: DebugScope::from_scope(
                                    scope,
                                    &recipe.secrets,
                                    emit_counts,
                                    step_responses,
                                ),
                            };
                            match dbg.before_step(pause, scope).await {
                                ResumeAction::Continue
                                | ResumeAction::StepOver
                                | ResumeAction::StepIn => {}
                                ResumeAction::Stop => {
                                    return Err(HttpError::Generic("stopped by debugger".into()));
                                }
                            }
                        }
                        *step_index += 1;
                        self.run_step(
                            step,
                            recipe,
                            auth_state,
                            evaluator,
                            scope,
                            requests_made,
                            step_responses,
                        )
                        .await?;
                    }
                    Statement::Emit(em) => {
                        let start_line = line_map.range(em.span.clone()).start.line;
                        if let Some(dbg) = self.debugger.clone() {
                            let pause = EmitPause {
                                type_name: em.type_name.clone(),
                                emit_index: *emit_index,
                                start_line,
                                scope: DebugScope::from_scope(
                                    scope,
                                    &recipe.secrets,
                                    emit_counts,
                                    step_responses,
                                ),
                            };
                            match dbg.before_emit(pause, scope).await {
                                ResumeAction::Continue
                                | ResumeAction::StepOver
                                | ResumeAction::StepIn => {}
                                ResumeAction::Stop => {
                                    return Err(HttpError::Generic("stopped by debugger".into()));
                                }
                            }
                        }
                        *emit_index += 1;
                        self.run_emit(em, evaluator, transport_ctx, scope, snapshot, emit_counts)
                            .await?;
                    }
                    Statement::ForLoop {
                        variable,
                        collection,
                        body,
                        span,
                    } => {
                        let start_line = line_map.range(span.clone()).start.line;
                        let collection_val = evaluator
                            .eval_extraction_async(collection, scope, transport_ctx)
                            .await?;
                        let mut items = match collection_val {
                            EvalValue::Array(xs) => xs,
                            EvalValue::NodeList(xs) => {
                                xs.into_iter().map(EvalValue::Node).collect()
                            }
                            EvalValue::Null => Vec::new(),
                            other => vec![other],
                        };
                        if top_level {
                            options.cap_top_level(&mut items);
                        }
                        let total = items.len();
                        if let Some(dbg) = self.debugger.clone() {
                            let pause = ForLoopPause {
                                variable: variable.clone(),
                                total,
                                start_line,
                                scope: DebugScope::from_scope(
                                    scope,
                                    &recipe.secrets,
                                    emit_counts,
                                    step_responses,
                                ),
                            };
                            match dbg.before_for_loop(pause, scope).await {
                                ResumeAction::Continue
                                | ResumeAction::StepOver
                                | ResumeAction::StepIn => {}
                                ResumeAction::Stop => {
                                    return Err(HttpError::Generic("stopped by debugger".into()));
                                }
                            }
                        }
                        for (idx, item) in items.into_iter().enumerate() {
                            scope.push_frame();
                            scope.bind(variable, item.clone());
                            let saved_current = scope.current.clone();
                            scope.current = Some(item);
                            if let Some(dbg) = self.debugger.clone() {
                                let pause = IterationPause {
                                    variable: variable.clone(),
                                    iteration: idx,
                                    total,
                                    start_line,
                                    scope: DebugScope::from_scope(
                                        scope,
                                        &recipe.secrets,
                                        emit_counts,
                                        step_responses,
                                    ),
                                };
                                match dbg.before_iteration(pause, scope).await {
                                    ResumeAction::Continue
                                    | ResumeAction::StepOver
                                    | ResumeAction::StepIn => {}
                                    ResumeAction::Stop => {
                                        scope.current = saved_current;
                                        scope.pop_frame();
                                        return Err(HttpError::Generic(
                                            "stopped by debugger".into(),
                                        ));
                                    }
                                }
                            }
                            self.run_statements(
                                body,
                                recipe,
                                auth_state,
                                evaluator,
                                transport_ctx,
                                scope,
                                snapshot,
                                requests_made,
                                emit_counts,
                                step_index,
                                emit_index,
                                step_responses,
                                line_map,
                                options,
                                false,
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

    #[allow(clippy::too_many_arguments)]
    async fn run_step(
        &self,
        step: &HTTPStep,
        recipe: &ForageFile,
        auth_state: &AuthState,
        evaluator: &Evaluator<'_>,
        scope: &mut Scope,
        requests_made: &mut u32,
        step_responses: &mut IndexMap<String, StepResponse>,
    ) -> HttpResult<()> {
        let mut driver = step.pagination.as_ref().map(PaginationDriver::new);
        let mut extra_query: Vec<(String, String)> = Vec::new();
        // Accumulator for paginated items. The strategy's `items_path` is
        // evaluated against each page's response body; the flattened list
        // becomes the step's final bound value so `$step[*]` works the same
        // as it does for non-paginated steps that return a top-level array.
        let mut accumulated_items: Vec<EvalValue> = Vec::new();
        let mut page: u32 = 1;
        let zero_indexed = matches!(
            step.pagination,
            Some(Pagination::PageWithTotal {
                page_zero_indexed: true,
                ..
            }) | Some(Pagination::UntilEmpty {
                page_zero_indexed: true,
                ..
            })
        );

        loop {
            if *requests_made >= self.config.max_requests {
                return Err(HttpError::Generic(format!(
                    "exceeded max_requests ({})",
                    self.config.max_requests
                )));
            }
            *requests_made += 1;

            // Bind `$page` in scope so the recipe can template the page
            // number into the request body or URL via `{$page}`. Necessary
            // for recipes whose pagination param lives in a POST body
            // (Leafbridge) — appending to the URL doesn't reach the server.
            let bound_page = if zero_indexed && page > 0 {
                page - 1
            } else {
                page
            };
            scope.bind("page", EvalValue::Int(bound_page as i64));

            let req =
                self.build_request(step, recipe, auth_state, evaluator, scope, &extra_query)?;
            let body_size = req.body.as_ref().map(|b| b.len()).unwrap_or(0);
            debug!(
                step = %step.name,
                page = page,
                method = %req.method,
                url = %req.url,
                body_size = body_size,
                "→ request"
            );
            if body_size > 0 {
                trace!(
                    step = %step.name,
                    body = %preview_bytes(req.body.as_deref().unwrap_or(&[]), 500),
                    "→ request body"
                );
            }
            self.emit(RunEvent::RequestSent {
                step: step.name.clone(),
                method: req.method.clone(),
                url: req.url.clone(),
                page,
            });

            let req_started = Instant::now();
            let resp = self.transport.fetch(req.clone()).await?;
            let duration_ms = req_started.elapsed().as_millis() as u64;
            debug!(
                step = %step.name,
                page = page,
                status = resp.status,
                bytes = resp.body.len(),
                duration_ms = duration_ms,
                "← response"
            );
            trace!(
                step = %step.name,
                body = %preview_bytes(&resp.body, 500),
                "← response body"
            );
            self.emit(RunEvent::ResponseReceived {
                step: step.name.clone(),
                status: resp.status,
                duration_ms,
                bytes: resp.body.len(),
            });
            // Capture the response BEFORE the status gate so 4xx/5xx
            // responses still land in `step_responses` for the debug
            // panel. The recipe will abort right after — but the user
            // needs to see the response body to know why.
            //
            // The full (uncapped) bytes go through the progress sink's
            // `step_response_full_body` hook so a host can stash them
            // somewhere outside the wire payload. The wire-side
            // `body_raw` is truncated to `BODY_CAPTURE_MAX` to keep
            // pause-time IPC bounded.
            let resolved_format = resolve_parse_format(step, &resp);
            let content_type_header = normalized_content_type(&resp);
            let (body_raw, body_truncated) = truncate_body_lossy(&resp.body);
            self.progress
                .step_response_full_body(&step.name, &resp.body);
            let captured = StepResponse {
                status: resp.status,
                headers: resp.headers.clone(),
                body_raw,
                body_truncated,
                format: resolved_format,
                content_type_header,
            };
            self.progress.step_response_captured(&step.name, &captured);
            step_responses.insert(step.name.clone(), captured);
            if !(200..400).contains(&resp.status) {
                return Err(HttpError::Status {
                    status: resp.status,
                    url: req.url,
                });
            }

            let body_val = parse_response_body(step, &resp)?;

            // Bind `$<stepName>` to the response body for downstream eval —
            // pagination accumulation overrides this at the end of the loop.
            scope.bind(&step.name, body_val.clone());
            scope.current = Some(body_val.clone());

            // extract.regex { pattern, groups } — bind each group name from
            // the response body. Used by Leafbridge-style auth.htmlPrime.
            if let Some(ex) = &step.extract {
                let body_str = resp.body_str();
                apply_regex_extract(ex, body_str, scope)?;
                for g in &ex.groups {
                    debug!(
                        step = %step.name,
                        var = %g,
                        value = %preview_value(scope.lookup(g), 80),
                        "extract.regex bound"
                    );
                }
            }

            // If pagination is declared, append this page's items to the
            // accumulator before driving to the next page.
            if let Some(pag) = &step.pagination {
                let items = items_for_page(pag, evaluator, scope)?;
                debug!(
                    step = %step.name,
                    page = page,
                    items_this_page = items.len(),
                    items_total = accumulated_items.len() + items.len(),
                    "paginate: page items"
                );
                accumulated_items.extend(items);
            }

            match driver.as_mut() {
                Some(d) => match d.advance(evaluator, scope)? {
                    NextPage::Stop => {
                        debug!(
                            step = %step.name,
                            pages = page,
                            total_items = accumulated_items.len(),
                            "paginate: stop"
                        );
                        // Re-bind `$<stepName>` to the accumulated items so
                        // downstream `$<stepName>[*]` iterates across pages.
                        scope.bind(&step.name, EvalValue::Array(accumulated_items.clone()));
                        scope.current = Some(EvalValue::Array(accumulated_items));
                        return Ok(());
                    }
                    NextPage::Continue(params) => {
                        debug!(
                            step = %step.name,
                            next_page = page + 1,
                            params = ?params,
                            "paginate: continue"
                        );
                        extra_query = params;
                        page += 1;
                    }
                },
                None => return Ok(()),
            }
        }
    }

    async fn run_emit(
        &self,
        em: &Emission,
        evaluator: &Evaluator<'_>,
        transport_ctx: &EngineTransportContext<'_>,
        scope: &mut Scope,
        snapshot: &mut Snapshot,
        emit_counts: &mut IndexMap<String, usize>,
    ) -> HttpResult<()> {
        let mut fields: IndexMap<String, JSONValue> = IndexMap::new();
        for b in &em.bindings {
            let v = evaluator
                .eval_extraction_async(&b.expr, scope, transport_ctx)
                .await?;
            fields.insert(b.field_name.clone(), v.into_json());
        }
        let id = snapshot.next_record_id();
        // If the emit was post-fixed with `as $v`, bind a `Ref` value
        // for the freshly-emitted record into the current scope so
        // sibling emits can reference it. The validator guarantees the
        // identifier is well-formed and not shadowing.
        if let Some(name) = &em.bind_name {
            scope.bind(
                name,
                EvalValue::Ref {
                    target_type: em.type_name.clone(),
                    id: id.clone(),
                },
            );
        }
        snapshot.emit(Record {
            id,
            type_name: em.type_name.clone(),
            fields,
        });
        let count = emit_counts.entry(em.type_name.clone()).or_insert(0);
        *count += 1;
        trace!(type_name = %em.type_name, total = *count, "emit");
        self.emit(RunEvent::Emitted {
            type_name: em.type_name.clone(),
            total: *count,
        });
        Ok(())
    }

    fn build_request(
        &self,
        step: &HTTPStep,
        recipe: &ForageFile,
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

/// First `max_len` UTF-8 chars of `bytes`, with newlines/tabs escaped and a
/// "…+N more" suffix if truncated. For HTTP body previews in logs.
fn preview_bytes(bytes: &[u8], max_len: usize) -> String {
    let s = std::str::from_utf8(bytes).unwrap_or("<binary>");
    preview_str(s, max_len)
}

fn preview_str(s: &str, max_len: usize) -> String {
    let total = s.len();
    let mut out = String::with_capacity(max_len.min(total) + 16);
    for (i, ch) in s.chars().enumerate() {
        if i >= max_len {
            out.push_str(&format!("…+{}B", total.saturating_sub(out.len())));
            return out;
        }
        match ch {
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(ch),
        }
    }
    out
}

fn preview_value(v: Option<&EvalValue>, max_len: usize) -> String {
    match v {
        None => "<unbound>".into(),
        Some(EvalValue::String(s)) => preview_str(s, max_len),
        Some(other) => preview_str(&format!("{other:?}"), max_len),
    }
}

fn auth_flavor(a: &AuthStrategy) -> &'static str {
    match a {
        AuthStrategy::StaticHeader { .. } => "staticHeader",
        AuthStrategy::HtmlPrime { .. } => "htmlPrime",
        AuthStrategy::Session(_) => "session",
    }
}

fn items_for_page(
    pag: &Pagination,
    ev: &Evaluator<'_>,
    scope: &Scope,
) -> HttpResult<Vec<EvalValue>> {
    let path = match pag {
        Pagination::PageWithTotal { items_path, .. } => items_path,
        Pagination::UntilEmpty { items_path, .. } => items_path,
        Pagination::Cursor { items_path, .. } => items_path,
    };
    match ev.eval_path(path, scope)? {
        EvalValue::Array(xs) => Ok(xs),
        EvalValue::NodeList(xs) => Ok(xs.into_iter().map(EvalValue::Node).collect()),
        EvalValue::Null => Ok(Vec::new()),
        other => Ok(vec![other]),
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

/// Bind a batch of upstream records into the recipe's matching input
/// slot. Looks for an `input <name>: [T]` decl (the batched form) or
/// `input <name>: T` (single-record consumption); errors when no slot
/// matches `prior.type_name`. The validator should reject this shape
/// before reaching the engine — when it does fire here, it means a
/// caller skipped validation or an unsigned recipe was deployed.
fn bind_prior_records(
    recipe: &ForageFile,
    prior: &PriorRecords,
    inputs: &mut IndexMap<String, EvalValue>,
) -> HttpResult<()> {
    let upstream = prior.type_name.as_str();
    let batched = recipe.inputs.iter().find(|i| match &i.ty {
        FieldType::Array(inner) => matches!(inner.as_ref(), FieldType::Record(n) if n == upstream),
        _ => false,
    });
    if let Some(decl) = batched {
        let values: Vec<EvalValue> = prior.records.iter().map(EvalValue::from).collect();
        inputs.insert(decl.name.clone(), EvalValue::Array(values));
        return Ok(());
    }
    let single = recipe
        .inputs
        .iter()
        .find(|i| matches!(&i.ty, FieldType::Record(n) if n == upstream));
    if let Some(decl) = single {
        let only = match prior.records.len() {
            0 => return Ok(()),
            1 => EvalValue::from(&prior.records[0]),
            n => {
                return Err(HttpError::Generic(format!(
                    "recipe '{}' declares input '{}' as a single {} but the upstream stage emitted {} records; declare the input as `[{}]` to consume them as a batch",
                    recipe.recipe_name().unwrap_or("<unknown>"),
                    decl.name,
                    upstream,
                    n,
                    upstream,
                )));
            }
        };
        inputs.insert(decl.name.clone(), only);
        return Ok(());
    }
    Err(HttpError::Generic(format!(
        "recipe '{}' has no input slot for upstream type '{}'; declare `input <name>: [{}]` (or `: {}`) to receive records from the prior stage",
        recipe.recipe_name().unwrap_or("<unknown>"),
        upstream,
        upstream,
        upstream,
    )))
}

fn parse_response_body(step: &HTTPStep, resp: &HttpResponse) -> HttpResult<EvalValue> {
    let body = resp.body_str();
    if body.is_empty() {
        return Ok(EvalValue::Null);
    }
    // The recipe's `parse : <fmt>` override (when present) takes
    // priority over content-type detection. Without an override we
    // keep the historical fallback: try JSON first (covers
    // text/plain-tagged JSON), then drop to a raw string.
    let format = resolve_parse_format(step, resp);
    match format {
        ParseFormat::Json => match serde_json::from_str::<serde_json::Value>(body) {
            Ok(v) => Ok((&v).into()),
            Err(_) => Ok(EvalValue::String(body.into())),
        },
        ParseFormat::Html | ParseFormat::Xml | ParseFormat::Text => {
            // The engine's existing $<step> binding for these formats
            // is the raw body string — DOM walking happens at the
            // recipe expression level via the dom-* transforms.
            Ok(EvalValue::String(body.into()))
        }
    }
}

/// Resolve the parse format for a step's response: recipe-level
/// override > Content-Type detection > JSON-first fallback for
/// headerless responses (replay fixtures often omit headers entirely).
fn resolve_parse_format(step: &HTTPStep, resp: &HttpResponse) -> ParseFormat {
    if let Some(f) = step.parse {
        return f;
    }
    match normalized_content_type(resp) {
        Some(mime) => ParseFormat::from_content_type(&mime),
        None => {
            // No Content-Type at all (raw HTTP capture / replay fixture):
            // try JSON first as a generous default so existing
            // recipes that don't declare `parse :` keep working
            // against pre-parse-format fixtures.
            let body = resp.body_str();
            if !body.is_empty() && serde_json::from_str::<serde_json::Value>(body).is_ok() {
                ParseFormat::Json
            } else {
                ParseFormat::Text
            }
        }
    }
}

/// Lower-case the `Content-Type` header (case-insensitive lookup) and
/// drop any `; charset=…` suffix. Returns `None` when the response
/// carried no `Content-Type` header.
fn normalized_content_type(resp: &HttpResponse) -> Option<String> {
    for (k, v) in &resp.headers {
        if k.eq_ignore_ascii_case("content-type") {
            let trimmed = v.split(';').next().unwrap_or("").trim();
            if trimmed.is_empty() {
                return None;
            }
            return Some(trimmed.to_ascii_lowercase());
        }
    }
    None
}

/// UTF-8-lossy decode of the response body, capped at
/// `BODY_CAPTURE_MAX` chars. Returns `(body_raw, body_truncated)` —
/// the wire-side fields the debugger UI reads.
fn truncate_body_lossy(bytes: &[u8]) -> (String, bool) {
    let s = String::from_utf8_lossy(bytes);
    if s.len() <= BODY_CAPTURE_MAX {
        return (s.into_owned(), false);
    }
    // Slice on a char boundary so the truncated body remains valid
    // UTF-8 for the wire and for downstream JSON serialization.
    let mut end = BODY_CAPTURE_MAX;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    (s[..end].to_string(), true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::ReplayTransport;
    use forage_core::parse;
    use forage_replay::{Capture, HttpExchange};

    /// Build a lonely-file catalog for tests that don't go through the
    /// workspace loader. Real call sites (CLI, daemon, Studio) get the
    /// merged catalog from `Workspace::catalog`.
    fn lonely_catalog(recipe: &ForageFile) -> TypeCatalog {
        TypeCatalog::from_file(recipe)
    }

    #[tokio::test]
    async fn runs_one_step_recipe_against_replay() {
        let src = r#"
            recipe "tiny"
            engine http
            type Item { id: String }
            step list {
                method "GET"
                url "https://api.example.com/items"
            }
            for $i in $list.items[*] {
                emit Item { id ← $i.id }
            }
        "#;
        let recipe = parse(src).unwrap();
        let catalog = lonely_catalog(&recipe);
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
            .run(
                &recipe,
                &catalog,
                IndexMap::new(),
                IndexMap::new(),
                &RunOptions::default(),
            )
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
    async fn paginated_step_binds_accumulated_items() {
        // Reproduces the Sweed bug: a paginated step whose response is an
        // object `{list: [...], total: N}` must bind `$<step>` to the
        // flattened items list across pages, not the last page's body.
        let src = r#"
            recipe "paged"
            engine http
            type Item { id: String }
            step products {
                method "GET"
                url "https://api.example.com/items"
                paginate pageWithTotal {
                    items: $.list, total: $.total,
                    pageParam: "page", pageSize: 2
                }
            }
            for $p in $products[*] {
                emit Item { id ← $p.id }
            }
        "#;
        let recipe = parse(src).unwrap();
        let catalog = lonely_catalog(&recipe);
        let page1 = Capture::Http(HttpExchange {
            url: "https://api.example.com/items".into(),
            method: "GET".into(),
            request_headers: IndexMap::new(),
            request_body: None,
            status: 200,
            response_headers: IndexMap::new(),
            body: r#"{"list":[{"id":"a"},{"id":"b"}],"total":4}"#.into(),
        });
        let page2 = Capture::Http(HttpExchange {
            url: "https://api.example.com/items?page=2&pageSize=2".into(),
            method: "GET".into(),
            request_headers: IndexMap::new(),
            request_body: None,
            status: 200,
            response_headers: IndexMap::new(),
            body: r#"{"list":[{"id":"c"},{"id":"d"}],"total":4}"#.into(),
        });
        let transport = ReplayTransport::new(vec![page1, page2]);
        let engine = Engine::new(&transport);
        let snap = engine
            .run(
                &recipe,
                &catalog,
                IndexMap::new(),
                IndexMap::new(),
                &RunOptions::default(),
            )
            .await
            .unwrap();
        assert_eq!(snap.records.len(), 4);
        let ids: Vec<_> = snap
            .records
            .iter()
            .map(|r| match r.fields.get("id") {
                Some(JSONValue::String(s)) => s.as_str(),
                _ => "?",
            })
            .collect();
        assert_eq!(ids, vec!["a", "b", "c", "d"]);
    }

    #[tokio::test]
    async fn page_binding_templates_into_form_body() {
        // Regression: Leafbridge sends the page number in a form body; the
        // engine's URL-append doesn't reach the server, so the recipe must
        // template `{$page}` into the body. The engine has to bind `$page`
        // before each request build so the template re-renders per page.
        use crate::transport::HttpRequest;
        use async_trait::async_trait;
        use std::sync::Mutex;

        struct RecordingTransport {
            pub seen: Mutex<Vec<HttpRequest>>,
            pub pages: Vec<&'static str>,
            pub idx: Mutex<usize>,
        }

        #[async_trait]
        impl Transport for RecordingTransport {
            async fn fetch(
                &self,
                req: HttpRequest,
            ) -> crate::error::HttpResult<crate::transport::HttpResponse> {
                let mut idx = self.idx.lock().unwrap();
                let body = self.pages.get(*idx).copied().unwrap_or("[]");
                *idx += 1;
                self.seen.lock().unwrap().push(req);
                Ok(crate::transport::HttpResponse {
                    status: 200,
                    headers: IndexMap::new(),
                    body: body.as_bytes().to_vec(),
                })
            }
        }

        let src = r#"
            recipe "leafy"
            engine http
            type Item { id: String }
            step products {
                method "POST"
                url "https://example.com/ajax"
                body.form {
                    "page": "{$page}"
                }
                paginate untilEmpty {
                    items: $.list, pageParam: "page"
                }
            }
            for $p in $products[*] {
                emit Item { id ← $p.id }
            }
        "#;
        let recipe = parse(src).unwrap();
        let catalog = lonely_catalog(&recipe);
        let transport = RecordingTransport {
            seen: Mutex::new(Vec::new()),
            pages: vec![
                r#"{"list":[{"id":"a"}]}"#,
                r#"{"list":[{"id":"b"}]}"#,
                r#"{"list":[]}"#,
            ],
            idx: Mutex::new(0),
        };
        let engine = Engine::new(&transport);
        let snap = engine
            .run(
                &recipe,
                &catalog,
                IndexMap::new(),
                IndexMap::new(),
                &RunOptions::default(),
            )
            .await
            .unwrap();
        assert_eq!(snap.records.len(), 2);

        // Each request's body should have the corresponding page number.
        let seen = transport.seen.lock().unwrap();
        assert_eq!(seen.len(), 3);
        let body_str = |r: &HttpRequest| -> String {
            String::from_utf8(r.body.clone().unwrap_or_default()).unwrap()
        };
        assert!(body_str(&seen[0]).contains("page=1"));
        assert!(body_str(&seen[1]).contains("page=2"));
        assert!(body_str(&seen[2]).contains("page=3"));
    }

    #[tokio::test]
    async fn progress_events_fire_in_order() {
        // The Studio "Run live" UX depends on these events firing in real
        // time. Regression for the silent-run problem: a 30-second
        // paginated run with no feedback is indistinguishable from a hang.
        use crate::progress::{CaptureSink, RunEvent};
        use std::sync::Arc;

        let src = r#"
            recipe "events"
            engine http
            type Item { id: String }
            step list {
                method "GET"
                url "https://api.example.com/items"
            }
            for $i in $list.items[*] {
                emit Item { id ← $i.id }
            }
        "#;
        let recipe = parse(src).unwrap();
        let catalog = lonely_catalog(&recipe);
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
        let sink = Arc::new(CaptureSink::default());
        let engine = Engine::new(&transport).with_progress(sink.clone());
        let snap = engine
            .run(
                &recipe,
                &catalog,
                IndexMap::new(),
                IndexMap::new(),
                &RunOptions::default(),
            )
            .await
            .unwrap();
        assert_eq!(snap.records.len(), 2);
        let events = sink.snapshot();
        let kinds: Vec<&str> = events
            .iter()
            .map(|e| match e {
                RunEvent::RunStarted { .. } => "run_started",
                RunEvent::Auth { .. } => "auth",
                RunEvent::RequestSent { .. } => "request_sent",
                RunEvent::ResponseReceived { .. } => "response_received",
                RunEvent::Emitted { .. } => "emitted",
                RunEvent::RunSucceeded { .. } => "run_succeeded",
                RunEvent::RunFailed { .. } => "run_failed",
            })
            .collect();
        assert_eq!(
            kinds,
            vec![
                "run_started",
                "request_sent",
                "response_received",
                "emitted",
                "emitted",
                "run_succeeded",
            ]
        );
        // The Emitted events carry the running total per type.
        let emits: Vec<usize> = events
            .iter()
            .filter_map(|e| match e {
                RunEvent::Emitted { total, .. } => Some(*total),
                _ => None,
            })
            .collect();
        assert_eq!(emits, vec![1, 2]);
        // RunSucceeded carries the final record count.
        match events.last().unwrap() {
            RunEvent::RunSucceeded { records, .. } => assert_eq!(*records, 2),
            other => panic!("expected RunSucceeded last, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn progress_emits_run_failed_on_error() {
        use crate::progress::{CaptureSink, RunEvent};
        use std::sync::Arc;

        let src = r#"
            recipe "broken"
            engine http
            type T { x: String }
            step go {
                method "GET"
                url "https://api.example.com/missing"
            }
            emit T { x ← "hi" }
        "#;
        let recipe = parse(src).unwrap();
        let catalog = lonely_catalog(&recipe);
        let transport = ReplayTransport::new(vec![]);
        let sink = Arc::new(CaptureSink::default());
        let engine = Engine::new(&transport).with_progress(sink.clone());
        let err = engine
            .run(
                &recipe,
                &catalog,
                IndexMap::new(),
                IndexMap::new(),
                &RunOptions::default(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, HttpError::NoFixture { .. }));
        let events = sink.snapshot();
        // Should start with run_started and end with run_failed.
        assert!(matches!(events.first(), Some(RunEvent::RunStarted { .. })));
        assert!(matches!(events.last(), Some(RunEvent::RunFailed { .. })));
    }

    #[tokio::test]
    async fn leafbridge_flow_prime_then_paginated_per_menu_type() {
        // Integration regression for the Leafbridge pattern used by the
        // remedy-* / zen-leaf-* recipes. Exercises:
        //   - auth.htmlPrime: GET menu page, regex-extract $ajaxUrl + $ajaxNonce
        //   - for $menu in $input.menuTypes: paginated POSTs
        //   - body templating with {$page} (without it, the loop would never
        //     terminate because the body would always say page=1)
        //   - $page resets to 1 at the start of each step invocation
        //
        // The mock transport returns 2 pages of 2 products per menu type, with
        // page 3 returning an empty list to terminate untilEmpty pagination.
        use crate::transport::{HttpRequest, HttpResponse};
        use async_trait::async_trait;
        use std::sync::Mutex;

        struct LeafbridgeMock {
            seen: Mutex<Vec<HttpRequest>>,
        }

        #[async_trait]
        impl Transport for LeafbridgeMock {
            async fn fetch(&self, req: HttpRequest) -> HttpResult<HttpResponse> {
                self.seen.lock().unwrap().push(req.clone());

                if req.method == "GET" {
                    let html = r#"<html><script>
                        var leafbridge_public_ajax_obj = {"ajaxurl":"https://remedy.test/wp-admin/admin-ajax.php","nonce":"deadbeef1234"};
                    </script></html>"#;
                    return Ok(HttpResponse {
                        status: 200,
                        headers: IndexMap::new(),
                        body: html.as_bytes().to_vec(),
                    });
                }

                let body =
                    String::from_utf8(req.body.clone().unwrap_or_default()).expect("utf8 body");
                let page: u32 = form_field(&body, "prods_pageNumber")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                let menu_type = form_field(&body, "wizard_data%5Bmenu_type%5D").unwrap_or_default();

                let products_json = if page <= 2 {
                    format!(
                        r#"[{{"id":"{menu_type}-p{page}-a","name":"{menu_type} P{page}A"}},{{"id":"{menu_type}-p{page}-b","name":"{menu_type} P{page}B"}}]"#,
                    )
                } else {
                    "[]".into()
                };
                let body = format!(r#"{{"data":{{"products_list":{products_json}}}}}"#);
                Ok(HttpResponse {
                    status: 200,
                    headers: IndexMap::new(),
                    body: body.into_bytes(),
                })
            }
        }

        fn form_field(form: &str, key: &str) -> Option<String> {
            let prefix = format!("{key}=");
            for kv in form.split('&') {
                if let Some(rest) = kv.strip_prefix(&prefix) {
                    return Some(rest.to_string());
                }
            }
            None
        }

        let src = r#"
            recipe "leafbridge-flow"
            engine http
            type Product { id: String, name: String }
            enum MenuType { RECREATIONAL, MEDICAL }

            input menuPageURL: String
            input menuTypes: [MenuType]
            input retailerId: String

            auth.htmlPrime {
                step:        prime
                nonceVar:    "ajaxNonce"
                ajaxUrlVar:  "ajaxUrl"
            }

            step prime {
                method "GET"
                url    "{$input.menuPageURL}"
                extract.regex {
                    pattern: "leafbridge_public_ajax_obj\\s*=\\s*\\{\"ajaxurl\":\"([^\"]+)\",\"nonce\":\"([a-f0-9]+)\"\\}"
                    groups: [ajaxUrl, ajaxNonce]
                }
            }

            for $menu in $input.menuTypes {
                step products {
                    method "POST"
                    url    "{$ajaxUrl}"
                    body.form {
                        "nonce_ajax":                          "{$ajaxNonce}"
                        "wizard_data[retailer_id]":            "{$input.retailerId}"
                        "wizard_data[menu_type]":              case $menu of {
                                                                   RECREATIONAL → "RECREATIONAL"
                                                                   MEDICAL      → "MEDICAL"
                                                               }
                        "prods_pageNumber":                    "{$page}"
                    }
                    paginate untilEmpty {
                        items:     $.data.products_list
                        pageParam: "prods_pageNumber"
                    }
                }

                for $p in $products[*] {
                    emit Product { id ← $p.id, name ← $p.name }
                }
            }
        "#;
        let recipe = parse(src).expect("recipe parses");
        let catalog = forage_core::TypeCatalog::from_file(&recipe);
        let validation =
            forage_core::validate(&recipe, &catalog, &forage_core::RecipeSignatures::default());
        assert!(
            !validation.has_errors(),
            "validation errors: {validation:?}"
        );

        let inputs: IndexMap<String, EvalValue> = [
            (
                "menuPageURL".into(),
                EvalValue::String("https://remedy.test/menu".into()),
            ),
            (
                "menuTypes".into(),
                EvalValue::Array(vec![
                    EvalValue::String("RECREATIONAL".into()),
                    EvalValue::String("MEDICAL".into()),
                ]),
            ),
            ("retailerId".into(), EvalValue::String("uuid-1234".into())),
        ]
        .into_iter()
        .collect();

        let transport = LeafbridgeMock {
            seen: Mutex::new(Vec::new()),
        };
        let snap = Engine::new(&transport)
            .run(
                &recipe,
                &catalog,
                inputs,
                IndexMap::new(),
                &RunOptions::default(),
            )
            .await
            .expect("run ok");

        // 2 menu types × 2 non-empty pages × 2 products = 8 records.
        let products: Vec<_> = snap
            .records
            .iter()
            .filter(|r| r.type_name == "Product")
            .collect();
        assert_eq!(products.len(), 8, "expected 8 products, got {products:?}");

        // 1 prime GET + 2 menu types × 3 POSTs (page 1, 2, 3-empty) = 7 requests.
        let seen = transport.seen.lock().unwrap();
        assert_eq!(seen.len(), 7, "expected 7 requests, got {}", seen.len());
        assert_eq!(seen[0].method, "GET");
        assert!(seen[0].url.contains("/menu"));
        for r in &seen[1..] {
            assert_eq!(r.method, "POST");
            assert!(
                r.url
                    .starts_with("https://remedy.test/wp-admin/admin-ajax.php")
            );
        }

        // Bodies should show $page templating: pages 1,2,3 for RECREATIONAL,
        // then pages 1,2,3 for MEDICAL.
        let body_str = |r: &HttpRequest| -> String {
            String::from_utf8(r.body.clone().unwrap_or_default()).unwrap()
        };
        let expected_pages = [1u32, 2, 3, 1, 2, 3];
        let expected_menus = [
            "RECREATIONAL",
            "RECREATIONAL",
            "RECREATIONAL",
            "MEDICAL",
            "MEDICAL",
            "MEDICAL",
        ];
        for (i, r) in seen[1..].iter().enumerate() {
            let b = body_str(r);
            let p = expected_pages[i];
            let m = expected_menus[i];
            assert!(
                b.contains(&format!("prods_pageNumber={p}")),
                "request {i}: expected prods_pageNumber={p}, body was {b}",
            );
            assert!(
                b.contains(&format!("wizard_data%5Bmenu_type%5D={m}")),
                "request {i}: expected menu_type={m}, body was {b}",
            );
            assert!(
                b.contains("nonce_ajax=deadbeef1234"),
                "request {i}: expected captured nonce in body, was {b}",
            );
        }
    }

    #[tokio::test]
    async fn missing_fixture_errors() {
        let src = r#"
            recipe "tiny"
            engine http
            type T { x: String }
            step go {
                method "GET"
                url "https://api.example.com/nope"
            }
            emit T { x ← "hi" }
        "#;
        let recipe = parse(src).unwrap();
        let catalog = lonely_catalog(&recipe);
        let transport = ReplayTransport::new(vec![]);
        let engine = Engine::new(&transport);
        let err = engine
            .run(
                &recipe,
                &catalog,
                IndexMap::new(),
                IndexMap::new(),
                &RunOptions::default(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, HttpError::NoFixture { .. }));
    }

    #[tokio::test]
    async fn debugger_fires_before_each_step_with_running_scope() {
        // A debugger plugged into the engine must see one pause per step in
        // execution order, with `step_index` monotonically increasing and
        // the scope reflecting bindings produced by prior steps (so the
        // user can inspect `$first` at the second pause).
        use crate::debug::RecordingDebugger;

        let src = r#"
            recipe "twostep"
            engine http
            secret token
            type Item { id: String }
            step first {
                method "GET"
                url "https://api.example.com/a"
            }
            step second {
                method "GET"
                url "https://api.example.com/b"
            }
            for $i in $second.items[*] {
                emit Item { id ← $i.id }
            }
        "#;
        let recipe = parse(src).unwrap();
        let catalog = lonely_catalog(&recipe);
        let first = Capture::Http(HttpExchange {
            url: "https://api.example.com/a".into(),
            method: "GET".into(),
            request_headers: IndexMap::new(),
            request_body: None,
            status: 200,
            response_headers: IndexMap::new(),
            body: r#"{"marker":"FIRST"}"#.into(),
        });
        let second = Capture::Http(HttpExchange {
            url: "https://api.example.com/b".into(),
            method: "GET".into(),
            request_headers: IndexMap::new(),
            request_body: None,
            status: 200,
            response_headers: IndexMap::new(),
            body: r#"{"items":[{"id":"x"}]}"#.into(),
        });
        let transport = ReplayTransport::new(vec![first, second]);
        let dbg = Arc::new(RecordingDebugger::new(vec![
            ResumeAction::Continue,
            ResumeAction::Continue,
        ]));
        let engine = Engine::new(&transport).with_debugger(dbg.clone());

        let mut secrets = IndexMap::new();
        secrets.insert("token".to_string(), "shhh".to_string());
        let snap = engine
            .run(
                &recipe,
                &catalog,
                IndexMap::new(),
                secrets,
                &RunOptions::default(),
            )
            .await
            .unwrap();
        assert_eq!(snap.records.len(), 1);

        let seen = dbg.seen_steps.lock().unwrap();
        assert_eq!(seen.len(), 2, "expected one pause per step, got {seen:?}");

        // First pause: step "first" at index 0, scope has $page but no
        // step bindings yet, secret name listed but value redacted.
        assert_eq!(seen[0].step, "first");
        assert_eq!(seen[0].step_index, 0);
        assert_eq!(seen[0].scope.secrets, vec!["token".to_string()]);
        let json0 = serde_json::to_string(&seen[0].scope).unwrap();
        assert!(
            !json0.contains("shhh"),
            "secret value leaked into first pause: {json0}"
        );
        assert!(
            !json0.contains("FIRST"),
            "first step's response should not be in scope yet: {json0}"
        );

        // Second pause: step "second" at index 1; the first step ran, so
        // `$first` is bound and carries the FIRST marker.
        assert_eq!(seen[1].step, "second");
        assert_eq!(seen[1].step_index, 1);
        let json1 = serde_json::to_string(&seen[1].scope).unwrap();
        assert!(
            json1.contains("FIRST"),
            "second pause should see $first bound: {json1}"
        );
        assert!(
            !json1.contains("shhh"),
            "secret value leaked into second pause: {json1}"
        );
    }

    #[tokio::test]
    async fn debugger_stop_aborts_before_first_request() {
        // Returning `Stop` from the first pause must short-circuit the run
        // before any HTTP fetch goes out — the transport sees zero requests.
        use crate::debug::RecordingDebugger;

        let src = r#"
            recipe "oneStep"
            engine http
            type T { x: String }
            step go {
                method "GET"
                url "https://api.example.com/x"
            }
            emit T { x ← "hi" }
        "#;
        let recipe = parse(src).unwrap();
        let catalog = lonely_catalog(&recipe);
        // An empty ReplayTransport would error on any fetch — if the engine
        // honors Stop, we never call it, so the test passes; if it doesn't,
        // we get NoFixture, not a debugger error, which would fail the assertion.
        let transport = ReplayTransport::new(vec![]);
        let dbg = Arc::new(RecordingDebugger::new(vec![ResumeAction::Stop]));
        let engine = Engine::new(&transport).with_debugger(dbg.clone());

        let err = engine
            .run(
                &recipe,
                &catalog,
                IndexMap::new(),
                IndexMap::new(),
                &RunOptions::default(),
            )
            .await
            .unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("stopped by debugger"),
            "expected 'stopped by debugger', got {msg}"
        );
        assert_eq!(dbg.seen_steps.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn debugger_pauses_inside_for_loop_iterations() {
        // The engine must call before_iteration once per item in a for-loop
        // collection, with the loop variable bound and the iteration index
        // tracked. Bugs in scope frame management here would either skip
        // pauses or expose stale bindings — pin both with a 3-item run.
        use crate::debug::RecordingDebugger;

        let src = r#"
            recipe "iter"
            engine http
            type Item { id: String }
            step list {
                method "GET"
                url "https://api.example.com/items"
            }
            for $i in $list.items[*] {
                emit Item { id ← $i.id }
            }
        "#;
        let recipe = parse(src).unwrap();
        let catalog = lonely_catalog(&recipe);
        let cap = Capture::Http(HttpExchange {
            url: "https://api.example.com/items".into(),
            method: "GET".into(),
            request_headers: IndexMap::new(),
            request_body: None,
            status: 200,
            response_headers: IndexMap::new(),
            body: r#"{"items":[{"id":"a"},{"id":"b"},{"id":"c"}]}"#.into(),
        });
        let transport = ReplayTransport::new(vec![cap]);
        let dbg = Arc::new(RecordingDebugger::new(vec![]));
        let engine = Engine::new(&transport).with_debugger(dbg.clone());

        let snap = engine
            .run(
                &recipe,
                &catalog,
                IndexMap::new(),
                IndexMap::new(),
                &RunOptions::default(),
            )
            .await
            .unwrap();
        assert_eq!(snap.records.len(), 3);

        let iters = dbg.seen_iterations.lock().unwrap();
        assert_eq!(iters.len(), 3, "one iteration pause per item");
        for (idx, p) in iters.iter().enumerate() {
            assert_eq!(p.variable, "i");
            assert_eq!(p.iteration, idx);
            assert_eq!(p.total, 3);
            // The loop variable should be bound to the current item at
            // pause time — assert the JSON contains the item's id.
            let json = serde_json::to_string(&p.scope).unwrap();
            let expected = match idx {
                0 => "\"a\"",
                1 => "\"b\"",
                2 => "\"c\"",
                _ => unreachable!(),
            };
            assert!(
                json.contains(expected),
                "iter {idx} should have $i bound to {expected}: {json}"
            );
        }
    }

    #[tokio::test]
    async fn debugger_stop_in_iteration_aborts_run() {
        // Returning Stop from an iteration pause must abort the run with
        // the same "stopped by debugger" error as a step Stop, even
        // partway through processing a collection.
        use crate::debug::RecordingDebugger;

        let src = r#"
            recipe "iter-stop"
            engine http
            type Item { id: String }
            step list {
                method "GET"
                url "https://api.example.com/items"
            }
            for $i in $list.items[*] {
                emit Item { id ← $i.id }
            }
        "#;
        let recipe = parse(src).unwrap();
        let catalog = lonely_catalog(&recipe);
        let cap = Capture::Http(HttpExchange {
            url: "https://api.example.com/items".into(),
            method: "GET".into(),
            request_headers: IndexMap::new(),
            request_body: None,
            status: 200,
            response_headers: IndexMap::new(),
            body: r#"{"items":[{"id":"a"},{"id":"b"},{"id":"c"}]}"#.into(),
        });
        let transport = ReplayTransport::new(vec![cap]);
        // Script: step pause = Continue, then iter#0 = Stop.
        let dbg = Arc::new(
            RecordingDebugger::new(vec![ResumeAction::Continue, ResumeAction::Stop])
                .with_iterations(),
        );
        let engine = Engine::new(&transport).with_debugger(dbg.clone());

        let err = engine
            .run(
                &recipe,
                &catalog,
                IndexMap::new(),
                IndexMap::new(),
                &RunOptions::default(),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("stopped by debugger"));
        let iters = dbg.seen_iterations.lock().unwrap();
        assert_eq!(iters.len(), 1, "stopped before iter #1 could fire");
    }

    #[tokio::test]
    async fn emit_as_binding_flows_through_typed_ref() {
        // End-to-end: `emit Product { … } as $p`, then a sibling
        // `emit Variant { product ← $p }` should land a snapshot record
        // whose `product` field carries the previously-emitted
        // Product's `_id` inside a `Ref` JSON object.
        let src = r#"
            recipe "refs"
            engine http
            type Product { id: String }
            type Variant {
                product: Ref<Product>
                id:      String
            }
            step list {
                method "GET"
                url "https://api.example.com/items"
            }
            for $p in $list.items[*] {
                emit Product { id ← $p.id } as $prod
                emit Variant { product ← $prod, id ← $p.id }
            }
        "#;
        let recipe = parse(src).unwrap();
        let catalog = lonely_catalog(&recipe);
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
            .run(
                &recipe,
                &catalog,
                IndexMap::new(),
                IndexMap::new(),
                &RunOptions::default(),
            )
            .await
            .unwrap();

        assert_eq!(snap.records.len(), 4);
        // Records interleave: Product, Variant, Product, Variant.
        assert_eq!(snap.records[0].type_name, "Product");
        assert_eq!(snap.records[0].id, "rec-0");
        assert_eq!(snap.records[1].type_name, "Variant");
        assert_eq!(snap.records[1].id, "rec-1");
        assert_eq!(snap.records[2].type_name, "Product");
        assert_eq!(snap.records[2].id, "rec-2");
        assert_eq!(snap.records[3].type_name, "Variant");
        assert_eq!(snap.records[3].id, "rec-3");

        // Variant.product points at the immediately-preceding Product.
        let JSONValue::Object(ref product_ref) = snap.records[1].fields["product"] else {
            panic!(
                "expected Variant.product to be an object Ref; got {:?}",
                snap.records[1].fields["product"],
            );
        };
        assert_eq!(
            product_ref.get("_ref"),
            Some(&JSONValue::String("rec-0".into())),
        );
        assert_eq!(
            product_ref.get("_type"),
            Some(&JSONValue::String("Product".into())),
        );

        // And the second Variant points at the second Product. The
        // for-loop iteration resets the binding, so `$prod` always
        // refers to the Product that was just emitted in the same
        // iteration — not the one from the previous iteration.
        let JSONValue::Object(ref product_ref_2) = snap.records[3].fields["product"] else {
            panic!("expected second Variant.product to be a Ref object");
        };
        assert_eq!(
            product_ref_2.get("_ref"),
            Some(&JSONValue::String("rec-2".into())),
        );
    }

    /// A type declared `share` in a sibling workspace file (and thus
    /// absent from the focal recipe's `types` list) still has to land in
    /// the snapshot's `record_types` with its alignments — JSON-LD
    /// writers and hub indexers read alignment metadata off the
    /// snapshot, not off the recipe source. The fix routes the catalog
    /// (not just `recipe.types`) into `set_record_types`.
    #[tokio::test]
    async fn workspace_shared_type_alignment_lands_in_snapshot() {
        use tempfile::tempdir;
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        std::fs::write(
            root.join("forage.toml"),
            "description = \"\"\ncategory = \"\"\ntags = []\n",
        )
        .unwrap();
        // Sibling decl file: shares a type with an alignment.
        std::fs::write(
            root.join("cannabis.forage"),
            "share type Product aligns schema.org/Product {\n\
             \x20   id: String aligns schema.org/identifier\n\
             }\n",
        )
        .unwrap();
        // Focal recipe: emits the shared Product but doesn't redeclare
        // it locally.
        let recipe_path = root.join("rec.forage");
        std::fs::write(
            &recipe_path,
            r#"recipe "rec"
engine http
step list {
    method "GET"
    url "https://api.example.com/items"
}
for $i in $list.items[*] {
    emit Product { id ← $i.id }
}
"#,
        )
        .unwrap();
        let ws = forage_core::workspace::load(root).unwrap();
        let recipe = forage_core::parse(&std::fs::read_to_string(&recipe_path).unwrap()).unwrap();
        let catalog = ws.catalog(&recipe, |p| std::fs::read_to_string(p)).unwrap();

        let exchange = Capture::Http(HttpExchange {
            url: "https://api.example.com/items".into(),
            method: "GET".into(),
            request_headers: IndexMap::new(),
            request_body: None,
            status: 200,
            response_headers: IndexMap::new(),
            body: r#"{"items":[{"id":"a"}]}"#.into(),
        });
        let transport = ReplayTransport::new(vec![exchange]);
        let engine = Engine::new(&transport);
        let snap = engine
            .run(
                &recipe,
                &catalog,
                IndexMap::new(),
                IndexMap::new(),
                &RunOptions::default(),
            )
            .await
            .unwrap();

        // The recipe emitted one Product record.
        assert_eq!(snap.records.len(), 1);
        assert_eq!(snap.records[0].type_name, "Product");

        // The workspace-shared Product is present in record_types with
        // both type-level and field-level alignments — even though
        // `recipe.types` is empty.
        assert!(recipe.types.is_empty(), "focal recipe declares no types");
        let product = snap
            .record_types
            .get("Product")
            .expect("Product RecordType from workspace catalog");
        assert_eq!(product.alignments.len(), 1);
        assert_eq!(product.alignments[0].ontology, "schema.org");
        assert_eq!(product.alignments[0].term, "Product");
        let id_field = product
            .fields
            .iter()
            .find(|f| f.name == "id")
            .expect("id field");
        let id_alignment = id_field.alignment.as_ref().expect("id alignment");
        assert_eq!(id_alignment.ontology, "schema.org");
        assert_eq!(id_alignment.term, "identifier");
    }

    /// `run_with_prior` binds the upstream records into the recipe's
    /// `[T]` input slot. The downstream recipe then iterates over the
    /// input and re-emits / transforms — no HTTP fetch required for
    /// the records themselves. Pin this so composition stages 2+ can
    /// rely on the input arriving where the validator promised.
    #[tokio::test]
    async fn run_with_prior_binds_records_into_matching_input() {
        let src = r#"
            recipe "downstream"
            engine http
            type Product { id: String }
            input prior: [Product]
            emits Product
            for $p in $input.prior {
                emit Product { id ← $p.id }
            }
        "#;
        let recipe = parse(src).unwrap();
        let catalog = lonely_catalog(&recipe);
        let prior = PriorRecords {
            records: vec![
                Record {
                    id: "rec-0".into(),
                    type_name: "Product".into(),
                    fields: [("id".to_string(), JSONValue::String("upstream-a".into()))]
                        .into_iter()
                        .collect(),
                },
                Record {
                    id: "rec-1".into(),
                    type_name: "Product".into(),
                    fields: [("id".to_string(), JSONValue::String("upstream-b".into()))]
                        .into_iter()
                        .collect(),
                },
            ],
            type_name: "Product".into(),
        };
        let transport = ReplayTransport::new(vec![]);
        let engine = Engine::new(&transport);
        let snap = engine
            .run_with_prior(
                &recipe,
                &catalog,
                IndexMap::new(),
                IndexMap::new(),
                &RunOptions::default(),
                prior,
            )
            .await
            .unwrap();
        let ids: Vec<&str> = snap
            .records
            .iter()
            .map(|r| match r.fields.get("id") {
                Some(JSONValue::String(s)) => s.as_str(),
                _ => "?",
            })
            .collect();
        assert_eq!(ids, vec!["upstream-a", "upstream-b"]);
    }

    #[tokio::test]
    async fn run_with_prior_errors_when_no_matching_input_slot() {
        let src = r#"
            recipe "no-slot"
            engine http
            type Product { id: String }
            emits Product
            emit Product { id ← "x" }
        "#;
        let recipe = parse(src).unwrap();
        let catalog = lonely_catalog(&recipe);
        let prior = PriorRecords {
            records: vec![Record {
                id: "rec-0".into(),
                type_name: "Product".into(),
                fields: IndexMap::new(),
            }],
            type_name: "Product".into(),
        };
        let transport = ReplayTransport::new(vec![]);
        let engine = Engine::new(&transport);
        let err = engine
            .run_with_prior(
                &recipe,
                &catalog,
                IndexMap::new(),
                IndexMap::new(),
                &RunOptions::default(),
                prior,
            )
            .await
            .unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("no input slot"),
            "expected slot-missing diagnostic; got {msg}",
        );
    }

    /// `RunOptions::sample_limit` caps each top-level `for` at the
    /// requested count. The recipe iterates over 100 items; the engine
    /// stops the loop after the fifth, so the snapshot carries exactly
    /// five records.
    #[tokio::test]
    async fn sample_limit_caps_top_level_for_loop() {
        let mut items = String::from("[");
        for i in 0..100 {
            if i > 0 {
                items.push(',');
            }
            items.push_str(&format!(r#"{{"id":"r-{i}"}}"#));
        }
        items.push(']');

        let src = r#"
            recipe "sampled"
            engine http
            type Item { id: String }
            step list {
                method "GET"
                url "https://api.example.com/items"
            }
            for $i in $list[*] {
                emit Item { id ← $i.id }
            }
        "#;
        let recipe = parse(src).unwrap();
        let catalog = lonely_catalog(&recipe);
        let exchange = Capture::Http(HttpExchange {
            url: "https://api.example.com/items".into(),
            method: "GET".into(),
            request_headers: IndexMap::new(),
            request_body: None,
            status: 200,
            response_headers: IndexMap::new(),
            body: items,
        });
        let transport = ReplayTransport::new(vec![exchange]);
        let engine = Engine::new(&transport);
        let options = RunOptions {
            sample_limit: Some(5),
        };
        let snap = engine
            .run(
                &recipe,
                &catalog,
                IndexMap::new(),
                IndexMap::new(),
                &options,
            )
            .await
            .unwrap();
        assert_eq!(snap.records.len(), 5);
        // First five items, in source order — confirms the cap chops
        // the tail rather than reshuffling.
        let ids: Vec<&str> = snap
            .records
            .iter()
            .filter_map(|r| match r.fields.get("id") {
                Some(JSONValue::String(s)) => Some(s.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(ids, vec!["r-0", "r-1", "r-2", "r-3", "r-4"]);
    }

    /// Nested for-loops always run fully — `sample_limit` only caps the
    /// outermost loop. A recipe that emits one record per
    /// `(category, item)` pair with 3 categories × 4 items inside, capped
    /// at sample 2, yields 2 * 4 = 8 records (two outer iterations,
    /// each iterating the full inner array).
    #[tokio::test]
    async fn sample_limit_only_caps_outermost_loop() {
        let src = r#"
            recipe "nested-sample"
            engine http
            type Item { id: String }
            step list {
                method "GET"
                url "https://api.example.com/items"
            }
            for $cat in $list[*] {
                for $i in $cat.items[*] {
                    emit Item { id ← $i.id }
                }
            }
        "#;
        let recipe = parse(src).unwrap();
        let catalog = lonely_catalog(&recipe);
        let body = r#"[
            {"items":[{"id":"a1"},{"id":"a2"},{"id":"a3"},{"id":"a4"}]},
            {"items":[{"id":"b1"},{"id":"b2"},{"id":"b3"},{"id":"b4"}]},
            {"items":[{"id":"c1"},{"id":"c2"},{"id":"c3"},{"id":"c4"}]}
        ]"#;
        let exchange = Capture::Http(HttpExchange {
            url: "https://api.example.com/items".into(),
            method: "GET".into(),
            request_headers: IndexMap::new(),
            request_body: None,
            status: 200,
            response_headers: IndexMap::new(),
            body: body.into(),
        });
        let transport = ReplayTransport::new(vec![exchange]);
        let engine = Engine::new(&transport);
        let options = RunOptions {
            sample_limit: Some(2),
        };
        let snap = engine
            .run(
                &recipe,
                &catalog,
                IndexMap::new(),
                IndexMap::new(),
                &options,
            )
            .await
            .unwrap();
        assert_eq!(snap.records.len(), 8);
        let ids: Vec<&str> = snap
            .records
            .iter()
            .filter_map(|r| match r.fields.get("id") {
                Some(JSONValue::String(s)) => Some(s.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(ids, vec!["a1", "a2", "a3", "a4", "b1", "b2", "b3", "b4"]);
    }

    #[tokio::test]
    async fn debugger_fires_before_each_for_loop_with_total() {
        use crate::debug::RecordingDebugger;

        // `before_for_loop` fires once on loop entry, regardless of
        // whether the collection has zero or many items — exactly the
        // 0-iteration debugger affordance the pause site exists for.
        let src = r#"
            recipe "for-entry"
            engine http
            type Item { id: String }
            step list {
                method "GET"
                url "https://api.example.com/items"
            }
            for $i in $list.items[*] {
                emit Item { id ← $i.id }
            }
        "#;
        let recipe = parse(src).unwrap();
        let catalog = lonely_catalog(&recipe);
        let cap = Capture::Http(HttpExchange {
            url: "https://api.example.com/items".into(),
            method: "GET".into(),
            request_headers: IndexMap::new(),
            request_body: None,
            status: 200,
            response_headers: IndexMap::new(),
            // Two items so the engine enters the for-loop body twice.
            body: r#"{"items":[{"id":"a"},{"id":"b"}]}"#.into(),
        });
        let transport = ReplayTransport::new(vec![cap]);
        let dbg = Arc::new(RecordingDebugger::new(Vec::new()));
        let engine = Engine::new(&transport).with_debugger(dbg.clone());
        engine
            .run(
                &recipe,
                &catalog,
                IndexMap::new(),
                IndexMap::new(),
                &RunOptions::default(),
            )
            .await
            .unwrap();
        let for_loops = dbg.seen_for_loops.lock().unwrap();
        assert_eq!(
            for_loops.len(),
            1,
            "before_for_loop fires once per for-loop"
        );
        assert_eq!(for_loops[0].variable, "i");
        assert_eq!(for_loops[0].total, 2);
        // `start_line` is 0-based; the `for` keyword sits on the 10th
        // source line (0 = empty leading newline, then count the lines
        // up to "            for $i …").
        assert!(
            for_loops[0].start_line > 0,
            "start_line should be non-zero when source is attached"
        );
    }

    #[tokio::test]
    async fn debugger_fires_before_for_loop_on_empty_collection() {
        use crate::debug::RecordingDebugger;

        // The zero-iteration case is the entire reason
        // before_for_loop exists — confirm it fires.
        let src = r#"
            recipe "empty-loop"
            engine http
            type Item { id: String }
            step list {
                method "GET"
                url "https://api.example.com/items"
            }
            for $i in $list.items[*] {
                emit Item { id ← $i.id }
            }
        "#;
        let recipe = parse(src).unwrap();
        let catalog = lonely_catalog(&recipe);
        let cap = Capture::Http(HttpExchange {
            url: "https://api.example.com/items".into(),
            method: "GET".into(),
            request_headers: IndexMap::new(),
            request_body: None,
            status: 200,
            response_headers: IndexMap::new(),
            body: r#"{"items":[]}"#.into(),
        });
        let transport = ReplayTransport::new(vec![cap]);
        let dbg = Arc::new(RecordingDebugger::new(Vec::new()));
        let engine = Engine::new(&transport).with_debugger(dbg.clone());
        engine
            .run(
                &recipe,
                &catalog,
                IndexMap::new(),
                IndexMap::new(),
                &RunOptions::default(),
            )
            .await
            .unwrap();
        let for_loops = dbg.seen_for_loops.lock().unwrap();
        assert_eq!(for_loops.len(), 1);
        assert_eq!(for_loops[0].total, 0, "0-item loop still fires entry");
        // No body iterations ran, so no per-iteration pauses.
        assert!(dbg.seen_iterations.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn debugger_fires_before_each_emit_with_running_index() {
        // The engine must call `before_emit` once per `emit` statement
        // it executes, with `emit_index` monotonically increasing
        // across the run (including emits inside for-loop iterations)
        // and `start_line` resolving to the source line of the `emit`
        // keyword. Pin all three fields so a regression in the index
        // counter, the line resolver, or the dispatch site shows up
        // immediately.
        use crate::debug::RecordingDebugger;

        // The recipe lives at known line offsets so the assertions
        // can compare absolute lines below.
        let src = "recipe \"emits\"\nengine http\ntype Item { id: String }\nstep list {\n    method \"GET\"\n    url \"https://api.example.com/items\"\n}\nfor $i in $list.items[*] {\n    emit Item { id ← $i.id }\n}\n";
        let recipe = parse(src).unwrap();
        let catalog = lonely_catalog(&recipe);
        let cap = Capture::Http(HttpExchange {
            url: "https://api.example.com/items".into(),
            method: "GET".into(),
            request_headers: IndexMap::new(),
            request_body: None,
            status: 200,
            response_headers: IndexMap::new(),
            body: r#"{"items":[{"id":"a"},{"id":"b"}]}"#.into(),
        });
        let transport = ReplayTransport::new(vec![cap]);
        let dbg = Arc::new(RecordingDebugger::new(vec![]));
        let engine = Engine::new(&transport).with_debugger(dbg.clone());

        let snap = engine
            .run(
                &recipe,
                &catalog,
                IndexMap::new(),
                IndexMap::new(),
                &RunOptions::default(),
            )
            .await
            .unwrap();
        assert_eq!(snap.records.len(), 2);

        let emits = dbg.seen_emits.lock().unwrap();
        assert_eq!(emits.len(), 2, "one emit pause per record, got {emits:?}");
        for (idx, p) in emits.iter().enumerate() {
            assert_eq!(p.type_name, "Item");
            assert_eq!(p.emit_index, idx);
            // The `emit` keyword sits on line 8 (0-based) in `src`.
            assert_eq!(
                p.start_line, 8,
                "emit pause #{idx} should resolve to line 8, got {:?}",
                p.start_line
            );
        }
    }

    #[tokio::test]
    async fn debugger_stop_in_emit_aborts_run() {
        // Returning Stop from an emit pause must abort the run with the
        // same "stopped by debugger" error path as the step / iteration
        // Stop arms, leaving any later records uncommitted.
        use crate::debug::RecordingDebugger;

        let src = "recipe \"emit-stop\"\nengine http\ntype Item { id: String }\nstep list {\n    method \"GET\"\n    url \"https://api.example.com/items\"\n}\nfor $i in $list.items[*] {\n    emit Item { id ← $i.id }\n}\n";
        let recipe = parse(src).unwrap();
        let catalog = lonely_catalog(&recipe);
        let cap = Capture::Http(HttpExchange {
            url: "https://api.example.com/items".into(),
            method: "GET".into(),
            request_headers: IndexMap::new(),
            request_body: None,
            status: 200,
            response_headers: IndexMap::new(),
            body: r#"{"items":[{"id":"a"},{"id":"b"},{"id":"c"}]}"#.into(),
        });
        let transport = ReplayTransport::new(vec![cap]);
        // Script: step = Continue, then emit#0 = Stop. Iteration pauses
        // short-circuit without consuming because we don't opt into
        // `with_iterations()`.
        let dbg = Arc::new(
            RecordingDebugger::new(vec![ResumeAction::Continue, ResumeAction::Stop]).with_emits(),
        );
        let engine = Engine::new(&transport).with_debugger(dbg.clone());

        let err = engine
            .run(
                &recipe,
                &catalog,
                IndexMap::new(),
                IndexMap::new(),
                &RunOptions::default(),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("stopped by debugger"));
        let emits = dbg.seen_emits.lock().unwrap();
        assert_eq!(emits.len(), 1, "stopped before emit #1 could fire");
        // Snapshot must be empty — Stop fires before run_emit commits.
        let steps = dbg.seen_steps.lock().unwrap();
        assert_eq!(steps.len(), 1, "step pause should have fired exactly once");
    }

    #[tokio::test]
    async fn debugger_stop_in_for_loop_aborts_run() {
        // Returning Stop from a for-loop entry must abort the run with
        // the same "stopped by debugger" error as the step / emit /
        // iteration sites; no iteration body fires after the stop.
        use crate::debug::RecordingDebugger;

        let src = r#"
            recipe "fstop"
            engine http
            type Item { id: String }
            step list {
                method "GET"
                url "https://api.example.com/items"
            }
            for $i in $list.items[*] {
                emit Item { id ← $i.id }
            }
        "#;
        let recipe = parse(src).unwrap();
        let catalog = lonely_catalog(&recipe);
        let cap = Capture::Http(HttpExchange {
            url: "https://api.example.com/items".into(),
            method: "GET".into(),
            request_headers: IndexMap::new(),
            request_body: None,
            status: 200,
            response_headers: IndexMap::new(),
            body: r#"{"items":[{"id":"a"},{"id":"b"}]}"#.into(),
        });
        let transport = ReplayTransport::new(vec![cap]);
        // Script: step pause = Continue, then for-loop entry = Stop.
        let dbg = Arc::new(
            RecordingDebugger::new(vec![ResumeAction::Continue, ResumeAction::Stop])
                .with_for_loops(),
        );
        let engine = Engine::new(&transport).with_debugger(dbg.clone());

        let err = engine
            .run(
                &recipe,
                &catalog,
                IndexMap::new(),
                IndexMap::new(),
                &RunOptions::default(),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("stopped by debugger"));
        let iters = dbg.seen_iterations.lock().unwrap();
        assert!(iters.is_empty(), "stopped before any iteration could fire");
    }

    #[tokio::test]
    async fn step_in_and_step_over_engine_equivalent_at_for_loop() {
        // `ResumeAction::StepIn` and `ResumeAction::StepOver` are wire-
        // distinct so the host can carry user intent across the
        // boundary — but the engine treats both identically today: each
        // pause site's resume action falls through, and the body's
        // first pause site (an emit here) re-pauses regardless of which
        // variant the host sent. Future engine work (body-suppression
        // counter keyed off StepOver) can break the symmetry without a
        // wire-shape change; this test pins the equivalence we ship
        // with so a regression that accidentally diverges the two
        // variants fails loudly.
        use crate::debug::RecordingDebugger;

        let src = "recipe \"pair\"\nengine http\ntype Item { id: String }\nstep list {\n    method \"GET\"\n    url \"https://api.example.com/items\"\n}\nfor $i in $list.items[*] {\n    emit Item { id ← $i.id }\n}\n";
        let recipe = parse(src).unwrap();
        let catalog = lonely_catalog(&recipe);

        // Per-run helper: same recipe, same capture, same script
        // shape; only the at-for-loop action differs. Returns the
        // emit-pause count at termination so the caller can compare.
        async fn emits_after(
            action: ResumeAction,
            recipe: &ForageFile,
            catalog: &TypeCatalog,
        ) -> usize {
            let cap = Capture::Http(HttpExchange {
                url: "https://api.example.com/items".into(),
                method: "GET".into(),
                request_headers: IndexMap::new(),
                request_body: None,
                status: 200,
                response_headers: IndexMap::new(),
                body: r#"{"items":[{"id":"a"},{"id":"b"},{"id":"c"}]}"#.into(),
            });
            let transport = ReplayTransport::new(vec![cap]);
            // Script: step pause = Continue, for-loop pause = action,
            // first emit pause = Stop. If `action` falls through and
            // the engine descends into the body, the emit pause fires
            // once before Stop short-circuits the run.
            let dbg = Arc::new(
                RecordingDebugger::new(vec![ResumeAction::Continue, action, ResumeAction::Stop])
                    .with_for_loops()
                    .with_emits(),
            );
            let engine = Engine::new(&transport).with_debugger(dbg.clone());
            let _ = engine
                .run(
                    recipe,
                    catalog,
                    IndexMap::new(),
                    IndexMap::new(),
                    &RunOptions::default(),
                )
                .await;
            dbg.seen_emits.lock().unwrap().len()
        }

        let in_emits = emits_after(ResumeAction::StepIn, &recipe, &catalog).await;
        let over_emits = emits_after(ResumeAction::StepOver, &recipe, &catalog).await;
        assert_eq!(in_emits, 1, "StepIn falls through into the body");
        assert_eq!(
            in_emits, over_emits,
            "StepIn and StepOver currently produce identical engine behavior at a for-loop pause",
        );
    }

    #[tokio::test]
    async fn step_in_at_step_falls_back_to_step_over() {
        // At a step pause site there's no body to descend into, so
        // StepIn must behave identically to StepOver: the engine
        // resumes and the next pause site fires. With two steps in
        // the recipe, StepIn at step 1 lets execution reach step 2.
        use crate::debug::RecordingDebugger;

        let src = r#"
            recipe "two"
            engine http
            type Item { id: String }
            step first {
                method "GET"
                url "https://api.example.com/a"
            }
            step second {
                method "GET"
                url "https://api.example.com/b"
            }
            emit Item { id ← "x" }
        "#;
        let recipe = parse(src).unwrap();
        let catalog = lonely_catalog(&recipe);
        let first = Capture::Http(HttpExchange {
            url: "https://api.example.com/a".into(),
            method: "GET".into(),
            request_headers: IndexMap::new(),
            request_body: None,
            status: 200,
            response_headers: IndexMap::new(),
            body: r#"{"ok":true}"#.into(),
        });
        let second = Capture::Http(HttpExchange {
            url: "https://api.example.com/b".into(),
            method: "GET".into(),
            request_headers: IndexMap::new(),
            request_body: None,
            status: 200,
            response_headers: IndexMap::new(),
            body: r#"{"ok":true}"#.into(),
        });
        let transport = ReplayTransport::new(vec![first, second]);
        // Script: step 1 = StepIn (falls through), step 2 = Stop.
        let dbg = Arc::new(RecordingDebugger::new(vec![
            ResumeAction::StepIn,
            ResumeAction::Stop,
        ]));
        let engine = Engine::new(&transport).with_debugger(dbg.clone());

        let err = engine
            .run(
                &recipe,
                &catalog,
                IndexMap::new(),
                IndexMap::new(),
                &RunOptions::default(),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("stopped by debugger"));
        let steps = dbg.seen_steps.lock().unwrap();
        assert_eq!(steps.len(), 2, "StepIn at step 1 reached step 2");
        assert_eq!(steps[0].step, "first");
        assert_eq!(steps[1].step, "second");
    }

    #[tokio::test]
    async fn step_response_captures_status_headers_body_and_format() {
        // The debugger's StepResponse map must populate after each
        // executed step with the resolved status, header map (verbatim
        // from the transport), raw body, content-type-detected format
        // (since `parse :` is absent here), and the normalized
        // content-type header value.
        use crate::debug::RecordingDebugger;

        let src = r#"
            recipe "capture"
            engine http
            type Item { id: String }
            step list {
                method "GET"
                url    "https://api.example.com/items"
            }
            for $i in $list.items[*] {
                emit Item { id ← $i.id }
            }
        "#;
        let recipe = parse(src).unwrap();
        let catalog = lonely_catalog(&recipe);
        let mut hdrs = IndexMap::new();
        hdrs.insert(
            "Content-Type".into(),
            "application/json; charset=utf-8".into(),
        );
        hdrs.insert("X-Trace-Id".into(), "abc-123".into());
        let cap = Capture::Http(HttpExchange {
            url: "https://api.example.com/items".into(),
            method: "GET".into(),
            request_headers: IndexMap::new(),
            request_body: None,
            status: 200,
            response_headers: hdrs,
            body: r#"{"items":[{"id":"a"}]}"#.into(),
        });
        let transport = ReplayTransport::new(vec![cap]);
        // Use the for-loop pause site as a deterministic inspection
        // point: it fires after `list` ran, so step_responses must
        // already contain the captured response.
        let dbg = Arc::new(RecordingDebugger::new(vec![]).with_for_loops());
        let engine = Engine::new(&transport).with_debugger(dbg.clone());
        let _ = engine
            .run(
                &recipe,
                &catalog,
                IndexMap::new(),
                IndexMap::new(),
                &RunOptions::default(),
            )
            .await
            .unwrap();

        let loops = dbg.seen_for_loops.lock().unwrap();
        let resp = loops[0]
            .scope
            .step_responses
            .get("list")
            .expect("step_responses should carry the executed step");
        assert_eq!(resp.status, 200);
        assert_eq!(resp.format, ParseFormat::Json);
        assert_eq!(
            resp.content_type_header.as_deref(),
            Some("application/json"),
        );
        assert_eq!(resp.body_raw, r#"{"items":[{"id":"a"}]}"#);
        assert!(!resp.body_truncated);
        assert_eq!(resp.headers.get("X-Trace-Id"), Some(&"abc-123".to_string()),);
    }

    #[tokio::test]
    async fn step_response_truncates_oversized_body() {
        // Pin the 1 MiB cap: any body larger than BODY_CAPTURE_MAX gets
        // sliced down with `body_truncated = true`, so the debug viewer
        // doesn't OOM at pause time and the user knows the body isn't
        // complete. The second step acts as a post-`big` inspection
        // point — its `before_step` pause snapshots scope after `big`
        // has executed and captured, but before anything else fires.
        use crate::debug::RecordingDebugger;

        let src = r#"
            recipe "truncate"
            engine http
            type Item { id: String }
            step big {
                method "GET"
                url    "https://api.example.com/big"
                parse  : text
            }
            step probe {
                method "GET"
                url    "https://api.example.com/probe"
            }
        "#;
        let recipe = parse(src).unwrap();
        let catalog = lonely_catalog(&recipe);
        let oversized = "x".repeat(BODY_CAPTURE_MAX + 1024);
        let captures = vec![
            Capture::Http(HttpExchange {
                url: "https://api.example.com/big".into(),
                method: "GET".into(),
                request_headers: IndexMap::new(),
                request_body: None,
                status: 200,
                response_headers: IndexMap::new(),
                body: oversized,
            }),
            Capture::Http(HttpExchange {
                url: "https://api.example.com/probe".into(),
                method: "GET".into(),
                request_headers: IndexMap::new(),
                request_body: None,
                status: 200,
                response_headers: IndexMap::new(),
                body: "{}".into(),
            }),
        ];
        let transport = ReplayTransport::new(captures);
        let dbg = Arc::new(RecordingDebugger::new(vec![]));
        let engine = Engine::new(&transport).with_debugger(dbg.clone());
        let _ = engine
            .run(
                &recipe,
                &catalog,
                IndexMap::new(),
                IndexMap::new(),
                &RunOptions::default(),
            )
            .await
            .unwrap();

        let steps = dbg.seen_steps.lock().unwrap();
        assert_eq!(steps.len(), 2, "two step pauses expected, got {steps:?}");
        // probe's `before_step` snapshot is taken after `big` ran, so
        // big's capture must be present and truncated.
        let resp = steps[1]
            .scope
            .step_responses
            .get("big")
            .expect("captured response for step `big`");
        assert!(
            resp.body_truncated,
            "oversized body should set body_truncated"
        );
        assert_eq!(
            resp.body_raw.len(),
            BODY_CAPTURE_MAX,
            "body_raw should be sliced to the cap",
        );
    }

    /// Sink that records every `step_response_captured` call so tests
    /// can assert the engine fires once per step, with the resolved
    /// StepResponse, on both success and failure paths. Also records
    /// `step_response_full_body` hits so the disk-stash precondition
    /// (uncapped bytes are visible to the host) stays pinned.
    struct RecordingSink {
        captured: std::sync::Mutex<Vec<(String, StepResponse)>>,
        full_bodies: std::sync::Mutex<Vec<(String, Vec<u8>)>>,
    }

    impl RecordingSink {
        fn new() -> Self {
            Self {
                captured: std::sync::Mutex::new(Vec::new()),
                full_bodies: std::sync::Mutex::new(Vec::new()),
            }
        }
    }

    impl ProgressSink for RecordingSink {
        fn emit(&self, _: RunEvent) {}
        fn step_response_captured(&self, step: &str, response: &StepResponse) {
            self.captured
                .lock()
                .expect("captured step responses")
                .push((step.to_string(), response.clone()));
        }
        fn step_response_full_body(&self, step: &str, body: &[u8]) {
            self.full_bodies
                .lock()
                .expect("captured full bodies")
                .push((step.to_string(), body.to_vec()));
        }
    }

    #[tokio::test]
    async fn progress_sink_receives_step_response_on_success() {
        // The sink hook fires once per executed step on a clean
        // 2xx path — the studio relies on this to populate its
        // Responses pane during a normal run.
        let src = r#"
            recipe "ok"
            engine http
            type Item { id: String }
            step list {
                method "GET"
                url    "https://api.example.com/items"
            }
            for $i in $list.items[*] {
                emit Item { id ← $i.id }
            }
        "#;
        let recipe = parse(src).unwrap();
        let catalog = lonely_catalog(&recipe);
        let mut hdrs = IndexMap::new();
        hdrs.insert("Content-Type".into(), "application/json".into());
        let cap = Capture::Http(HttpExchange {
            url: "https://api.example.com/items".into(),
            method: "GET".into(),
            request_headers: IndexMap::new(),
            request_body: None,
            status: 200,
            response_headers: hdrs,
            body: r#"{"items":[{"id":"a"}]}"#.into(),
        });
        let transport = ReplayTransport::new(vec![cap]);
        let sink = Arc::new(RecordingSink::new());
        let engine = Engine::new(&transport).with_progress(sink.clone());
        let _ = engine
            .run(
                &recipe,
                &catalog,
                IndexMap::new(),
                IndexMap::new(),
                &RunOptions::default(),
            )
            .await
            .unwrap();

        let seen = sink.captured.lock().unwrap();
        assert_eq!(seen.len(), 1, "expected one capture, got {seen:?}");
        assert_eq!(seen[0].0, "list");
        assert_eq!(seen[0].1.status, 200);
        assert_eq!(seen[0].1.format, ParseFormat::Json);
        assert_eq!(seen[0].1.body_raw, r#"{"items":[{"id":"a"}]}"#);
    }

    #[tokio::test]
    async fn progress_sink_receives_step_response_on_5xx_failure() {
        // The whole point of streaming captures independent of pause
        // state is that the user can still inspect a 5xx response
        // even when the engine aborts the run on the status gate.
        // Pin that the sink fires before the gate trips.
        let src = r#"
            recipe "bad"
            engine http
            type Item { id: String }
            step list {
                method "GET"
                url    "https://api.example.com/items"
            }
            for $i in $list.items[*] {
                emit Item { id ← $i.id }
            }
        "#;
        let recipe = parse(src).unwrap();
        let catalog = lonely_catalog(&recipe);
        let mut hdrs = IndexMap::new();
        hdrs.insert("Content-Type".into(), "application/json".into());
        let cap = Capture::Http(HttpExchange {
            url: "https://api.example.com/items".into(),
            method: "GET".into(),
            request_headers: IndexMap::new(),
            request_body: None,
            status: 500,
            response_headers: hdrs,
            body: r#"{"error":"oops"}"#.into(),
        });
        let transport = ReplayTransport::new(vec![cap]);
        let sink = Arc::new(RecordingSink::new());
        let engine = Engine::new(&transport).with_progress(sink.clone());
        let err = engine
            .run(
                &recipe,
                &catalog,
                IndexMap::new(),
                IndexMap::new(),
                &RunOptions::default(),
            )
            .await
            .expect_err("5xx should abort the run");
        assert!(matches!(err, HttpError::Status { status: 500, .. }));

        let seen = sink.captured.lock().unwrap();
        assert_eq!(
            seen.len(),
            1,
            "the failing step's capture should still reach the sink",
        );
        assert_eq!(seen[0].0, "list");
        assert_eq!(seen[0].1.status, 500);
        assert_eq!(seen[0].1.body_raw, r#"{"error":"oops"}"#);
    }

    #[tokio::test]
    async fn progress_sink_receives_full_body_uncapped() {
        // The `step_response_full_body` hook is the disk-stash
        // precondition: hosts get the raw bytes before the cap is
        // applied, so a multi-MB response stays inspectable via the
        // "Load full" affordance even though `body_raw` was sliced.
        // Pin the invariant by running a single step whose response
        // body crosses the cap and asserting the hook saw every byte.
        let src = r#"
            recipe "full"
            engine http
            type Item { id: String }
            step big {
                method "GET"
                url    "https://api.example.com/big"
                parse  : text
            }
        "#;
        let recipe = parse(src).unwrap();
        let catalog = lonely_catalog(&recipe);
        let oversized = "x".repeat(BODY_CAPTURE_MAX + 1024);
        let oversized_len = oversized.len();
        let cap = Capture::Http(HttpExchange {
            url: "https://api.example.com/big".into(),
            method: "GET".into(),
            request_headers: IndexMap::new(),
            request_body: None,
            status: 200,
            response_headers: IndexMap::new(),
            body: oversized.clone(),
        });
        let transport = ReplayTransport::new(vec![cap]);
        let sink = Arc::new(RecordingSink::new());
        let engine = Engine::new(&transport).with_progress(sink.clone());
        let _ = engine
            .run(
                &recipe,
                &catalog,
                IndexMap::new(),
                IndexMap::new(),
                &RunOptions::default(),
            )
            .await
            .unwrap();

        let full = sink.full_bodies.lock().unwrap();
        assert_eq!(full.len(), 1, "one full-body hit per step");
        assert_eq!(full[0].0, "big");
        assert_eq!(
            full[0].1.len(),
            oversized_len,
            "the host must see the uncapped bytes — body_raw is truncated, the stash isn't",
        );
        assert!(full[0].1.iter().all(|&b| b == b'x'));

        // Sanity: the in-payload `body_raw` did get truncated, so the
        // disk stash is the *only* path to the full body.
        let captured = sink.captured.lock().unwrap();
        assert!(captured[0].1.body_truncated);
        assert_eq!(captured[0].1.body_raw.len(), BODY_CAPTURE_MAX);
    }

    #[tokio::test]
    async fn parse_override_forces_json_on_text_plain_response() {
        // The headline use case: a server claims `text/plain` for
        // what is actually JSON. Without `parse :`, the engine's
        // content-type detection would treat the body as raw text
        // and downstream `$step.field` wouldn't resolve. With
        // `parse : json`, the engine binds the parsed object.
        let src = r#"
            recipe "json-override"
            engine http
            type Item { id: String }
            step list {
                method "GET"
                url "https://api.example.com/items"
                parse : json
            }
            for $i in $list.items[*] {
                emit Item { id ← $i.id }
            }
        "#;
        let recipe = parse(src).unwrap();
        let catalog = lonely_catalog(&recipe);
        let mut headers = IndexMap::new();
        headers.insert("content-type".to_string(), "text/plain".to_string());
        let cap = Capture::Http(HttpExchange {
            url: "https://api.example.com/items".into(),
            method: "GET".into(),
            request_headers: IndexMap::new(),
            request_body: None,
            status: 200,
            response_headers: headers,
            body: r#"{"items":[{"id":"a"},{"id":"b"}]}"#.into(),
        });
        let transport = ReplayTransport::new(vec![cap]);
        let engine = Engine::new(&transport);
        let snap = engine
            .run(
                &recipe,
                &catalog,
                IndexMap::new(),
                IndexMap::new(),
                &RunOptions::default(),
            )
            .await
            .unwrap();
        assert_eq!(snap.records.len(), 2);
    }
}
