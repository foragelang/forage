//! Forage expression / template / transform evaluator.
//!
//! Public surface: `Evaluator::new(registry)` then either
//! `evaluator.eval_extraction(expr, scope)` (sync, for pure transforms)
//! or `evaluator.eval_extraction_async(expr, scope, &transport)` (async,
//! routes transport-aware transforms through a [`TransportContext`]).

pub mod error;
pub mod html;
pub mod scope;
pub mod transforms;
pub mod transforms_async;
pub mod value;

pub use error::EvalError;
pub use scope::Scope;
pub use transforms::{
    AsyncTransformFn, TransformFn, TransformFuture, TransformRegistry, default_registry,
};
pub use value::{EvalValue, RegexValue};

use std::future::Future;
use std::pin::Pin;

use indexmap::IndexMap;

use crate::ast::*;

/// Bridge from a transport-aware transform back to the host engine's
/// HTTP transport. Defined in `forage-core` (no I/O dependencies) so
/// the language core can declare async transforms without pulling in
/// `forage-http`; concrete engines implement it by wrapping their
/// `Transport`. Routing every transform fetch through the same
/// transport is what lets `--replay <fixtures>` capture wikidata
/// reconciliation alongside step-level requests.
#[async_trait::async_trait]
pub trait TransportContext: Send + Sync {
    /// Issue a GET against `url`, parse the response body as JSON, and
    /// hand back the parsed value. Transport-aware transforms call this
    /// for their network needs; the engine's implementation also threads
    /// progress events and request/response logging.
    async fn fetch_json(&self, url: &str) -> Result<EvalValue, EvalError>;
}

/// Sentinel transport context the sync evaluator paths bind to. Every
/// transport-aware transform routed through this context fails with
/// [`EvalError::TransformRequiresTransport`]; that's the diagnostic
/// surfaced to callers that try to fire `wikidataEntity` from a sync
/// path. The async engine path replaces it with a live transport.
pub struct NoTransport;

#[async_trait::async_trait]
impl TransportContext for NoTransport {
    async fn fetch_json(&self, _url: &str) -> Result<EvalValue, EvalError> {
        Err(EvalError::TransformRequiresTransport {
            name: "<unknown>".into(),
        })
    }
}

/// Knobs the for-loop walker reads off the engine on every run.
///
/// The wider runtime "what flags did the user pass for this invocation"
/// shape (replay / ephemeral on top of sample_limit) lives at the
/// daemon layer — the engine's only sampling concern is how to cap a
/// top-level `for` over a captured array. Replay is realized at the
/// transport layer; ephemeral is a daemon output-store choice. Keeping
/// the engine's options narrow means a downstream consumer can sample
/// without pulling in transport-shaped state.
#[derive(Debug, Clone, Default)]
pub struct RunOptions {
    /// Cap each top-level `for $x in $arr[*]` iteration at this many
    /// items. `None` = no cap. Nested for-loops always run fully — the
    /// outermost loop is the recipe's record-producing unit; capping
    /// nested loops would chop the fields of an individual record
    /// (e.g. all of a product's variants), which is rarely what the
    /// playground wants.
    pub sample_limit: Option<u32>,
}

impl RunOptions {
    /// Apply the sample cap to a top-level for-loop's items vector,
    /// trimming it down to `sample_limit` when set. A `None` cap or a
    /// cap larger than the vector is a no-op.
    pub fn cap_top_level(&self, items: &mut Vec<EvalValue>) {
        if let Some(limit) = self.sample_limit
            && (items.len() as u64) > limit as u64
        {
            items.truncate(limit as usize);
        }
    }
}

pub struct Evaluator<'r> {
    pub registry: &'r TransformRegistry,
}

impl<'r> Evaluator<'r> {
    pub fn new(registry: &'r TransformRegistry) -> Self {
        Self { registry }
    }

    pub fn eval_extraction(
        &self,
        expr: &ExtractionExpr,
        scope: &Scope,
    ) -> Result<EvalValue, EvalError> {
        match expr {
            ExtractionExpr::Path(p) => self.eval_path(p, scope),
            ExtractionExpr::Literal(j) => Ok(EvalValue::from(j.clone())),
            ExtractionExpr::Template(t) => self.render_template(t, scope).map(EvalValue::String),
            ExtractionExpr::Pipe(head, calls) => {
                let mut v = self.eval_extraction(head, scope)?;
                for call in calls {
                    v = self.apply_pipe_call(&call.name, v, &call.args, scope)?;
                }
                Ok(v)
            }
            ExtractionExpr::Call { name, args } => self.apply_direct_call(name, args, scope),
            ExtractionExpr::CaseOf {
                scrutinee,
                branches,
            } => {
                let v = self.eval_path(scrutinee, scope)?;
                let label = match &v {
                    EvalValue::Bool(b) => b.to_string(),
                    EvalValue::String(s) => s.clone(),
                    EvalValue::Int(n) => n.to_string(),
                    EvalValue::Null => "null".into(),
                    other => format!("{other:?}"),
                };
                let mut default_arm: Option<&ExtractionExpr> = None;
                for (l, arm) in branches {
                    if l == "_" {
                        default_arm = Some(arm);
                        continue;
                    }
                    if l == &label {
                        return self.eval_extraction(arm, scope);
                    }
                }
                if let Some(arm) = default_arm {
                    return self.eval_extraction(arm, scope);
                }
                Err(EvalError::CaseNoMatch { label })
            }
            ExtractionExpr::MapTo { path, .. } => {
                // Emission inside MapTo is handled by the snapshot/engine layer;
                // here we just resolve the path and return.
                self.eval_path(path, scope)
            }
            ExtractionExpr::BinaryOp { op, lhs, rhs } => {
                let l = self.eval_extraction(lhs, scope)?;
                let r = self.eval_extraction(rhs, scope)?;
                apply_binary(*op, l, r)
            }
            ExtractionExpr::Unary { op, operand } => {
                let v = self.eval_extraction(operand, scope)?;
                apply_unary(*op, v)
            }
            ExtractionExpr::StructLiteral { fields } => {
                let mut out: IndexMap<String, EvalValue> = IndexMap::new();
                for f in fields {
                    if out.contains_key(&f.field_name) {
                        return Err(EvalError::DuplicateStructField(f.field_name.clone()));
                    }
                    let v = self.eval_extraction(&f.expr, scope)?;
                    out.insert(f.field_name.clone(), v);
                }
                Ok(EvalValue::Object(out))
            }
            ExtractionExpr::Index { base, index } => {
                let b = self.eval_extraction(base, scope)?;
                let i = self.eval_extraction(index, scope)?;
                apply_index(b, i)
            }
            ExtractionExpr::RegexLiteral(lit) => {
                let re = crate::parse::parser::build_regex(&lit.pattern, &lit.flags).map_err(
                    |e| EvalError::Generic(format!("regex /{}/{}: {e}", lit.pattern, lit.flags)),
                )?;
                Ok(EvalValue::Regex(crate::eval::value::RegexValue {
                    pattern: lit.pattern.clone(),
                    flags: lit.flags.clone(),
                    re,
                }))
            }
        }
    }

