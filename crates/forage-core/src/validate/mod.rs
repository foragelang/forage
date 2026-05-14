//! Semantic checker over the AST. Produces `Vec<ValidationIssue>` with
//! severities. Validation is best-effort — even if some checks fail,
//! others still run, so the user sees the full picture.
//!
//! Public entry: `validate(recipe, catalog) -> ValidationReport`. The
//! catalog folds in workspace-shared declarations files plus the
//! recipe's local types; recipes outside a workspace pass
//! `TypeCatalog::from_recipe(&recipe)` for lonely-recipe mode.

use serde::{Deserialize, Serialize};

use crate::ast::*;
use crate::workspace::TypeCatalog;

/// Top-level entry point. `catalog` is the merged type namespace for
/// this recipe — see `Workspace::catalog`. Lonely-recipe mode (no
/// surrounding `forage.toml`) passes `TypeCatalog::from_recipe(recipe)`.
pub fn validate(recipe: &Recipe, catalog: &TypeCatalog) -> ValidationReport {
    let mut v = Validator::new(recipe, catalog);
    v.run();
    ValidationReport { issues: v.issues }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValidationReport {
    pub issues: Vec<ValidationIssue>,
}

impl ValidationReport {
    pub fn has_errors(&self) -> bool {
        self.issues.iter().any(|i| i.severity == Severity::Error)
    }

    pub fn errors(&self) -> impl Iterator<Item = &ValidationIssue> {
        self.issues.iter().filter(|i| i.severity == Severity::Error)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValidationIssue {
    pub severity: Severity,
    pub code: ValidationCode,
    pub message: String,
    /// Byte range in the source pinpointing what the issue is about.
    /// `0..0` means "no specific location" (typically a recipe-wide
    /// invariant) and consumers should render it at the file root.
    #[serde(default)]
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ValidationCode {
    EngineMismatch,
    UnknownType,
    UnknownEnum,
    UnknownInput,
    UnknownSecret,
    UnknownStep,
    UnknownVariable,
    UnknownTransform,
    DuplicateType,
    DuplicateEnum,
    DuplicateInput,
    DuplicateSecret,
    DuplicateBinding,
    MissingRequiredField,
    MissingRefAssignment,
    RefTypeMismatch,
    UnknownField,
    UnknownEnumVariant,
    MissingBrowserConfig,
    UnexpectedBrowserConfig,
    AuthOnBrowserEngine,
    MissingAuthStep,
    /// Two `fn` declarations share a name. Calls would be ambiguous —
    /// only the first one would resolve.
    DuplicateFn,
    /// A `fn` declaration lists the same `$param` name twice.
    DuplicateParam,
    /// A `fn` parameter is named after a reserved engine binding
    /// (`page`, currently the only one). The lexer already rejects
    /// `$input` and `$secret` as parameter names (they're distinct
    /// token kinds), so this code only fires for engine-injected vars.
    ReservedParam,
    /// A user-fn declaration shares a name with a built-in transform.
    /// The call site resolves the user-fn first, masking the built-in;
    /// useful for testing, dangerous in production.
    ShadowsBuiltin,
    /// A call site passes the wrong number of arguments to a user-fn.
    WrongArity,
    /// A `fn` body references itself by name. Direct recursion compiles
    /// but the runtime will not terminate; emitted as a warning so the
    /// recipe still builds.
    RecursiveFunction,
}

/// Static list of built-in transforms — mirrors `eval::transforms::build_default`.
/// Keeping a separate list here so the validator doesn't need a registry
/// at construction time. If a recipe references a transform not in here,
/// it's flagged as Unknown.
pub const BUILTIN_TRANSFORMS: &[&str] = &[
    "toString",
    "lower",
    "upper",
    "trim",
    "capitalize",
    "titleCase",
    "parseInt",
    "parseFloat",
    "parseBool",
    "length",
    "dedup",
    "first",
    "coalesce",
    "default",
    "parseSize",
    "normalizeOzToGrams",
    "sizeValue",
    "sizeUnit",
    "normalizeUnitToGrams",
    "prevalenceNormalize",
    "parseJaneWeight",
    "janeWeightUnit",
    "janeWeightKey",
    "getField",
    "parseHtml",
    "parseJson",
    "select",
    "text",
    "attr",
    "html",
    "innerHtml",
];

struct Validator<'a> {
    recipe: &'a Recipe,
    catalog: &'a TypeCatalog,
    issues: Vec<ValidationIssue>,
    /// Variable bindings in scope at the current walking position. Includes
    /// step names (recipe-body-wide), for-loop variables (nested),
    /// htmlPrime-extracted vars (from auth or step.extract.regex.groups),
    /// and `emit … as $v` bindings.
    known_vars: std::collections::HashSet<String>,
    /// `emit … as $v` bindings active at the current walking position,
    /// mapping the bare identifier (no `$`) to the emit's target type
    /// name. Used for `Ref<T>` field type-checks: a `product ← $p`
    /// binding inside an `emit Variant {…}` is valid only when `$p` is
    /// in this map and points at `Product`.
    ref_bindings: std::collections::HashMap<String, String>,
    /// User-fn name → declared arity, collected before any body
    /// validation runs. Forward references resolve through this map.
    user_fn_arity: std::collections::HashMap<String, usize>,
    /// When validating a user-fn body, the enclosing fn's name. Body
    /// expressions reference it to surface direct-recursion warnings.
    enclosing_fn: Option<String>,
    /// Source range of the enclosing AST node being checked. Set by the
    /// callers as they descend (`with_span` / `Statement::span`) and read
    /// by `err_here` / `warn_here` so diagnostics inherit the smallest
    /// available location without every call needing to thread spans.
    cur_span: Span,
}

impl<'a> Validator<'a> {
    fn new(recipe: &'a Recipe, catalog: &'a TypeCatalog) -> Self {
        let mut known_vars = std::collections::HashSet::new();
        collect_bindings(&recipe.body, &mut known_vars);
        if let Some(b) = &recipe.browser {
            for cap in &b.captures {
                known_vars.insert(cap.iter_var.clone());
                collect_bindings(&cap.body, &mut known_vars);
            }
            if let Some(doc) = &b.document_capture {
                known_vars.insert(doc.iter_var.clone());
                collect_bindings(&doc.body, &mut known_vars);
            }
        }
        // Auth.htmlPrime captured vars.
        if let Some(AuthStrategy::HtmlPrime { captured_vars, .. }) = &recipe.auth {
            for v in captured_vars {
                known_vars.insert(v.var_name.clone());
            }
        }
        // Engine-injected variables: the HTTP engine binds `$page` inside
        // every step so recipes can template page numbers into bodies or
        // URLs (Leafbridge's `prods_pageNumber`, Sweed's `page`).
        known_vars.insert("page".into());
        // Collect user-fn arities up front so forward references and
        // mutual lookups resolve. Duplicates surface in
        // `check_user_fns` — the map only keeps the first arity since
        // a duplicate emits an error anyway.
        let mut user_fn_arity = std::collections::HashMap::new();
        for f in &recipe.functions {
            user_fn_arity
                .entry(f.name.clone())
                .or_insert(f.params.len());
        }
        Self {
            recipe,
            catalog,
            issues: Vec::new(),
            known_vars,
            ref_bindings: std::collections::HashMap::new(),
            user_fn_arity,
            enclosing_fn: None,
            cur_span: 0..0,
        }
    }

    /// Run `f` with `cur_span` temporarily set to `span`. Restores the
    /// previous span on the way out. Used by checks that descend into a
    /// new locatable construct (Step, Emit, ForLoop, …) and want inner
    /// `err_here` calls to anchor at that construct.
    fn with_span<R>(&mut self, span: Span, f: impl FnOnce(&mut Self) -> R) -> R {
        let saved = std::mem::replace(&mut self.cur_span, span);
        let r = f(self);
        self.cur_span = saved;
        r
    }

    /// Emit an issue anchored at the current span.
    fn err_here(&mut self, code: ValidationCode, message: impl Into<String>) {
        self.err(self.cur_span.clone(), code, message);
    }
    fn warn_here(&mut self, code: ValidationCode, message: impl Into<String>) {
        self.warn(self.cur_span.clone(), code, message);
    }

    fn err(&mut self, span: Span, code: ValidationCode, message: impl Into<String>) {
        self.issues.push(ValidationIssue {
            severity: Severity::Error,
            code,
            message: message.into(),
            span,
        });
    }

    fn warn(&mut self, span: Span, code: ValidationCode, message: impl Into<String>) {
        self.issues.push(ValidationIssue {
            severity: Severity::Warning,
            code,
            message: message.into(),
            span,
        });
    }

    /// Cross-cutting issue with no specific source location (engine
    /// consistency, missing-recipe-wide-decl). Renders at the file root.
    fn err_recipe(&mut self, code: ValidationCode, message: impl Into<String>) {
        self.err(0..0, code, message);
    }
    fn warn_recipe(&mut self, code: ValidationCode, message: impl Into<String>) {
        self.warn(0..0, code, message);
    }

    fn run(&mut self) {
        self.check_duplicates();
        self.check_engine_consistency();
        self.check_user_fns();
        self.check_references();
        self.check_emit_records();
    }

    /// Walk every `fn` declaration: duplicate detection, parameter rules,
    /// shadow-of-builtin warning, body validation in a fresh scope, and
    /// direct-recursion warning.
    fn check_user_fns(&mut self) {
        let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for f in &self.recipe.functions.clone() {
            self.with_span(f.span.clone(), |v| {
                if !seen.insert(f.name.as_str()) {
                    v.err_here(
                        ValidationCode::DuplicateFn,
                        format!("function '{}' declared more than once", f.name),
                    );
                }
                if BUILTIN_TRANSFORMS.contains(&f.name.as_str()) {
                    v.warn_here(
                        ValidationCode::ShadowsBuiltin,
                        format!(
                            "function '{}' shadows a built-in transform of the same name",
                            f.name,
                        ),
                    );
                }
                let mut param_seen: std::collections::HashSet<&str> =
                    std::collections::HashSet::new();
                for p in &f.params {
                    if !param_seen.insert(p.as_str()) {
                        v.err_here(
                            ValidationCode::DuplicateParam,
                            format!(
                                "function '{}' declares parameter '${}' more than once",
                                f.name, p,
                            ),
                        );
                    }
                    if p == "page" {
                        v.err_here(
                            ValidationCode::ReservedParam,
                            format!(
                                "function '{}' parameter '${}' shadows the engine-injected '$page' binding",
                                f.name, p,
                            ),
                        );
                    }
                }

                // Validate the body in a closed scope: only the params
                // are visible. The recipe-level `$secret.*` / `$input.*`
                // remain accessible through their dedicated path heads
                // (not `known_vars`), so we don't need to inject them here.
                let saved_vars = std::mem::take(&mut v.known_vars);
                let saved_refs = std::mem::take(&mut v.ref_bindings);
                let mut body_vars: std::collections::HashSet<String> =
                    std::collections::HashSet::new();
                for p in &f.params {
                    body_vars.insert(p.clone());
                }
                v.known_vars = body_vars;
                let saved_fn = v.enclosing_fn.replace(f.name.clone());
                v.check_extraction(&f.body);
                v.enclosing_fn = saved_fn;
                v.known_vars = saved_vars;
                v.ref_bindings = saved_refs;
            });
        }
    }

    // --- duplicates --------------------------------------------------------

    fn check_duplicates(&mut self) {
        let mut seen_types = std::collections::HashSet::new();
        for t in &self.recipe.types {
            if !seen_types.insert(&t.name) {
                self.err(
                    t.span.clone(),
                    ValidationCode::DuplicateType,
                    format!("type '{}' declared more than once", t.name),
                );
            }
        }
        let mut seen_enums = std::collections::HashSet::new();
        for e in &self.recipe.enums {
            if !seen_enums.insert(&e.name) {
                self.err(
                    e.span.clone(),
                    ValidationCode::DuplicateEnum,
                    format!("enum '{}' declared more than once", e.name),
                );
            }
        }
        let mut seen_inputs = std::collections::HashSet::new();
        for i in &self.recipe.inputs {
            if !seen_inputs.insert(&i.name) {
                self.err(
                    i.span.clone(),
                    ValidationCode::DuplicateInput,
                    format!("input '{}' declared more than once", i.name),
                );
            }
        }
        let mut seen_secrets = std::collections::HashSet::new();
        for s in &self.recipe.secrets {
            if !seen_secrets.insert(s) {
                self.err_recipe(
                    ValidationCode::DuplicateSecret,
                    format!("secret '{s}' declared more than once"),
                );
            }
        }
    }

    // --- engine consistency ------------------------------------------------

    fn check_engine_consistency(&mut self) {
        match self.recipe.engine_kind {
            EngineKind::Http => {
                if self.recipe.browser.is_some() {
                    self.err_recipe(
                        ValidationCode::UnexpectedBrowserConfig,
                        "HTTP-engine recipe must not declare a `browser { … }` block",
                    );
                }
            }
            EngineKind::Browser => {
                if self.recipe.browser.is_none() {
                    self.err_recipe(
                        ValidationCode::MissingBrowserConfig,
                        "browser-engine recipe must declare a `browser { … }` block",
                    );
                }
                if matches!(self.recipe.auth, Some(AuthStrategy::Session(_))) {
                    self.warn_recipe(
                        ValidationCode::AuthOnBrowserEngine,
                        "auth.session.* on a browser-engine recipe — credentials are best handled inside the browser flow",
                    );
                }
            }
        }
        if let Some(AuthStrategy::HtmlPrime { step_name, .. }) = &self.recipe.auth {
            let referenced = self
                .recipe
                .body
                .iter()
                .any(|s| matches!(s, Statement::Step(st) if &st.name == step_name));
            if !referenced {
                self.err_recipe(
                    ValidationCode::MissingAuthStep,
                    format!("auth.htmlPrime references step '{step_name}' which is not declared"),
                );
            }
        }
    }

    // --- name resolution ---------------------------------------------------

    fn check_references(&mut self) {
        for s in self.recipe.body.clone() {
            self.check_statement(&s);
        }
        if let Some(b) = &self.recipe.browser {
            self.check_template(&b.initial_url);
            if let Some(i) = &b.interactive {
                if let Some(u) = &i.bootstrap_url {
                    self.check_template(u);
                }
            }
            for cap in &b.captures.clone() {
                self.check_extraction(&cap.iter_path);
                let inserted = self.known_vars.insert(cap.iter_var.clone());
                for s in &cap.body {
                    self.check_statement(s);
                }
                if inserted {
                    self.known_vars.remove(&cap.iter_var);
                }
            }
            if let Some(doc) = &b.document_capture.clone() {
                self.check_extraction(&doc.iter_path);
                let inserted = self.known_vars.insert(doc.iter_var.clone());
                for s in &doc.body {
                    self.check_statement(s);
                }
                if inserted {
                    self.known_vars.remove(&doc.iter_var);
                }
            }
        }
    }

    fn check_statement(&mut self, s: &Statement) {
        let span = s.span().clone();
        self.with_span(span, |v| match s {
            Statement::Step(step) => {
                v.check_template(&step.request.url);
                for (_, hv) in &step.request.headers {
                    v.check_template(hv);
                }
                if let Some(b) = &step.request.body {
                    v.check_body(b);
                }
            }
            Statement::Emit(em) => {
                v.check_emit(em);
                // After the emit, its `as $v` binding (if any) is in
                // scope for subsequent statements in the same lexical
                // body. Tracked separately from `known_vars` so the
                // type-checker can ask "is $p a ref?" and "to what?".
                //
                // Both directions of name collision are errors:
                //   1. `$v` already bound by another `emit … as $v` in
                //      the same scope (DuplicateBinding).
                //   2. `$v` shadowing a `for $v in …` loop variable —
                //      the for-direction is caught in the ForLoop arm
                //      below; this is the symmetric as-side check that
                //      otherwise lets a recipe silently rebind the
                //      loop variable mid-iteration.
                if let Some(name) = &em.bind_name {
                    if v.ref_bindings.contains_key(name) {
                        v.err_here(
                            ValidationCode::DuplicateBinding,
                            format!(
                                "binding '${name}' is already declared in this scope",
                            ),
                        );
                    } else if v.known_vars.contains(name) {
                        v.err_here(
                            ValidationCode::DuplicateBinding,
                            format!(
                                "`as ${name}` shadows the for-loop variable '${name}' in this scope",
                            ),
                        );
                    } else {
                        v.ref_bindings
                            .insert(name.clone(), em.type_name.clone());
                    }
                }
            }
            Statement::ForLoop {
                variable,
                collection,
                body,
                ..
            } => {
                v.check_extraction(collection);
                // Lexical scoping for for-loop bodies: any `as $v` (and
                // any inner for-loop variable) that appears inside must
                // disappear when the loop ends, so siblings can't see
                // it. Snapshot the binding state on entry and restore on
                // exit.
                let inserted = v.known_vars.insert(variable.clone());
                let saved_refs = v.ref_bindings.clone();
                if v.ref_bindings.contains_key(variable) {
                    v.err_here(
                        ValidationCode::DuplicateBinding,
                        format!(
                            "for-loop variable '${variable}' shadows an `as` binding from an enclosing scope",
                        ),
                    );
                }
                for s in body {
                    v.check_statement(s);
                }
                v.ref_bindings = saved_refs;
                if inserted {
                    v.known_vars.remove(variable);
                }
            }
        });
    }

    fn check_emit(&mut self, em: &Emission) {
        self.with_span(em.span.clone(), |v| {
            let Some(ty) = v.catalog.ty(&em.type_name).cloned() else {
                v.err_here(
                    ValidationCode::UnknownType,
                    format!("emit Type '{}' is not declared", em.type_name),
                );
                return;
            };
            let bound: std::collections::HashSet<&str> =
                em.bindings.iter().map(|b| b.field_name.as_str()).collect();
            // Required fields must be bound; `Ref<T>` fields are
            // *always* required regardless of the `optional` flag —
            // there is no implicit-null ref. (We re-flag them with the
            // dedicated `MissingRefAssignment` code so authors get a
            // clearer message than the generic missing-field one.)
            for f in &ty.fields {
                if !bound.contains(f.name.as_str()) {
                    if matches!(f.ty, FieldType::Ref(_)) {
                        v.err_here(
                            ValidationCode::MissingRefAssignment,
                            format!(
                                "emit {} missing required Ref field '{}' (every Ref<T> field must be explicitly bound)",
                                em.type_name, f.name,
                            ),
                        );
                    } else if !f.optional {
                        v.err_here(
                            ValidationCode::MissingRequiredField,
                            format!("emit {} missing required field '{}'", em.type_name, f.name),
                        );
                    }
                }
            }
            for b in &em.bindings {
                match ty.field(&b.field_name) {
                    None => {
                        v.err_here(
                            ValidationCode::UnknownField,
                            format!("type {} has no field '{}'", em.type_name, b.field_name),
                        );
                    }
                    Some(f) => {
                        if let FieldType::Ref(target) = &f.ty {
                            v.check_ref_expr(target, &b.field_name, &em.type_name, &b.expr);
                        }
                    }
                }
                v.check_extraction(&b.expr);
            }
        });
    }

    /// The RHS of a `Ref<T>` field binding must evaluate to a
    /// `Ref<T>` value. The only construct that produces one is a path
    /// expression naming a variable bound via `emit … as $v` — so this
    /// check rejects literals, templates, pipes, and case-ofs outright,
    /// and (for path expressions) requires the head variable to live in
    /// `ref_bindings` with a matching target type.
    fn check_ref_expr(
        &mut self,
        target: &str,
        field_name: &str,
        emit_type: &str,
        expr: &ExtractionExpr,
    ) {
        let var = match expr {
            ExtractionExpr::Path(PathExpr::Variable(name)) => Some(name),
            _ => None,
        };
        let Some(var) = var else {
            self.err_here(
                ValidationCode::RefTypeMismatch,
                format!(
                    "field '{emit_type}.{field_name}' is Ref<{target}>; expected a `$name` introduced by `emit {target} {{…}} as $name`",
                ),
            );
            return;
        };
        match self.ref_bindings.get(var) {
            None => {
                self.err_here(
                    ValidationCode::RefTypeMismatch,
                    format!(
                        "field '{emit_type}.{field_name}' expects a Ref<{target}>; '${var}' is not an `emit … as $name` binding",
                    ),
                );
            }
            Some(bound_type) if bound_type != target => {
                self.err_here(
                    ValidationCode::RefTypeMismatch,
                    format!(
                        "field '{emit_type}.{field_name}' expects Ref<{target}> but '${var}' is Ref<{bound_type}>",
                    ),
                );
            }
            Some(_) => {}
        }
    }

    fn check_extraction(&mut self, e: &ExtractionExpr) {
        match e {
            ExtractionExpr::Path(p) => self.check_path(p),
            ExtractionExpr::Pipe(inner, calls) => {
                self.check_extraction(inner);
                for c in calls {
                    // Pipe call: head is param 0, explicit args fill the rest.
                    self.check_call_site(&c.name, c.args.len() + 1);
                    for a in &c.args {
                        self.check_extraction(a);
                    }
                }
            }
            ExtractionExpr::CaseOf {
                scrutinee,
                branches,
            } => {
                self.check_path(scrutinee);
                self.check_case_branches(scrutinee, branches.iter().map(|(l, _)| l.as_str()));
                for (_, arm) in branches {
                    self.check_extraction(arm);
                }
            }
            ExtractionExpr::MapTo { path, emission } => {
                self.check_path(path);
                self.check_emit(emission);
            }
            ExtractionExpr::Template(t) => self.check_template(t),
            ExtractionExpr::Call { name, args } => {
                // Direct call: every arg is explicit.
                self.check_call_site(name, args.len());
                for a in args {
                    self.check_extraction(a);
                }
            }
            ExtractionExpr::Literal(_) => {}
        }
    }

    /// Resolve a transform-name reference against user fns first, then
    /// the built-in registry. `call_arity` is the total number of values
    /// the call site supplies (head + explicit args at a pipe site,
    /// explicit args at a direct-call site).
    fn check_call_site(&mut self, name: &str, call_arity: usize) {
        if let Some(declared) = self.user_fn_arity.get(name).copied() {
            if declared != call_arity {
                self.err_here(
                    ValidationCode::WrongArity,
                    format!(
                        "function '{name}' expects {declared} argument{}, got {call_arity}",
                        if declared == 1 { "" } else { "s" },
                    ),
                );
            }
            // Direct recursion: the body of `enclosing_fn` references
            // itself by name. The runtime won't terminate; surface it
            // as a warning so the recipe still builds.
            if Some(name) == self.enclosing_fn.as_deref() {
                self.warn_here(
                    ValidationCode::RecursiveFunction,
                    format!("function '{name}' calls itself; the runtime has no recursion guard",),
                );
            }
            return;
        }
        if !BUILTIN_TRANSFORMS.contains(&name) {
            self.err_here(
                ValidationCode::UnknownTransform,
                format!("transform '{name}' is not registered"),
            );
        }
    }

    fn check_case_branches<'b>(
        &mut self,
        scrutinee: &PathExpr,
        labels: impl Iterator<Item = &'b str>,
    ) {
        // Detect enum scrutinees: walk the path; if the leaf is `$input.X`
        // where X has an EnumRef type, check label set against the enum.
        if let Some(enum_name) = self.enum_for_path(scrutinee) {
            if let Some(en) = self.catalog.recipe_enum(&enum_name).cloned() {
                let known: std::collections::HashSet<&str> =
                    en.variants.iter().map(|s| s.as_str()).collect();
                let used: Vec<String> = labels.map(|s| s.to_string()).collect();
                for l in &used {
                    if !known.contains(l.as_str()) {
                        self.err_here(
                            ValidationCode::UnknownEnumVariant,
                            format!("case label '{l}' is not a variant of enum {enum_name}"),
                        );
                    }
                }
                let used_set: std::collections::HashSet<&str> =
                    used.iter().map(|s| s.as_str()).collect();
                for v in &en.variants {
                    if !used_set.contains(v.as_str()) {
                        self.warn_here(
                            ValidationCode::UnknownEnumVariant,
                            format!("case-of over {enum_name} doesn't cover variant '{v}'"),
                        );
                    }
                }
            }
        }
    }

    fn enum_for_path(&self, p: &PathExpr) -> Option<String> {
        match p {
            PathExpr::Field(base, field) | PathExpr::OptField(base, field) => {
                // `$input.<name>` of an EnumRef type.
                if let PathExpr::Input = base.as_ref() {
                    if let Some(inp) = self.recipe.input(field) {
                        if let FieldType::EnumRef(name) = &inp.ty {
                            return Some(name.clone());
                        }
                    }
                }
                None
            }
            _ => None,
        }
    }

    fn check_path(&mut self, p: &PathExpr) {
        match p {
            PathExpr::Secret(name) => {
                if !self.recipe.secrets.iter().any(|s| s == name) {
                    self.err_here(
                        ValidationCode::UnknownSecret,
                        format!("$secret.{name} references an undeclared secret"),
                    );
                }
            }
            PathExpr::Variable(name) => {
                // `as $v` bindings live in `ref_bindings` (scope-tracked
                // per body); for-loop vars / step names / regex captures
                // live in `known_vars`. A `$name` reference is valid if
                // it's present in either — out-of-scope references to
                // an `as` binding fall through both checks and surface
                // here, regardless of whether the receiving field is a
                // Ref or a plain expression.
                if !self.known_vars.contains(name) && !self.ref_bindings.contains_key(name) {
                    self.err_here(
                        ValidationCode::UnknownVariable,
                        format!("$ {name} is an unbound variable"),
                    );
                }
            }
            PathExpr::Field(base, field) | PathExpr::OptField(base, field) => {
                // `$input.X` — check X is declared.
                if let PathExpr::Input = base.as_ref() {
                    if self.recipe.input(field).is_none() {
                        self.err_here(
                            ValidationCode::UnknownInput,
                            format!("$input.{field} references an undeclared input"),
                        );
                    }
                }
                self.check_path(base);
            }
            PathExpr::Index(base, _) | PathExpr::Wildcard(base) => self.check_path(base),
            PathExpr::Current | PathExpr::Input => {}
        }
    }

    fn check_template(&mut self, t: &Template) {
        for p in &t.parts {
            if let TemplatePart::Interp(e) = p {
                self.check_extraction(e);
            }
        }
    }

    fn check_body(&mut self, b: &HTTPBody) {
        match b {
            HTTPBody::JsonObject(kvs) => {
                for kv in kvs {
                    self.check_body_value(&kv.value);
                }
            }
            HTTPBody::Form(kvs) => {
                for (_, v) in kvs {
                    self.check_body_value(v);
                }
            }
            HTTPBody::Raw(t) => self.check_template(t),
        }
    }

    fn check_body_value(&mut self, v: &BodyValue) {
        match v {
            BodyValue::TemplateString(t) => self.check_template(t),
            BodyValue::Path(p) => self.check_path(p),
            BodyValue::Object(kvs) => {
                for kv in kvs {
                    self.check_body_value(&kv.value);
                }
            }
            BodyValue::Array(xs) => {
                for x in xs {
                    self.check_body_value(x);
                }
            }
            BodyValue::CaseOf {
                scrutinee,
                branches,
            } => {
                self.check_path(scrutinee);
                for (_, val) in branches {
                    self.check_body_value(val);
                }
            }
            BodyValue::Literal(_) => {}
        }
    }

    fn check_emit_records(&mut self) {
        // Verify each declared type's field types either resolve to a
        // primitive, an Array of one, an EnumRef to a declared enum, or a
        // Record reference to a declared type.
        for ty in &self.recipe.types.clone() {
            self.with_span(ty.span.clone(), |v| {
                for f in &ty.fields {
                    v.check_field_type(&f.ty, &format!("type {}.{}", ty.name, f.name));
                }
            });
        }
        for inp in &self.recipe.inputs.clone() {
            self.with_span(inp.span.clone(), |v| {
                v.check_field_type(&inp.ty, &format!("input {}", inp.name));
            });
        }
    }

    fn check_field_type(&mut self, t: &FieldType, where_: &str) {
        match t {
            FieldType::Array(inner) => self.check_field_type(inner, where_),
            FieldType::Record(name) => {
                if self.catalog.ty(name).is_none() && self.catalog.recipe_enum(name).is_none() {
                    self.err_here(
                        ValidationCode::UnknownType,
                        format!("{where_} references unknown type '{name}'"),
                    );
                }
            }
            FieldType::EnumRef(name) => {
                if self.catalog.recipe_enum(name).is_none() {
                    self.err_here(
                        ValidationCode::UnknownEnum,
                        format!("{where_} references unknown enum '{name}'"),
                    );
                }
            }
            FieldType::Ref(name) => {
                if self.catalog.ty(name).is_none() {
                    self.err_here(
                        ValidationCode::UnknownType,
                        format!("{where_} references unknown type 'Ref<{name}>'"),
                    );
                }
            }
            FieldType::String | FieldType::Int | FieldType::Double | FieldType::Bool => {}
        }
    }
}

/// Walk a body recursively and collect every globally-known variable
/// name. Used to seed `known_vars` so `$x` references in extraction
/// expressions resolve against the full set of step names, regex
/// captures, and for-loop variables — `check_path` doesn't have to
/// know about lexical scope to accept a reference into an enclosing
/// for-loop.
///
/// `emit … as $v` bindings are deliberately excluded. They live in
/// `ref_bindings`, which is scope-tracked (snapshotted on for-loop
/// entry, restored on exit), so the Emit branch in `check_statement`
/// catches out-of-scope `$v` references symmetrically with the in-scope
/// shadow check.
fn collect_bindings(body: &[Statement], out: &mut std::collections::HashSet<String>) {
    for s in body {
        match s {
            Statement::Step(step) => {
                out.insert(step.name.clone());
                if let Some(ex) = &step.extract {
                    for g in &ex.groups {
                        out.insert(g.clone());
                    }
                }
            }
            Statement::ForLoop { variable, body, .. } => {
                out.insert(variable.clone());
                collect_bindings(body, out);
            }
            Statement::Emit(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    #[test]
    fn clean_recipe_has_no_errors() {
        let src = r#"
            recipe "ok"
            engine http
            type Item { id: String }
            input limit: Int
            step list {
                method "GET"
                url    "https://example.com/items?limit={$input.limit}"
            }
            for $x in $list.items[*] {
                emit Item { id ← $x.id }
            }
        "#;
        let r = parse(src).unwrap();
        let cat = TypeCatalog::from_recipe(&r);
        let rep = validate(&r, &cat);
        assert!(!rep.has_errors(), "got errors: {:?}", rep.issues);
    }

    #[test]
    fn unknown_input_flagged() {
        let src = r#"
            recipe "bad"
            engine http
            type Item { id: String }
            step list {
                method "GET"
                url    "https://example.com/items?limit={$input.notDeclared}"
            }
            for $x in $list.items[*] {
                emit Item { id ← $x.id }
            }
        "#;
        let r = parse(src).unwrap();
        let cat = TypeCatalog::from_recipe(&r);
        let rep = validate(&r, &cat);
        assert!(rep.has_errors());
        assert!(
            rep.errors()
                .any(|i| matches!(i.code, ValidationCode::UnknownInput))
        );
    }

    #[test]
    fn missing_required_field_flagged() {
        let src = r#"
            recipe "bad"
            engine http
            type Item { id: String, name: String }
            step list {
                method "GET"
                url "https://example.com"
            }
            for $x in $list.items[*] {
                emit Item { id ← $x.id }
            }
        "#;
        let r = parse(src).unwrap();
        let cat = TypeCatalog::from_recipe(&r);
        let rep = validate(&r, &cat);
        assert!(
            rep.errors()
                .any(|i| matches!(i.code, ValidationCode::MissingRequiredField))
        );
    }

    #[test]
    fn validation_issues_carry_span_to_their_construct() {
        // Without spans on ValidationIssue, the LSP anchored every
        // diagnostic at byte 0 of the file. Pin the validator to
        // attach the span of the construct the issue is about:
        //
        // - duplicate type → the duplicate type-decl block
        // - missing required field → the emit block
        // - unknown transform → the enclosing emit (until expression
        //   spans land; granularity ≥ statement is the contract)
        let src = r#"recipe "spans"
engine http
type Item { id: String }
type Item { name: String }
step list {
    method "GET"
    url "https://example.com"
}
for $x in $list.items[*] {
    emit Item { id ← $x.id | nopeTransform }
}
emit Item { }
"#;
        let r = parse(src).unwrap();
        let cat = TypeCatalog::from_recipe(&r);
        let rep = validate(&r, &cat);

        let dup = rep
            .issues
            .iter()
            .find(|i| i.code == ValidationCode::DuplicateType)
            .expect("DuplicateType");
        assert!(dup.span.start < dup.span.end, "empty span: {dup:?}");
        // The duplicate is the *second* `type Item { … }` block. The
        // first is at the canonical location; the validator emits the
        // duplicate at the second, so the span should slice that one.
        assert!(
            src[dup.span.clone()].starts_with("type Item { name"),
            "got {:?}",
            &src[dup.span.clone()],
        );

        let unk = rep
            .issues
            .iter()
            .find(|i| i.code == ValidationCode::UnknownTransform)
            .expect("UnknownTransform");
        // Anchored at the enclosing emit (granularity ≥ statement).
        assert!(unk.span.start < unk.span.end);
        assert!(
            src[unk.span.clone()].starts_with("emit Item { id ← $x.id | nopeTransform }"),
            "got {:?}",
            &src[unk.span.clone()],
        );

        // The bare `emit Item { }` at the bottom should still be
        // flagged — though there may be other MissingRequiredField
        // issues earlier in the issues list (duplicate `type Item`
        // declarations mean the catalog adopts the *last* declared
        // shape, so `id ← …` no longer satisfies the schema).
        let missing = rep
            .issues
            .iter()
            .filter(|i| i.code == ValidationCode::MissingRequiredField)
            .find(|i| src[i.span.clone()].starts_with("emit Item { }"))
            .expect("MissingRequiredField on the bare emit");
        assert!(missing.span.start < missing.span.end);
    }

    #[test]
    fn http_recipe_with_browser_block_flagged() {
        let src = r#"
            recipe "bad"
            engine http
            type Item { id: String }
            browser {
                initialURL: "x"
                observe:    "y"
                paginate browserPaginate.scroll { until: noProgressFor(1) }
            }
        "#;
        let r = parse(src).unwrap();
        let cat = TypeCatalog::from_recipe(&r);
        let rep = validate(&r, &cat);
        assert!(
            rep.errors()
                .any(|i| matches!(i.code, ValidationCode::UnexpectedBrowserConfig))
        );
    }

    #[test]
    fn undeclared_secret_flagged() {
        let src = r#"
            recipe "bad"
            engine http
            type Item { id: String }
            step list {
                method "GET"
                url "https://example.com/{$secret.token}"
            }
            for $x in $list.items[*] {
                emit Item { id ← $x.id }
            }
        "#;
        let r = parse(src).unwrap();
        let cat = TypeCatalog::from_recipe(&r);
        let rep = validate(&r, &cat);
        assert!(
            rep.errors()
                .any(|i| matches!(i.code, ValidationCode::UnknownSecret))
        );
    }

    #[test]
    fn ref_to_unknown_target_type_flagged() {
        let src = r#"
            recipe "bad"
            engine http
            type Variant { product: Ref<DoesNotExist>, id: String }
            step list { method "GET" url "https://x.test" }
            for $p in $list[*] {
                emit Variant { id ← $p.id, product ← $missing }
            }
        "#;
        let r = parse(src).unwrap();
        let cat = TypeCatalog::from_recipe(&r);
        let rep = validate(&r, &cat);
        assert!(
            rep.errors().any(|i| i.code == ValidationCode::UnknownType
                && i.message.contains("Ref<DoesNotExist>")),
            "expected UnknownType for Ref<DoesNotExist>; got {:?}",
            rep.issues,
        );
    }

    #[test]
    fn missing_ref_assignment_flagged() {
        // A `Ref<T>` field has no implicit default — the emit site must
        // bind it explicitly. Even on optional fields the binding is
        // required (the meaningful absence of a ref is "no record was
        // emitted as the parent"; you can't infer it).
        let src = r#"
            recipe "missing"
            engine http
            type Product { id: String }
            type Variant {
                product: Ref<Product>
                id:      String
            }
            step list { method "GET" url "https://x.test" }
            for $p in $list[*] {
                emit Product { id ← $p.id } as $prod
                emit Variant { id ← $p.id }
            }
        "#;
        let r = parse(src).unwrap();
        let cat = TypeCatalog::from_recipe(&r);
        let rep = validate(&r, &cat);
        assert!(
            rep.errors()
                .any(|i| i.code == ValidationCode::MissingRefAssignment),
            "expected MissingRefAssignment; got {:?}",
            rep.issues,
        );
    }

    #[test]
    fn ref_type_mismatch_flagged() {
        let src = r#"
            recipe "mismatch"
            engine http
            type Product { id: String }
            type Category { id: String }
            type Variant {
                product: Ref<Product>
                id:      String
            }
            step list { method "GET" url "https://x.test" }
            for $p in $list[*] {
                emit Category { id ← $p.id } as $cat
                emit Variant { product ← $cat, id ← $p.id }
            }
        "#;
        let r = parse(src).unwrap();
        let cat = TypeCatalog::from_recipe(&r);
        let rep = validate(&r, &cat);
        assert!(
            rep.errors()
                .any(|i| i.code == ValidationCode::RefTypeMismatch),
            "expected RefTypeMismatch; got {:?}",
            rep.issues,
        );
    }

    #[test]
    fn ref_field_bound_to_literal_flagged() {
        let src = r#"
            recipe "lit"
            engine http
            type Product { id: String }
            type Variant {
                product: Ref<Product>
                id:      String
            }
            step list { method "GET" url "https://x.test" }
            for $p in $list[*] {
                emit Variant { product ← "rec-0", id ← $p.id }
            }
        "#;
        let r = parse(src).unwrap();
        let cat = TypeCatalog::from_recipe(&r);
        let rep = validate(&r, &cat);
        assert!(
            rep.errors()
                .any(|i| i.code == ValidationCode::RefTypeMismatch),
            "expected RefTypeMismatch; got {:?}",
            rep.issues,
        );
    }

    #[test]
    fn duplicate_as_binding_flagged() {
        let src = r#"
            recipe "dup"
            engine http
            type Product { id: String }
            step list { method "GET" url "https://x.test" }
            for $p in $list[*] {
                emit Product { id ← $p.id } as $prod
                emit Product { id ← $p.id } as $prod
            }
        "#;
        let r = parse(src).unwrap();
        let cat = TypeCatalog::from_recipe(&r);
        let rep = validate(&r, &cat);
        assert!(
            rep.errors()
                .any(|i| i.code == ValidationCode::DuplicateBinding),
            "expected DuplicateBinding; got {:?}",
            rep.issues,
        );
    }

    #[test]
    fn valid_typed_ref_recipe_has_no_errors() {
        // End-to-end: nested for-loops with `emit … as $v` and refs
        // pointing back to outer-scope emits — should validate cleanly.
        let src = r#"
            recipe "ok"
            engine http
            type Product { id: String }
            type Variant {
                product: Ref<Product>
                id:      String
            }
            type PriceObservation {
                product: Ref<Product>
                variant: Ref<Variant>
                price:   Double?
            }
            step list { method "GET" url "https://x.test" }
            for $p in $list[*] {
                emit Product { id ← $p.id } as $prod
                for $v in $p.variants[*] {
                    emit Variant { product ← $prod, id ← $v.id } as $var
                    emit PriceObservation {
                        product ← $prod
                        variant ← $var
                        price   ← $v.price
                    }
                }
            }
        "#;
        let r = parse(src).unwrap();
        let cat = TypeCatalog::from_recipe(&r);
        let rep = validate(&r, &cat);
        assert!(!rep.has_errors(), "unexpected errors: {:?}", rep.issues);
    }

    #[test]
    fn as_binding_shadowing_for_variable_is_flagged() {
        // `emit … as $prod` inside `for $prod in …` silently rebinds the
        // loop variable to a Ref mid-iteration: `$prod.id` works in the
        // emit's own bindings (because `$prod` was on the frame before
        // the emit pushed its as-binding), but any subsequent reference
        // in the loop body sees the Ref instead of the iteration item.
        // The validator must reject this — the symmetric for-side check
        // already exists for the reverse direction (for-loop var shadowing
        // an enclosing `as`).
        let src = r#"
            recipe "shadow"
            engine http
            type Product { id: String }
            step list { method "GET" url "https://x.test" }
            for $prod in $list[*] {
                emit Product { id ← $prod.id } as $prod
            }
        "#;
        let r = parse(src).unwrap();
        let cat = TypeCatalog::from_recipe(&r);
        let rep = validate(&r, &cat);
        assert!(
            rep.errors()
                .any(|i| i.code == ValidationCode::DuplicateBinding),
            "expected DuplicateBinding when `as $prod` shadows `for $prod`; got {:?}",
            rep.issues,
        );
    }

    #[test]
    fn as_binding_does_not_leak_out_of_for_loop() {
        // `$prod` is introduced inside the for-loop body; a sibling emit
        // outside the loop must NOT see it.
        let src = r#"
            recipe "scope"
            engine http
            type Product { id: String }
            type Wrap {
                product: Ref<Product>
                id:      String
            }
            step list { method "GET" url "https://x.test" }
            for $p in $list[*] {
                emit Product { id ← $p.id } as $prod
            }
            emit Wrap { product ← $prod, id ← "x" }
        "#;
        let r = parse(src).unwrap();
        let cat = TypeCatalog::from_recipe(&r);
        let rep = validate(&r, &cat);
        assert!(
            rep.errors()
                .any(|i| i.code == ValidationCode::RefTypeMismatch),
            "expected RefTypeMismatch when $prod leaks out of the loop; got {:?}",
            rep.issues,
        );
    }

    // ---- user-defined functions --------------------------------------

    fn fn_recipe(extra: &str) -> Recipe {
        let src = format!(
            r#"
                recipe "ok"
                engine http
                {extra}
                type Item {{ id: String }}
                step list {{ method "GET" url "https://x.test" }}
                for $x in $list[*] {{
                    emit Item {{ id ← $x.id }}
                }}
            "#
        );
        parse(&src).expect("parse")
    }

    #[test]
    fn valid_user_fn_validates() {
        let r = fn_recipe("fn shout($x) { $x | upper }");
        let cat = TypeCatalog::from_recipe(&r);
        let rep = validate(&r, &cat);
        assert!(!rep.has_errors(), "unexpected errors: {:?}", rep.issues);
    }

    #[test]
    fn unknown_fn_call_flagged() {
        let src = r#"
            recipe "bad"
            engine http
            type T { id: String }
            step list { method "GET" url "https://x.test" }
            for $x in $list[*] {
                emit T { id ← $x.id | mysteryFn }
            }
        "#;
        let r = parse(src).unwrap();
        let cat = TypeCatalog::from_recipe(&r);
        let rep = validate(&r, &cat);
        assert!(
            rep.errors()
                .any(|i| i.code == ValidationCode::UnknownTransform),
            "expected UnknownTransform; got {:?}",
            rep.issues,
        );
    }

    #[test]
    fn duplicate_fn_name_flagged() {
        let r = fn_recipe("fn dup($x) { $x }\nfn dup($x) { $x }");
        let cat = TypeCatalog::from_recipe(&r);
        let rep = validate(&r, &cat);
        assert!(
            rep.errors().any(|i| i.code == ValidationCode::DuplicateFn),
            "expected DuplicateFn; got {:?}",
            rep.issues,
        );
    }

    #[test]
    fn duplicate_param_name_flagged() {
        let r = fn_recipe("fn dupParams($x, $x) { $x }");
        let cat = TypeCatalog::from_recipe(&r);
        let rep = validate(&r, &cat);
        assert!(
            rep.errors()
                .any(|i| i.code == ValidationCode::DuplicateParam),
            "expected DuplicateParam; got {:?}",
            rep.issues,
        );
    }

    #[test]
    fn reserved_param_name_flagged() {
        // `$page` is engine-injected (HTTP step pagination); it must
        // not be reusable as a fn parameter. `$input` / `$secret` are
        // already excluded at the lexer level (distinct token kinds).
        let r = fn_recipe("fn nope($page) { $page }");
        let cat = TypeCatalog::from_recipe(&r);
        let rep = validate(&r, &cat);
        assert!(
            rep.errors()
                .any(|i| i.code == ValidationCode::ReservedParam),
            "expected ReservedParam; got {:?}",
            rep.issues,
        );
    }

    #[test]
    fn wrong_arity_call_flagged_via_pipe() {
        // `fn two($a, $b) { $a }` expects 2 args. Calling `$x |> two`
        // passes only the head — 1 of 2 → WrongArity.
        let src = r#"
            recipe "bad"
            engine http
            fn two($a, $b) { $a }
            type T { id: String }
            step list { method "GET" url "https://x.test" }
            for $x in $list[*] {
                emit T { id ← $x.id | two }
            }
        "#;
        let r = parse(src).unwrap();
        let cat = TypeCatalog::from_recipe(&r);
        let rep = validate(&r, &cat);
        assert!(
            rep.errors()
                .any(|i| i.code == ValidationCode::WrongArity && i.message.contains("two")),
            "expected WrongArity mentioning the fn name; got {:?}",
            rep.issues,
        );
    }

    #[test]
    fn wrong_arity_call_flagged_via_direct_call() {
        let src = r#"
            recipe "bad"
            engine http
            fn two($a, $b) { $a }
            type T { id: String }
            step list { method "GET" url "https://x.test" }
            for $x in $list[*] {
                emit T { id ← two($x.id) }
            }
        "#;
        let r = parse(src).unwrap();
        let cat = TypeCatalog::from_recipe(&r);
        let rep = validate(&r, &cat);
        assert!(
            rep.errors().any(|i| i.code == ValidationCode::WrongArity),
            "expected WrongArity; got {:?}",
            rep.issues,
        );
    }

    #[test]
    fn user_fn_can_call_built_in_transform() {
        let r = fn_recipe("fn shouty($x) { $x | upper }");
        let cat = TypeCatalog::from_recipe(&r);
        let rep = validate(&r, &cat);
        assert!(!rep.has_errors(), "unexpected errors: {:?}", rep.issues);
    }

    #[test]
    fn user_fn_can_call_other_user_fn_declared_later() {
        // Forward reference: `a` calls `b` declared below it.
        let r = fn_recipe("fn a($x) { $x | b }\nfn b($y) { $y | upper }");
        let cat = TypeCatalog::from_recipe(&r);
        let rep = validate(&r, &cat);
        assert!(!rep.has_errors(), "unexpected errors: {:?}", rep.issues);
    }

    #[test]
    fn direct_recursion_emits_warning() {
        let r = fn_recipe("fn loopy($x) { $x | loopy }");
        let cat = TypeCatalog::from_recipe(&r);
        let rep = validate(&r, &cat);
        assert!(
            !rep.has_errors(),
            "recursion must compile (warning only); got errors: {:?}",
            rep.issues,
        );
        assert!(
            rep.issues
                .iter()
                .any(|i| i.code == ValidationCode::RecursiveFunction
                    && i.severity == Severity::Warning),
            "expected RecursiveFunction warning; got {:?}",
            rep.issues,
        );
    }

    #[test]
    fn user_fn_shadowing_built_in_emits_warning() {
        // `lower` exists as a built-in; redefining it warns but doesn't error.
        let r = fn_recipe("fn lower($x) { $x }");
        let cat = TypeCatalog::from_recipe(&r);
        let rep = validate(&r, &cat);
        assert!(
            !rep.has_errors(),
            "shadowing must not error; got: {:?}",
            rep.issues,
        );
        assert!(
            rep.issues.iter().any(
                |i| i.code == ValidationCode::ShadowsBuiltin && i.severity == Severity::Warning
            ),
            "expected ShadowsBuiltin warning; got {:?}",
            rep.issues,
        );
    }

    #[test]
    fn for_loop_var_not_visible_in_fn_body() {
        // `$item` is bound only inside the for-loop. A `fn` defined
        // anywhere can't see it.
        let src = r#"
            recipe "scoped"
            engine http
            fn leaky($x) { $item }
            type T { id: String }
            step list { method "GET" url "https://x.test" }
            for $item in $list[*] {
                emit T { id ← $item.id | leaky }
            }
        "#;
        let r = parse(src).unwrap();
        let cat = TypeCatalog::from_recipe(&r);
        let rep = validate(&r, &cat);
        assert!(
            rep.errors()
                .any(|i| i.code == ValidationCode::UnknownVariable && i.message.contains("item")),
            "expected UnknownVariable for $item inside fn body; got {:?}",
            rep.issues,
        );
    }

    #[test]
    fn secret_and_input_visible_in_fn_body() {
        // `$input.X` and `$secret.X` are reachable through their path
        // heads, not `known_vars`; the closed fn scope keeps them.
        let src = r#"
            recipe "ok"
            engine http
            secret token
            input mode: String
            fn tag($x) { "{$secret.token}:{$input.mode}:{$x}" }
            type T { id: String }
            step list { method "GET" url "https://x.test/{$secret.token}" }
            for $i in $list[*] {
                emit T { id ← $i.id | tag }
            }
        "#;
        let r = parse(src).unwrap();
        let cat = TypeCatalog::from_recipe(&r);
        let rep = validate(&r, &cat);
        assert!(!rep.has_errors(), "got errors: {:?}", rep.issues);
    }

    #[test]
    fn as_binding_not_visible_in_fn_body() {
        // `$prod` is introduced via `emit … as $prod` at a call site.
        // The fn body must not see it — functions are closed units.
        let src = r#"
            recipe "scoped"
            engine http
            fn leaky($x) { $prod }
            type Product { id: String }
            step list { method "GET" url "https://x.test" }
            for $p in $list[*] {
                emit Product { id ← $p.id } as $prod
                emit Product { id ← $p.id | leaky }
            }
        "#;
        let r = parse(src).unwrap();
        let cat = TypeCatalog::from_recipe(&r);
        let rep = validate(&r, &cat);
        assert!(
            rep.errors()
                .any(|i| i.code == ValidationCode::UnknownVariable && i.message.contains("prod")),
            "expected UnknownVariable for $prod inside fn body; got {:?}",
            rep.issues,
        );
    }
}