    pub fn eval_path(&self, p: &PathExpr, scope: &Scope) -> Result<EvalValue, EvalError> {
        match p {
            PathExpr::Current => Ok(scope.current.clone().unwrap_or(EvalValue::Null)),
            PathExpr::Input => Ok(EvalValue::Object(
                scope
                    .inputs()
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
            )),
            PathExpr::Secret(name) => scope
                .secret(name)
                .map(|s| EvalValue::String(s.into()))
                .ok_or(EvalError::UndefinedSecret(name.clone())),
            PathExpr::Variable(name) => scope
                .lookup(name)
                .cloned()
                .ok_or(EvalError::UndefinedVariable(name.clone())),
            PathExpr::Field(base, field) => {
                // Special case: `$input.<x>` — look up the input directly.
                if let PathExpr::Input = base.as_ref() {
                    return scope
                        .input(field)
                        .cloned()
                        .ok_or(EvalError::UndefinedInput(field.clone()));
                }
                let v = self.eval_path(base, scope)?;
                Ok(field_of(&v, field))
            }
            PathExpr::OptField(base, field) => {
                if let PathExpr::Input = base.as_ref() {
                    return Ok(scope.input(field).cloned().unwrap_or(EvalValue::Null));
                }
                let v = self.eval_path(base, scope)?;
                if matches!(v, EvalValue::Null) {
                    return Ok(EvalValue::Null);
                }
                Ok(field_of(&v, field))
            }
            PathExpr::Index(base, idx) => {
                // Out-of-bounds indexing returns Null rather than erroring.
                // Scraping recipes routinely access fields like
                // `$product.potency?.range[0]` where the array may be empty
                // for some records — that's the same shape of "missing" the
                // `?.` operator handles for objects. Authors who want a hard
                // assertion can use an `expect { … }` block.
                let v = self.eval_path(base, scope)?;
                match v {
                    EvalValue::Array(mut xs) => {
                        let len = xs.len();
                        let i = resolve_index(*idx, len);
                        match i {
                            Some(i) => Ok(xs.swap_remove(i)),
                            None => Ok(EvalValue::Null),
                        }
                    }
                    EvalValue::NodeList(mut xs) => {
                        let len = xs.len();
                        let i = resolve_index(*idx, len);
                        match i {
                            Some(i) => Ok(EvalValue::Node(xs.swap_remove(i))),
                            None => Ok(EvalValue::Null),
                        }
                    }
                    EvalValue::Null => Ok(EvalValue::Null),
                    other => Err(EvalError::TypeMismatch {
                        expected: "array",
                        actual: kind_of(&other),
                    }),
                }
            }
            PathExpr::Wildcard(base) => {
                let v = self.eval_path(base, scope)?;
                match v {
                    EvalValue::Array(xs) => Ok(EvalValue::Array(xs)),
                    EvalValue::NodeList(xs) => Ok(EvalValue::NodeList(xs)),
                    EvalValue::Null => Ok(EvalValue::Array(vec![])),
                    other => Err(EvalError::WildcardOnNonArray {
                        kind: kind_of(&other),
                    }),
                }
            }
        }
    }

    pub fn render_template(&self, t: &Template, scope: &Scope) -> Result<String, EvalError> {
        let mut out = String::new();
        for part in &t.parts {
            match part {
                TemplatePart::Literal(s) => out.push_str(s),
                TemplatePart::Interp(e) => {
                    let v = self.eval_extraction(e, scope)?;
                    out.push_str(&stringify(&v));
                }
            }
        }
        Ok(out)
    }

    /// Pipe-style application: `<head> |> name(args...)`. The pipe head
    /// is always passed in as the leading value; for user fns it
    /// becomes param 0, and `args` fill params 1..N.
    ///
    /// A pipe call always supplies a head, so the arity it presents to
    /// a user fn is `1 + args.len()` — never zero. A zero-parameter user
    /// fn is therefore uncallable via pipe; the validator flags it as
    /// `WrongArity` and the runtime check below catches anything that
    /// slips through.
    fn apply_pipe_call(
        &self,
        name: &str,
        head: EvalValue,
        args: &[ExtractionExpr],
        scope: &Scope,
    ) -> Result<EvalValue, EvalError> {
        let mut resolved = Vec::with_capacity(args.len());
        for a in args {
            resolved.push(self.eval_extraction(a, scope)?);
        }
        if let Some(decl) = self.registry.get_user_fn(name) {
            let provided = 1 + resolved.len();
            return self.apply_user_fn(decl, Some(head), &resolved, provided, scope);
        }
        if self.registry.get_async(name).is_some() {
            return Err(EvalError::TransformRequiresTransport { name: name.into() });
        }
        let f = self
            .registry
            .get(name)
            .ok_or_else(|| EvalError::UnknownTransform { name: name.into() })?;
        f(head, &resolved)
    }

    /// Direct-call application: `name(args...)`. For user fns every
    /// argument is explicit — args[0] is param 0, args[1..] fill the
    /// rest, and zero args is the valid zero-param call. For built-in
    /// transforms we preserve the historical convention of passing
    /// `scope.current` as the head value because the cannabis-domain
    /// transforms rely on it.
    fn apply_direct_call(
        &self,
        name: &str,
        args: &[ExtractionExpr],
        scope: &Scope,
    ) -> Result<EvalValue, EvalError> {
        let mut resolved = Vec::with_capacity(args.len());
        for a in args {
            resolved.push(self.eval_extraction(a, scope)?);
        }
        if let Some(decl) = self.registry.get_user_fn(name) {
            let provided = resolved.len();
            return self.apply_user_fn(decl, None, &resolved, provided, scope);
        }
        if self.registry.get_async(name).is_some() {
            return Err(EvalError::TransformRequiresTransport { name: name.into() });
        }
        let f = self
            .registry
            .get(name)
            .ok_or_else(|| EvalError::UnknownTransform { name: name.into() })?;
        let head = scope.current.clone().unwrap_or(EvalValue::Null);
        f(head, &resolved)
    }

    /// Evaluate a user-fn body with parameters bound. `head` carries the
    /// pipe head when called via `apply_pipe_call`; direct calls pass
    /// `None` so the explicit `args` fill every parameter. `provided`
    /// is the call site's count (head + args for pipe, args for direct)
    /// — set by the caller so each dispatch path keeps its own arity
    /// convention. The body sees only the parameters plus the
    /// recipe-level `$secret.*` / `$input.*` paths — for-loop variables
    /// and `as $v` bindings at the call site are deliberately invisible.
    fn apply_user_fn(
        &self,
        decl: &crate::ast::FnDecl,
        head: Option<EvalValue>,
        args: &[EvalValue],
        provided: usize,
        scope: &Scope,
    ) -> Result<EvalValue, EvalError> {
        let expected = decl.params.len();
        if provided != expected {
            return Err(EvalError::FnArityMismatch {
                name: decl.name.clone(),
                expected,
                got: provided,
            });
        }
        // Build a closed scope from the parent's inputs + secrets only.
        // The parent's frames (for-loop vars, `as $v` bindings) are
        // deliberately excluded so functions are closed units.
        let mut child = Scope::new()
            .with_inputs(scope.inputs().clone())
            .with_secrets(scope.secrets_map().clone());
        let mut params = decl.params.iter();
        if let Some(h) = head
            && let Some(first) = params.next()
        {
            child.bind(first, h);
        }
        for (p, v) in params.zip(args.iter()) {
            child.bind(p, v.clone());
        }
        // Evaluate let-bindings in declaration order, each visible to
        // the next and to the trailing expression. Validator already
        // rejected duplicates and parameter shadowing, so a clean
        // recipe binds linearly.
        for b in &decl.body.bindings {
            let v = self.eval_extraction(&b.value, &child)?;
            child.bind(&b.name, v);
        }
        self.eval_extraction(&decl.body.result, &child)
    }

    /// Async counterpart to `eval_extraction`. Same logic, but the pipe
    /// and direct-call dispatchers consult the registry's async table
    /// first and `.await` the resulting future when the call hits a
    /// transport-aware transform. Sync transforms still dispatch via
    /// the sync table — there's no behavioural difference for any
    /// expression that doesn't touch the network.
    ///
    /// `transport` is the bridge to the engine's `Transport`. Callers
    /// that don't have one pass [`NoTransport`]; any transport-aware
    /// transform reached through that path errors with
    /// [`EvalError::TransformRequiresTransport`].
    pub fn eval_extraction_async<'a>(
        &'a self,
        expr: &'a ExtractionExpr,
        scope: &'a Scope,
        transport: &'a dyn TransportContext,
    ) -> Pin<Box<dyn Future<Output = Result<EvalValue, EvalError>> + Send + 'a>> {
        Box::pin(async move {
            match expr {
                ExtractionExpr::Path(p) => self.eval_path(p, scope),
                ExtractionExpr::Literal(j) => Ok(EvalValue::from(j.clone())),
                ExtractionExpr::Template(t) => self
                    .render_template_async(t, scope, transport)
                    .await
                    .map(EvalValue::String),
                ExtractionExpr::Pipe(head, calls) => {
                    let mut v = self.eval_extraction_async(head, scope, transport).await?;
                    for call in calls {
                        v = self
                            .apply_pipe_call_async(&call.name, v, &call.args, scope, transport)
                            .await?;
                    }
                    Ok(v)
                }
                ExtractionExpr::Call { name, args } => {
                    self.apply_direct_call_async(name, args, scope, transport).await
                }
                ExtractionExpr::CaseOf { scrutinee, branches } => {
                    let v = self.eval_path(scrutinee, scope)?;
                    let label = match &v {
                        EvalValue::Bool(b) => b.to_string(),
                        EvalValue::String(s) => s.clone(),
                        EvalValue::Int(n) => n.to_string(),
                        EvalValue::Null => "null".into(),
                        other => format!("{other:?}"),
                    };
                    let mut default_arm: Option<&ExtractionExpr> = None;
                    for (l, arm) in branches {
                        if l == "_" {
                            default_arm = Some(arm);
                            continue;
                        }
                        if l == &label {
                            return self.eval_extraction_async(arm, scope, transport).await;
                        }
                    }
                    if let Some(arm) = default_arm {
                        return self.eval_extraction_async(arm, scope, transport).await;
                    }
                    Err(EvalError::CaseNoMatch { label })
                }
                ExtractionExpr::MapTo { path, .. } => self.eval_path(path, scope),
                ExtractionExpr::BinaryOp { op, lhs, rhs } => {
                    let l = self.eval_extraction_async(lhs, scope, transport).await?;
                    let r = self.eval_extraction_async(rhs, scope, transport).await?;
                    apply_binary(*op, l, r)
                }
                ExtractionExpr::Unary { op, operand } => {
                    let v = self.eval_extraction_async(operand, scope, transport).await?;
                    apply_unary(*op, v)
                }
                ExtractionExpr::StructLiteral { fields } => {
                    let mut out: IndexMap<String, EvalValue> = IndexMap::new();
                    for f in fields {
                        if out.contains_key(&f.field_name) {
                            return Err(EvalError::DuplicateStructField(f.field_name.clone()));
                        }
                        let v = self.eval_extraction_async(&f.expr, scope, transport).await?;
                        out.insert(f.field_name.clone(), v);
                    }
                    Ok(EvalValue::Object(out))
                }
                ExtractionExpr::Index { base, index } => {
                    let b = self.eval_extraction_async(base, scope, transport).await?;
                    let i = self.eval_extraction_async(index, scope, transport).await?;
                    apply_index(b, i)
                }
                ExtractionExpr::RegexLiteral(lit) => {
                    let re = crate::parse::parser::build_regex(&lit.pattern, &lit.flags)
                        .map_err(|e| {
                            EvalError::Generic(format!("regex /{}/{}: {e}", lit.pattern, lit.flags))
                        })?;
                    Ok(EvalValue::Regex(crate::eval::value::RegexValue {
                        pattern: lit.pattern.clone(),
                        flags: lit.flags.clone(),
                        re,
                    }))
                }
            }
        })
    }

    /// Async counterpart to `render_template`. Interpolated expressions
    /// route through `eval_extraction_async` so a template that pipes
    /// through a transport-aware transform resolves correctly.
    pub async fn render_template_async(
        &self,
        t: &Template,
        scope: &Scope,
        transport: &dyn TransportContext,
    ) -> Result<String, EvalError> {
        let mut out = String::new();
        for part in &t.parts {
            match part {
                TemplatePart::Literal(s) => out.push_str(s),
                TemplatePart::Interp(e) => {
                    let v = self.eval_extraction_async(e, scope, transport).await?;
                    out.push_str(&stringify(&v));
                }
            }
        }
        Ok(out)
    }

    async fn apply_pipe_call_async(
        &self,
        name: &str,
        head: EvalValue,
        args: &[ExtractionExpr],
        scope: &Scope,
        transport: &dyn TransportContext,
    ) -> Result<EvalValue, EvalError> {
        let mut resolved = Vec::with_capacity(args.len());
        for a in args {
            resolved.push(self.eval_extraction_async(a, scope, transport).await?);
        }
        if let Some(decl) = self.registry.get_user_fn(name) {
            let provided = 1 + resolved.len();
            return self
                .apply_user_fn_async(decl, Some(head), resolved, provided, scope, transport)
                .await;
        }
        if let Some(f) = self.registry.get_async(name) {
            return f(head, resolved, transport).await;
        }
        let f = self
            .registry
            .get(name)
            .ok_or_else(|| EvalError::UnknownTransform { name: name.into() })?;
        f(head, &resolved)
    }

    async fn apply_direct_call_async(
        &self,
        name: &str,
        args: &[ExtractionExpr],
        scope: &Scope,
        transport: &dyn TransportContext,
    ) -> Result<EvalValue, EvalError> {
        let mut resolved = Vec::with_capacity(args.len());
        for a in args {
            resolved.push(self.eval_extraction_async(a, scope, transport).await?);
        }
        if let Some(decl) = self.registry.get_user_fn(name) {
            let provided = resolved.len();
            return self
                .apply_user_fn_async(decl, None, resolved, provided, scope, transport)
                .await;
        }
        if let Some(f) = self.registry.get_async(name) {
            let head = scope.current.clone().unwrap_or(EvalValue::Null);
            return f(head, resolved, transport).await;
        }
        let f = self
            .registry
            .get(name)
            .ok_or_else(|| EvalError::UnknownTransform { name: name.into() })?;
        let head = scope.current.clone().unwrap_or(EvalValue::Null);
        f(head, &resolved)
    }

    fn apply_user_fn_async<'a>(
        &'a self,
        decl: &'a crate::ast::FnDecl,
        head: Option<EvalValue>,
        args: Vec<EvalValue>,
        provided: usize,
        scope: &'a Scope,
        transport: &'a dyn TransportContext,
    ) -> Pin<Box<dyn Future<Output = Result<EvalValue, EvalError>> + Send + 'a>> {
        Box::pin(async move {
            let expected = decl.params.len();
            if provided != expected {
                return Err(EvalError::FnArityMismatch {
                    name: decl.name.clone(),
                    expected,
                    got: provided,
                });
            }
            let mut child = Scope::new()
                .with_inputs(scope.inputs().clone())
                .with_secrets(scope.secrets_map().clone());
            let mut params = decl.params.iter();
            if let Some(h) = head
                && let Some(first) = params.next()
            {
                child.bind(first, h);
            }
            for (p, v) in params.zip(args) {
                child.bind(p, v);
            }
            for b in &decl.body.bindings {
                let v = self
                    .eval_extraction_async(&b.value, &child, transport)
                    .await?;
                child.bind(&b.name, v);
            }
            self.eval_extraction_async(&decl.body.result, &child, transport)
                .await
        })
    }
}

/// Apply `lhs op rhs` with the language's numeric-coercion rule:
///   * `Int op Int`:  `+`, `-`, `*` stay `Int`; `/` and `%` always
///     promote to `Double` (no integer division — `1/2`
///     is `0.5`, not `0`). Division by zero
///     (numerator+denominator both zero or just
///     denominator) is a domain error.
///   * Any `Double` operand promotes the other side to `Double`.
///   * `String + String` concatenates.
///   * Everything else (`1 + "x"`, etc.) is `TypeMismatch`.
///
/// Null on either side is `TypeMismatch` — we deliberately don't fold
/// null into the numeric rules, because `null + 1 → 1` is exactly the
/// kind of silent coercion that produces broken records downstream.
fn apply_binary(op: BinOp, lhs: EvalValue, rhs: EvalValue) -> Result<EvalValue, EvalError> {
    // String concat is the only non-numeric path.
    if let (BinOp::Add, EvalValue::String(a), EvalValue::String(b)) = (op, &lhs, &rhs) {
        return Ok(EvalValue::String(format!("{a}{b}")));
    }
    let (l, r) = (as_number(&lhs)?, as_number(&rhs)?);
    let promote = matches!((&l, &r), (Numeric::Double(_), _) | (_, Numeric::Double(_)));
    match op {
        BinOp::Add => Ok(combine_numeric(l, r, false, promote, |a, b| a + b, i64::checked_add)),
        BinOp::Sub => Ok(combine_numeric(l, r, false, promote, |a, b| a - b, i64::checked_sub)),
        BinOp::Mul => Ok(combine_numeric(l, r, false, promote, |a, b| a * b, i64::checked_mul)),
        BinOp::Div => {
            let (a, b) = (l.to_double(), r.to_double());
            if b == 0.0 {
                return Err(EvalError::ArithmeticDomain("division by zero".into()));
            }
            Ok(EvalValue::Double(a / b))
        }
        BinOp::Mod => {
            // Always promote to Double when either side is fractional;
            // pure Int % Int stays Int but a zero divisor is a domain
            // error rather than a panic.
            match (l, r) {
                (Numeric::Int(_), Numeric::Int(0)) => Err(EvalError::ArithmeticDomain(
                    "modulo by zero".into(),
                )),
                (Numeric::Int(a), Numeric::Int(b)) => Ok(EvalValue::Int(a % b)),
                (a, b) => {
                    let (af, bf) = (a.to_double(), b.to_double());
                    if bf == 0.0 {
                        return Err(EvalError::ArithmeticDomain("modulo by zero".into()));
                    }
                    Ok(EvalValue::Double(af % bf))
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Numeric {
    Int(i64),
    Double(f64),
}

impl Numeric {
    fn to_double(self) -> f64 {
        match self {
            Numeric::Int(n) => n as f64,
            Numeric::Double(n) => n,
        }
    }
}

/// Pull a numeric view out of an `EvalValue`. `TypeMismatch` for
/// anything else — including `Null`. Surfacing null-as-zero coercion
/// would silently soak up missing data.
fn as_number(v: &EvalValue) -> Result<Numeric, EvalError> {
    match v {
        EvalValue::Int(n) => Ok(Numeric::Int(*n)),
        EvalValue::Double(n) => Ok(Numeric::Double(*n)),
        other => Err(EvalError::TypeMismatch {
            expected: "number",
            actual: kind_of(other),
        }),
    }
}

/// `+`, `-`, `*` with checked-int fast path and Double fallback when
/// either side is fractional or the int op would overflow. The boolean
/// `_unused` reserves a slot for future "must-promote" flags (e.g. `/`
/// at this site) without rewriting every caller.
fn combine_numeric(
    a: Numeric,
    b: Numeric,
    _unused: bool,
    promote_to_double: bool,
    f_d: fn(f64, f64) -> f64,
    f_i: fn(i64, i64) -> Option<i64>,
) -> EvalValue {
    if promote_to_double {
        return EvalValue::Double(f_d(a.to_double(), b.to_double()));
    }
    match (a, b) {
        (Numeric::Int(x), Numeric::Int(y)) => match f_i(x, y) {
            Some(n) => EvalValue::Int(n),
            None => EvalValue::Double(f_d(x as f64, y as f64)),
        },
        (x, y) => EvalValue::Double(f_d(x.to_double(), y.to_double())),
    }
}

fn apply_unary(op: UnOp, v: EvalValue) -> Result<EvalValue, EvalError> {
    match op {
        UnOp::Neg => match v {
            EvalValue::Int(n) => match n.checked_neg() {
                Some(n) => Ok(EvalValue::Int(n)),
                None => Ok(EvalValue::Double(-(n as f64))),
            },
            EvalValue::Double(n) => Ok(EvalValue::Double(-n)),
            other => Err(EvalError::TypeMismatch {
                expected: "number",
                actual: kind_of(&other),
            }),
        },
    }
}

/// Expression-level `base[index]`. Strict (vs. the null-tolerant
/// path-level `[N]`): a non-Array base or out-of-bounds index is an
/// error, not `Null`. Authors write this form after `match`, where the
/// captures array always exists but a missing group is `Null` *inside*
/// the array; reaching past the array's length is a real bug.
fn apply_index(base: EvalValue, index: EvalValue) -> Result<EvalValue, EvalError> {
    let i = match index {
        EvalValue::Int(n) => n,
        other => {
            return Err(EvalError::TypeMismatch {
                expected: "integer",
                actual: kind_of(&other),
            });
        }
    };
    match base {
        EvalValue::Array(mut xs) => {
            let len = xs.len();
            let idx = resolve_index(i, len);
            match idx {
                Some(k) => Ok(xs.swap_remove(k)),
                None => Err(EvalError::IndexOutOfBounds { index: i, len }),
            }
        }
        EvalValue::NodeList(mut xs) => {
            let len = xs.len();
            let idx = resolve_index(i, len);
            match idx {
                Some(k) => Ok(EvalValue::Node(xs.swap_remove(k))),
                None => Err(EvalError::IndexOutOfBounds { index: i, len }),
            }
        }
        other => Err(EvalError::InvalidIndexBase {
            kind: kind_of(&other),
        }),
    }
}

/// Resolve a possibly-negative index against an array of `len` items. Returns
/// `None` if the index falls outside the array (including indexing into an
/// empty array, which is the common case for scrapers).
fn resolve_index(idx: i64, len: usize) -> Option<usize> {
    let i = if idx < 0 {
        let from_end = idx.unsigned_abs() as usize;
        if from_end > len {
            return None;
        }
        len - from_end
    } else {
        idx as usize
    };
    if i >= len { None } else { Some(i) }
}

fn field_of(v: &EvalValue, field: &str) -> EvalValue {
    match v {
        EvalValue::Object(o) => o.get(field).cloned().unwrap_or(EvalValue::Null),
        // `[*].field` projects: distribute field access across array elements.
        // Without this, `terpenes[*].name | dedup` blows up because `.name`
        // returns null on an Array, then dedup chokes on null.
        EvalValue::Array(xs) => EvalValue::Array(xs.iter().map(|x| field_of(x, field)).collect()),
        _ => EvalValue::Null,
    }
}

fn kind_of(v: &EvalValue) -> &'static str {
    match v {
        EvalValue::Null => "null",
        EvalValue::Bool(_) => "bool",
        EvalValue::Int(_) => "int",
        EvalValue::Double(_) => "double",
        EvalValue::String(_) => "string",
        EvalValue::Array(_) => "array",
        EvalValue::Object(_) => "object",
        EvalValue::Node(_) => "node",
        EvalValue::NodeList(_) => "nodelist",
        EvalValue::Ref { .. } => "ref",
        EvalValue::Regex(_) => "regex",
    }
}

/// String coercion used by templates and `toString`.
fn stringify(v: &EvalValue) -> String {
    match v {
        EvalValue::Null => String::new(),
        EvalValue::Bool(b) => b.to_string(),
        EvalValue::Int(n) => n.to_string(),
        EvalValue::Double(n) => n.to_string(),
        EvalValue::String(s) => s.clone(),
        EvalValue::Node(s) => s.clone(),
        EvalValue::Array(xs) => xs.iter().map(stringify).collect::<Vec<_>>().join(","),
        EvalValue::Object(o) => serde_json::to_string(
            &o.iter()
                .map(|(k, v)| (k.clone(), v.clone().into_json()))
                .collect::<indexmap::IndexMap<_, _>>(),
        )
        .unwrap_or_default(),
        EvalValue::NodeList(xs) => xs.join(","),
        // A ref stringifies to its target id — useful inside templates
        // and `toString` pipelines (e.g. logging, debug rendering). The
        // structured `{_ref, _type}` shape only appears in JSON-bound
        // contexts via `into_json`.
        EvalValue::Ref { id, .. } => id.clone(),
        // Regex stringifies to its source form. Useful for diagnostic
        // messages; shouldn't appear in production templates.
        EvalValue::Regex(r) => format!("/{}/{}", r.pattern, r.flags),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    fn eval_recipe_expr(src: &str) -> EvalValue {
        let r = parse(src).expect("parse");
        let registry = default_registry();
        let ev = Evaluator::new(registry);
        // The fixture wraps an extraction in `emit` so we can grab the expression.
        let Statement::Emit(em) = &r.body.statements()[0] else {
            panic!("expected emit")
        };
        let scope = Scope::new();
        ev.eval_extraction(&em.bindings[0].expr, &scope).unwrap()
    }

    #[test]
    fn literal_eval() {
        let v = eval_recipe_expr(
            r#"
            recipe "x"
            engine http
            type T { f: String }
            emit T { f ← "hello" }
        "#,
        );
        assert_eq!(v, EvalValue::String("hello".into()));
    }

    #[test]
    fn template_with_input() {
        let r = parse(
            r#"
            recipe "x"
            engine http
            input name: String
            type T { f: String }
            emit T { f ← "hi {$input.name}" }
        "#,
        )
        .unwrap();
        let registry = default_registry();
        let ev = Evaluator::new(registry);
        let inputs: indexmap::IndexMap<String, EvalValue> =
            [("name".to_string(), EvalValue::String("world".into()))]
                .into_iter()
                .collect();
        let scope = Scope::new().with_inputs(inputs);
        let Statement::Emit(em) = &r.body.statements()[0] else {
            panic!()
        };
        let v = ev.eval_extraction(&em.bindings[0].expr, &scope).unwrap();
        assert_eq!(v, EvalValue::String("hi world".into()));
    }

    #[test]
    fn pipe_dedup_lowercases() {
        let r = parse(
            r#"
            recipe "x"
            engine http
            type T { f: String }
            emit T { f ← "HELLO" | lower }
        "#,
        )
        .unwrap();
        let registry = default_registry();
        let ev = Evaluator::new(registry);
        let scope = Scope::new();
        let Statement::Emit(em) = &r.body.statements()[0] else {
            panic!()
        };
        let v = ev.eval_extraction(&em.bindings[0].expr, &scope).unwrap();
        assert_eq!(v, EvalValue::String("hello".into()));
    }

    #[test]
    fn index_out_of_bounds_returns_null() {
        // Regression for the live `remedy-baltimore` run: a recipe wrote
        // `$product.potencyThc?.range[0]` and the engine errored with
        // "path index 0 out of bounds for array of length 0" on records
        // where the potency range happened to be an empty array. Indexing
        // is now null-tolerant, matching the spirit of the `?.` chain.
        let r = parse(
            r#"
            recipe "x"
            engine http
            type T { x: Int? }
            emit T { x ← $p.range[0] }
        "#,
        )
        .unwrap();
        let registry = default_registry();
        let ev = Evaluator::new(registry);
        let Statement::Emit(em) = &r.body.statements()[0] else {
            panic!()
        };

        // Case 1: empty array.
        let mut scope = Scope::new();
        scope.bind(
            "p",
            EvalValue::Object(
                [("range".to_string(), EvalValue::Array(vec![]))]
                    .into_iter()
                    .collect(),
            ),
        );
        assert_eq!(
            ev.eval_extraction(&em.bindings[0].expr, &scope).unwrap(),
            EvalValue::Null,
        );

        // Case 2: missing field (so `.range` resolves to Null and `[0]` is
        // already Null-safe via the existing branch).
        let mut scope = Scope::new();
        scope.bind("p", EvalValue::Object(indexmap::IndexMap::new()));
        assert_eq!(
            ev.eval_extraction(&em.bindings[0].expr, &scope).unwrap(),
            EvalValue::Null,
        );

        // Case 3: in-bounds still works.
        let mut scope = Scope::new();
        scope.bind(
            "p",
            EvalValue::Object(
                [(
                    "range".to_string(),
                    EvalValue::Array(vec![EvalValue::Int(21), EvalValue::Int(28)]),
                )]
                .into_iter()
                .collect(),
            ),
        );
        assert_eq!(
            ev.eval_extraction(&em.bindings[0].expr, &scope).unwrap(),
            EvalValue::Int(21),
        );
    }

    #[test]
    fn wildcard_field_projects_over_array() {
        // `$xs[*].name` on an array of objects should yield an array of names,
        // not Null. Without field-on-array distribution, downstream collection
        // transforms (dedup, join, count) blow up because they see Null.
        let r = parse(
            r#"
            recipe "x"
            engine http
            type T { fs: [String] }
            emit T { fs ← $xs[*].name | dedup }
        "#,
        )
        .unwrap();
        let registry = default_registry();
        let ev = Evaluator::new(registry);
        let mut scope = Scope::new();
        let mk = |n: &str| -> EvalValue {
            EvalValue::Object(
                [("name".to_string(), EvalValue::String(n.into()))]
                    .into_iter()
                    .collect(),
            )
        };
        scope.bind(
            "xs",
            EvalValue::Array(vec![mk("alpha"), mk("beta"), mk("alpha")]),
        );
        let Statement::Emit(em) = &r.body.statements()[0] else {
            panic!()
        };
        let v = ev.eval_extraction(&em.bindings[0].expr, &scope).unwrap();
        assert_eq!(
            v,
            EvalValue::Array(vec![
                EvalValue::String("alpha".into()),
                EvalValue::String("beta".into()),
            ])
        );
    }

    #[test]
    fn opt_field_then_wildcard_collapses_to_empty_array() {
        // Reproduces the Sweed runtime error:
        //   "transform 'dedup': can only dedup arrays, got null"
        // when a recipe writes `$product.strain?.terpenes[*].name | dedup`
        // and the strain object is null. `?.terpenes` is null,
        // `[*]` widens null → [], `.name` projects → [] (now, with the
        // distribute-over-array fix), `dedup` succeeds with [].
        let r = parse(
            r#"
            recipe "x"
            engine http
            type T { fs: [String] }
            emit T { fs ← $product.strain?.terpenes[*].name | dedup }
        "#,
        )
        .unwrap();
        let registry = default_registry();
        let ev = Evaluator::new(registry);
        let mut scope = Scope::new();
        // Strain present but null.
        scope.bind(
            "product",
            EvalValue::Object(
                [("strain".to_string(), EvalValue::Null)]
                    .into_iter()
                    .collect(),
            ),
        );
        let Statement::Emit(em) = &r.body.statements()[0] else {
            panic!()
        };
        let v = ev.eval_extraction(&em.bindings[0].expr, &scope).unwrap();
        assert_eq!(v, EvalValue::Array(vec![]));
    }

    #[test]
    fn case_of_evaluates() {
        let r = parse(
            r#"
            recipe "x"
            engine http
            input flag: Bool
            type T { f: Int }
            emit T { f ← case $input.flag of { true → 1, false → 0 } }
        "#,
        )
        .unwrap();
        let registry = default_registry();
        let ev = Evaluator::new(registry);
        let inputs: indexmap::IndexMap<String, EvalValue> =
            [("flag".to_string(), EvalValue::Bool(true))]
                .into_iter()
                .collect();
        let scope = Scope::new().with_inputs(inputs);
        let Statement::Emit(em) = &r.body.statements()[0] else {
            panic!()
        };
        let v = ev.eval_extraction(&em.bindings[0].expr, &scope).unwrap();
        assert_eq!(v, EvalValue::Int(1));
    }

    fn eval_first_emit(src: &str) -> EvalValue {
        let r = parse(src).expect("parse");
        let registry =
            TransformRegistry::with_user_fns(default_registry(), r.functions.clone());
        let ev = Evaluator::new(&registry);
        let Statement::Emit(em) = &r.body.statements()[0] else {
            panic!("expected emit, got {:?}", r.body);
        };
        ev.eval_extraction(&em.bindings[0].expr, &Scope::new()).unwrap()
    }

    #[test]
    fn arithmetic_precedence_and_coercion() {
        // `a + b * c` reads as `a + (b * c)`; mixing Int and Double
        // promotes the result to Double; `/` always promotes.
        assert_eq!(
            eval_first_emit(
                r#"
                recipe "x"
                engine http
                type T { v: Double }
                emit T { v ← 1 + 2 * 3.0 }
                "#,
            ),
            EvalValue::Double(7.0),
        );
        assert_eq!(
            eval_first_emit(
                r#"
                recipe "x"
                engine http
                type T { v: Double }
                emit T { v ← 6 / 4 }
                "#,
            ),
            EvalValue::Double(1.5),
        );
        // Unary minus binds tighter than multiplication.
        assert_eq!(
            eval_first_emit(
                r#"
                recipe "x"
                engine http
                type T { v: Int }
                emit T { v ← -3 * 2 }
                "#,
            ),
            EvalValue::Int(-6),
        );
    }

    #[test]
    fn division_by_zero_surfaces_arithmetic_domain() {
        let r = parse(
            r#"
            recipe "x"
            engine http
            type T { v: Double }
            emit T { v ← 1 / 0 }
            "#,
        )
        .unwrap();
        let registry = default_registry();
        let ev = Evaluator::new(registry);
        let Statement::Emit(em) = &r.body.statements()[0] else {
            panic!()
        };
        let err = ev.eval_extraction(&em.bindings[0].expr, &Scope::new()).unwrap_err();
        assert!(matches!(err, EvalError::ArithmeticDomain(_)), "got {err:?}");
    }

    #[test]
    fn string_concat_is_only_string_op() {
        assert_eq!(
            eval_first_emit(
                r#"
                recipe "x"
                engine http
                type T { v: String }
                emit T { v ← "hi-" + "world" }
                "#,
            ),
            EvalValue::String("hi-world".into()),
        );
        // Mixed string + int is a type error — no silent coercion.
        let r = parse(
            r#"
            recipe "x"
            engine http
            type T { v: String }
            emit T { v ← "n=" + 7 }
            "#,
        )
        .unwrap();
        let registry = default_registry();
        let ev = Evaluator::new(registry);
        let Statement::Emit(em) = &r.body.statements()[0] else {
            panic!()
        };
        let err = ev.eval_extraction(&em.bindings[0].expr, &Scope::new()).unwrap_err();
        assert!(matches!(err, EvalError::TypeMismatch { .. }), "got {err:?}");
    }

    #[test]
    fn regex_match_captures_through_pipe() {
        // `$s | match(/.../) | <pull-field>` — the canonical shape for
        // the migrated `parseSize`-style transforms.
        let v = eval_first_emit(
            r#"
            recipe "x"
            engine http
            fn parseSize($s) {
                let $m = $s | match(/([0-9.]+)\s*([a-z]+)/)
                case $m.matched of {
                    true → { value: $m.captures[1] | parseFloat, unit: $m.captures[2] }
                    false → null
                }
            }
            type T { v: Double? }
            emit T { v ← "3.5 g" | parseSize | sizeJustValue }
            fn sizeJustValue($p) { $p.value }
            "#,
        );
        assert_eq!(v, EvalValue::Double(3.5));
    }

    #[test]
    fn struct_literal_evaluates_to_object() {
        let v = eval_first_emit(
            r#"
            recipe "x"
            engine http
            type T { v: Double? }
            emit T { v ← obj() | pickValue }
            fn obj() { { value: 7.0, unit: "g" } }
            fn pickValue($o) { $o.value }
            "#,
        );
        assert_eq!(v, EvalValue::Double(7.0));
    }

    #[test]
    fn let_bindings_visible_in_trailing_expression() {
        // Each let-binding shadows nothing previously bound; the trailing
        // expression sees them all.
        let v = eval_first_emit(
            r#"
            recipe "x"
            engine http
            fn compute($s) {
                let $a = $s * 2
                let $b = $a + 1
                $b
            }
            type T { v: Int }
            emit T { v ← 5 | compute }
            "#,
        );
        assert_eq!(v, EvalValue::Int(11));
    }

    #[test]
    fn case_of_with_default_arm() {
        // `_ → expr` matches anything no labelled arm caught; needed
        // for prevalenceNormalize-style fall-through.
        let v = eval_first_emit(
            r#"
            recipe "x"
            engine http
            fn classify($s) {
                case $s of {
                    "indica" → "INDICA"
                    "sativa" → "SATIVA"
                    _ → "OTHER"
                }
            }
            type T { v: String }
            emit T { v ← "hybrid" | classify }
            "#,
        );
        assert_eq!(v, EvalValue::String("OTHER".into()));
    }

    // --- transport-aware transform extension point -------------------------

    /// Mock `TransportContext` for the async-eval tests. Returns a
    /// fixed JSON object built from the requested URL.
    struct MockTransport;

    #[async_trait::async_trait]
    impl super::TransportContext for MockTransport {
        async fn fetch_json(&self, url: &str) -> Result<EvalValue, EvalError> {
            let mut obj = indexmap::IndexMap::new();
            obj.insert("url".to_string(), EvalValue::String(url.into()));
            obj.insert("ok".to_string(), EvalValue::Bool(true));
            Ok(EvalValue::Object(obj))
        }
    }

    #[tokio::test]
    async fn async_eval_routes_async_transform_through_transport() {
        // Registers an async transform that calls the transport directly
        // and returns the parsed JSON. Exercises the dispatch path used
        // by every future transport-aware built-in.
        let mut reg = TransformRegistry::default();
        reg.register_async("probe", |_head, args, ctx| {
            Box::pin(async move {
                let url = match args.first() {
                    Some(EvalValue::String(s)) => s.clone(),
                    _ => return Err(EvalError::Generic("probe needs a string url".into())),
                };
                ctx.fetch_json(&url).await
            })
        });
        let ev = Evaluator::new(&reg);
        let call = ExtractionExpr::Call {
            name: "probe".into(),
            args: vec![ExtractionExpr::Literal(JSONValue::String(
                "https://example.test/x".into(),
            ))],
        };
        let scope = Scope::new();
        let transport = MockTransport;
        let v = ev
            .eval_extraction_async(&call, &scope, &transport)
            .await
            .unwrap();
        let EvalValue::Object(o) = v else {
            panic!("expected object, got {v:?}");
        };
        assert_eq!(
            o.get("url"),
            Some(&EvalValue::String("https://example.test/x".into())),
        );
        assert_eq!(o.get("ok"), Some(&EvalValue::Bool(true)));
    }

    #[test]
    fn sync_eval_rejects_async_only_transform() {
        // The sync path has no transport — invoking a transform that
        // only exists in the async table must surface a typed error so
        // callers know to switch to the async API.
        let mut reg = TransformRegistry::default();
        reg.register_async("probe", |_h, _a, _c| {
            Box::pin(async move { Ok(EvalValue::Null) })
        });
        let ev = Evaluator::new(&reg);
        let call = ExtractionExpr::Call {
            name: "probe".into(),
            args: vec![],
        };
        let err = ev.eval_extraction(&call, &Scope::new()).unwrap_err();
        assert!(
            matches!(err, EvalError::TransformRequiresTransport { ref name } if name == "probe"),
            "got {err:?}",
        );
    }

    #[tokio::test]
    async fn async_eval_falls_through_to_sync_transforms() {
        // Async eval must still dispatch pure transforms — otherwise
        // every existing recipe breaks the moment the engine switches
        // over. Pipe `"hi" | upper` through the async path.
        let reg = default_registry();
        let ev = Evaluator::new(reg);
        let expr = ExtractionExpr::Pipe(
            Box::new(ExtractionExpr::Literal(JSONValue::String("hi".into()))),
            vec![TransformCall {
                name: "upper".into(),
                args: vec![],
            }],
        );
        let v = ev
            .eval_extraction_async(&expr, &Scope::new(), &NoTransport)
            .await
            .unwrap();
        assert_eq!(v, EvalValue::String("HI".into()));
    }
}
