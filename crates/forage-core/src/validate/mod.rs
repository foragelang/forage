//! Semantic checker over the AST. Produces `Vec<ValidationIssue>` with
//! severities. Validation is best-effort — even if some checks fail,
//! others still run, so the user sees the full picture.
//!
//! Public entry: `validate(file, catalog) -> ValidationReport`. The
//! catalog folds in workspace-shared declarations plus the file's local
//! declarations; files outside a workspace pass
//! `TypeCatalog::from_file(&file)` for lonely-file mode.

use serde::{Deserialize, Serialize};

use crate::ast::*;
use crate::workspace::TypeCatalog;

/// Top-level entry point. `catalog` is the merged type namespace for
/// this file — see `Workspace::catalog`. Lonely-file mode (no surrounding
/// `forage.toml`) passes `TypeCatalog::from_file(file)`.
pub fn validate(file: &ForageFile, catalog: &TypeCatalog) -> ValidationReport {
    let mut v = Validator::new(file, catalog);
    v.run();
    ValidationReport { issues: v.issues }
}

/// One file's contribution to a workspace cross-file validation pass.
/// `path` is the file's filesystem location (used in the diagnostic
/// message of any other file that collides on the same `share`d name);
/// `file` is the parsed AST.
#[derive(Debug, Clone, Copy)]
pub struct WorkspaceFileRef<'a> {
    pub path: &'a std::path::Path,
    pub file: &'a ForageFile,
}

/// Walk every file in the workspace and emit cross-file collision
/// diagnostics:
///
/// - `DuplicateSharedDeclaration` whenever two files declare a `share`d
///   type/enum/fn with the same name. File-local (non-`share`d)
///   declarations never participate.
/// - `DuplicateRecipeName` whenever two files declare a recipe with the
///   same header name. The recipe-name namespace is flat across the
///   workspace; `Workspace::recipe_by_name` resolves to the first match
///   in path order, but every duplicate file surfaces a diagnostic so
///   the user can find and resolve the collision.
///
/// Both checks are symmetric — every colliding file surfaces its own
/// diagnostic. Returns a map keyed by the file path that owns each
/// issue. Callers (the LSP docstore, Studio's per-file save) consume
/// only the slice matching the file they're publishing diagnostics for.
pub fn validate_workspace_shared(
    files: &[WorkspaceFileRef<'_>],
) -> std::collections::HashMap<std::path::PathBuf, Vec<ValidationIssue>> {
    use std::collections::HashMap;
    use std::path::PathBuf;

    // Kind discriminator: types, enums, and fns live in separate
    // namespaces — a `share type Foo` and a `share enum Foo` don't
    // collide.
    let mut types: HashMap<&str, Vec<(PathBuf, Span)>> = HashMap::new();
    let mut enums: HashMap<&str, Vec<(PathBuf, Span)>> = HashMap::new();
    let mut fns: HashMap<&str, Vec<(PathBuf, Span)>> = HashMap::new();
    let mut recipes: HashMap<&str, Vec<(PathBuf, Span)>> = HashMap::new();

    for entry in files {
        for t in &entry.file.types {
            if t.shared {
                types
                    .entry(t.name.as_str())
                    .or_default()
                    .push((entry.path.to_path_buf(), t.span.clone()));
            }
        }
        for e in &entry.file.enums {
            if e.shared {
                enums
                    .entry(e.name.as_str())
                    .or_default()
                    .push((entry.path.to_path_buf(), e.span.clone()));
            }
        }
        for f in &entry.file.functions {
            if f.shared {
                fns.entry(f.name.as_str())
                    .or_default()
                    .push((entry.path.to_path_buf(), f.span.clone()));
            }
        }
        // Only the first header per file participates in the cross-file
        // check; same-file duplicates are the `DuplicateRecipeHeader`
        // rule's responsibility, not this pass.
        if let Some(header) = entry.file.recipe_header() {
            recipes
                .entry(header.name.as_str())
                .or_default()
                .push((entry.path.to_path_buf(), header.span.clone()));
        }
    }

    let mut out: HashMap<PathBuf, Vec<ValidationIssue>> = HashMap::new();
    emit_collisions(
        ValidationCode::DuplicateSharedDeclaration,
        |name, others| format!("share type '{name}' is also declared in: {others}"),
        types,
        &mut out,
    );
    emit_collisions(
        ValidationCode::DuplicateSharedDeclaration,
        |name, others| format!("share enum '{name}' is also declared in: {others}"),
        enums,
        &mut out,
    );
    emit_collisions(
        ValidationCode::DuplicateSharedDeclaration,
        |name, others| format!("share fn '{name}' is also declared in: {others}"),
        fns,
        &mut out,
    );
    emit_collisions(
        ValidationCode::DuplicateRecipeName,
        |name, others| format!("recipe '{name}' is also declared in: {others}"),
        recipes,
        &mut out,
    );
    out
}

fn emit_collisions(
    code: ValidationCode,
    message: impl Fn(&str, &str) -> String,
    sites_by_name: std::collections::HashMap<&str, Vec<(std::path::PathBuf, Span)>>,
    out: &mut std::collections::HashMap<std::path::PathBuf, Vec<ValidationIssue>>,
) {
    for (name, sites) in sites_by_name {
        if sites.len() <= 1 {
            continue;
        }
        for (path, span) in &sites {
            let others: Vec<String> = sites
                .iter()
                .filter(|(p, _)| p != path)
                .map(|(p, _)| p.display().to_string())
                .collect();
            out.entry(path.clone()).or_default().push(ValidationIssue {
                severity: Severity::Error,
                code,
                message: message(name, &others.join(", ")),
                span: span.clone(),
            });
        }
    }
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
    /// A struct literal declares the same field name twice. The
    /// runtime would silently keep one and drop the other; surfacing
    /// the duplicate at validate time forces the author to pick.
    DuplicateStructField,
    /// A `let $name = expr` binding inside a fn body shares its name
    /// with one of the fn's parameters. The body would silently shadow
    /// the parameter — almost always a typo.
    LetShadowsParam,
    /// Two `let` bindings in the same fn body use the same name. The
    /// language is single-assignment; rebinding is a recipe-author
    /// error, not a feature.
    DuplicateLetBinding,
    /// A file declares two or more `recipe "<name>"` headers. The
    /// grammar accepts a flat sequence of top-level forms; the
    /// semantic constraint "at most one header per file" lives here.
    DuplicateRecipeHeader,
    /// A header-less file declares recipe-context forms (auth /
    /// browser / expect / statements) — these only make sense
    /// alongside a `recipe` header.
    RecipeContextWithoutHeader,
    /// Two `share`d declarations across the workspace share a name.
    /// Anchored on the file being validated; the cross-file pass that
    /// detected the conflict is the one that emits this issue.
    DuplicateSharedDeclaration,
    /// Two recipes across the workspace declare the same header name.
    /// The recipe-name namespace is flat workspace-wide;
    /// `Workspace::recipe_by_name` resolves to the first match in path
    /// order, but each colliding file gets its own diagnostic.
    DuplicateRecipeName,
    /// `emit X { … }` whose `X` is not listed in the recipe's `output`
    /// declaration. Fires only when an `output` clause is present —
    /// recipes that haven't been migrated to a typed output yet skip
    /// the check entirely.
    MissingFromOutput,
    /// `output` clause was declared with no types listed. Almost
    /// always a typo (`output` followed by a non-TypeName like the
    /// next top-level keyword); the parser keeps the empty clause and
    /// the validator surfaces it here.
    EmptyOutput,
    /// `output` declared in a header-less file. The output signature
    /// is recipe-local; a declarations-only file has nothing to sign.
    OutputWithoutHeader,
    /// `output T` is declared but no `emit T` exists anywhere in the
    /// recipe body. Warning, not error — a recipe that *could* emit
    /// `T` (conditionally, based on inputs) is legitimate, but
    /// most of the time this is a stale signature.
    UnusedInOutput,
}

/// Static list of built-in transforms — mirrors `eval::transforms::build_default`.
/// Keeping a separate list here so the validator doesn't need a registry
/// at construction time. If a recipe references a transform not in here,
/// it's flagged as Unknown.
pub const BUILTIN_TRANSFORMS: &[&str] = &[
    // --- string ---
    "toString",
    "lower",
    "upper",
    "trim",
    "capitalize",
    "titleCase",
    "lowercase",
    "uppercase",
    "replace",
    "split",
    // --- regex ---
    "match",
    "matches",
    "replaceAll",
    // --- parsing scalars ---
    "parseInt",
    "parseFloat",
    "parseBool",
    // --- list / object ---
    "length",
    "dedup",
    "first",
    "coalesce",
    "default",
    // --- field access (dynamic) ---
    "getField",
    // --- HTML / JSON parsing ---
    "parseHtml",
    "parseJson",
    "select",
    "text",
    "attr",
    "html",
    "innerHtml",
];

struct Validator<'a> {
    file: &'a ForageFile,
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
    fn new(file: &'a ForageFile, catalog: &'a TypeCatalog) -> Self {
        let mut known_vars = std::collections::HashSet::new();
        collect_bindings(&file.body, &mut known_vars);
        if let Some(b) = &file.browser {
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
        if let Some(AuthStrategy::HtmlPrime { captured_vars, .. }) = &file.auth {
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
        for f in &file.functions {
            user_fn_arity
                .entry(f.name.clone())
                .or_insert(f.params.len());
        }
        Self {
            file,
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
        self.check_recipe_headers();
        self.check_recipe_context();
        self.check_duplicates();
        self.check_engine_consistency();
        self.check_user_fns();
        self.check_references();
        self.check_emit_records();
        self.check_output_decl();
    }

    /// Walk every `fn` declaration: duplicate detection, parameter rules,
    /// shadow-of-builtin warning, body validation in a fresh scope, and
    /// direct-recursion warning.
    fn check_user_fns(&mut self) {
        let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for f in &self.file.functions.clone() {
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
                let params: std::collections::HashSet<&str> =
                    f.params.iter().map(|s| s.as_str()).collect();
                for p in &f.params {
                    body_vars.insert(p.clone());
                }
                v.known_vars = body_vars;
                let saved_fn = v.enclosing_fn.replace(f.name.clone());

                // Let-bindings: each adds to scope after its value is
                // validated. Catches `LetShadowsParam` and
                // `DuplicateLetBinding`; the runtime never sees a
                // shadowed name because the validator refuses the
                // recipe before eval boots.
                let mut let_names: std::collections::HashSet<String> =
                    std::collections::HashSet::new();
                for b in &f.body.bindings {
                    v.check_extraction(&b.value);
                    if params.contains(b.name.as_str()) {
                        v.err_here(
                            ValidationCode::LetShadowsParam,
                            format!(
                                "let binding '${name}' shadows the function parameter '${name}'",
                                name = b.name,
                            ),
                        );
                    }
                    if !let_names.insert(b.name.clone()) {
                        v.err_here(
                            ValidationCode::DuplicateLetBinding,
                            format!(
                                "let binding '${name}' is declared more than once in this fn body",
                                name = b.name,
                            ),
                        );
                    }
                    v.known_vars.insert(b.name.clone());
                }
                v.check_extraction(&f.body.result);
                v.enclosing_fn = saved_fn;
                v.known_vars = saved_vars;
                v.ref_bindings = saved_refs;
            });
        }
    }

    // --- duplicates --------------------------------------------------------

    fn check_duplicates(&mut self) {
        let mut seen_types = std::collections::HashSet::new();
        for t in &self.file.types {
            if !seen_types.insert(&t.name) {
                self.err(
                    t.span.clone(),
                    ValidationCode::DuplicateType,
                    format!("type '{}' declared more than once", t.name),
                );
            }
        }
        let mut seen_enums = std::collections::HashSet::new();
        for e in &self.file.enums {
            if !seen_enums.insert(&e.name) {
                self.err(
                    e.span.clone(),
                    ValidationCode::DuplicateEnum,
                    format!("enum '{}' declared more than once", e.name),
                );
            }
        }
        let mut seen_inputs = std::collections::HashSet::new();
        for i in &self.file.inputs {
            if !seen_inputs.insert(&i.name) {
                self.err(
                    i.span.clone(),
                    ValidationCode::DuplicateInput,
                    format!("input '{}' declared more than once", i.name),
                );
            }
        }
        let mut seen_secrets = std::collections::HashSet::new();
        for s in &self.file.secrets {
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
        // Engine consistency only applies to recipe-bearing files. The
        // `RecipeContextWithoutHeader` rule has already flagged any
        // recipe-context forms in a header-less file; nothing else to
        // check here.
        let Some(engine_kind) = self.file.engine_kind() else {
            return;
        };
        match engine_kind {
            EngineKind::Http => {
                if self.file.browser.is_some() {
                    self.err_recipe(
                        ValidationCode::UnexpectedBrowserConfig,
                        "HTTP-engine recipe must not declare a `browser { … }` block",
                    );
                }
            }
            EngineKind::Browser => {
                if self.file.browser.is_none() {
                    self.err_recipe(
                        ValidationCode::MissingBrowserConfig,
                        "browser-engine recipe must declare a `browser { … }` block",
                    );
                }
                if matches!(self.file.auth, Some(AuthStrategy::Session(_))) {
                    self.warn_recipe(
                        ValidationCode::AuthOnBrowserEngine,
                        "auth.session.* on a browser-engine recipe — credentials are best handled inside the browser flow",
                    );
                }
            }
        }
        if let Some(AuthStrategy::HtmlPrime { step_name, .. }) = &self.file.auth {
            let referenced = self
                .file
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

    /// `DuplicateRecipeHeader` — a file with two or more `recipe "<name>"`
    /// openers. The parser is permissive; the constraint lives here.
    /// Anchors each duplicate diagnostic at its own span so editors can
    /// jump to the right line.
    fn check_recipe_headers(&mut self) {
        // The first header is canonical; every additional one is a
        // duplicate. Iterate by index so we anchor the diagnostic on the
        // duplicate's own span, not the canonical one.
        for header in self.file.recipe_headers.iter().skip(1) {
            self.err(
                header.span.clone(),
                ValidationCode::DuplicateRecipeHeader,
                format!(
                    "file declares more than one recipe header; the second '{}' is a duplicate",
                    header.name,
                ),
            );
        }
    }

    /// `RecipeContextWithoutHeader` — recipe-context forms (auth /
    /// browser / expect / statements) only make sense alongside a
    /// `recipe` header. Anchors at the first offending form so the
    /// user lands on something they can act on.
    fn check_recipe_context(&mut self) {
        if self.file.recipe_header().is_some() {
            return;
        }
        if self.file.auth.is_some() {
            self.err_recipe(
                ValidationCode::RecipeContextWithoutHeader,
                "auth block requires a `recipe \"<name>\" engine <kind>` header",
            );
        }
        if self.file.browser.is_some() {
            self.err_recipe(
                ValidationCode::RecipeContextWithoutHeader,
                "browser block requires a `recipe \"<name>\" engine <kind>` header",
            );
        }
        for e in &self.file.expectations.clone() {
            self.err(
                e.span.clone(),
                ValidationCode::RecipeContextWithoutHeader,
                "expect block requires a `recipe \"<name>\" engine <kind>` header",
            );
        }
        for s in &self.file.body.clone() {
            self.err(
                s.span().clone(),
                ValidationCode::RecipeContextWithoutHeader,
                "statements (step / for / emit) require a `recipe \"<name>\" engine <kind>` header",
            );
        }
        if !self.file.secrets.is_empty() {
            self.err_recipe(
                ValidationCode::RecipeContextWithoutHeader,
                "`secret` declarations require a `recipe \"<name>\" engine <kind>` header",
            );
        }
        if !self.file.inputs.is_empty() {
            self.err_recipe(
                ValidationCode::RecipeContextWithoutHeader,
                "`input` declarations require a `recipe \"<name>\" engine <kind>` header",
            );
        }
        if let Some(out) = &self.file.output {
            self.err(
                out.span.clone(),
                ValidationCode::OutputWithoutHeader,
                "`output` declarations require a `recipe \"<name>\" engine <kind>` header",
            );
        }
    }

    // --- name resolution ---------------------------------------------------

    fn check_references(&mut self) {
        for s in self.file.body.clone() {
            self.check_statement(&s);
        }
        if let Some(b) = &self.file.browser {
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
            // `output` cross-check. Skip when the recipe has no
            // declared output (legacy un-migrated recipes) or when the
            // declared output is empty — `EmptyOutput` already covers
            // that case and adding `MissingFromOutput` per emit would
            // bury the real diagnostic.
            if let Some(out) = &v.file.output {
                if !out.types.is_empty() && !out.types.iter().any(|t| t == &em.type_name) {
                    v.err_here(
                        ValidationCode::MissingFromOutput,
                        format!(
                            "emit {} is not listed in the recipe's `output` declaration",
                            em.type_name,
                        ),
                    );
                }
            }
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
            ExtractionExpr::BinaryOp { lhs, rhs, .. } => {
                self.check_extraction(lhs);
                self.check_extraction(rhs);
            }
            ExtractionExpr::Unary { operand, .. } => {
                self.check_extraction(operand);
            }
            ExtractionExpr::StructLiteral { fields } => {
                let mut seen: std::collections::HashSet<&str> =
                    std::collections::HashSet::new();
                for f in fields {
                    if !seen.insert(f.field_name.as_str()) {
                        self.err_here(
                            ValidationCode::DuplicateStructField,
                            format!(
                                "struct literal declares field '{}' more than once",
                                f.field_name,
                            ),
                        );
                    }
                    self.check_extraction(&f.expr);
                }
            }
            ExtractionExpr::Index { base, index } => {
                self.check_extraction(base);
                self.check_extraction(index);
            }
            ExtractionExpr::RegexLiteral(_) => {
                // Regex literals are validated at parse time (pattern
                // compiles, flags recognized). Nothing else to check.
            }
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
                    // `_` is the catch-all sentinel — not a real variant.
                    if l == "_" {
                        continue;
                    }
                    if !known.contains(l.as_str()) {
                        self.err_here(
                            ValidationCode::UnknownEnumVariant,
                            format!("case label '{l}' is not a variant of enum {enum_name}"),
                        );
                    }
                }
                let used_set: std::collections::HashSet<&str> =
                    used.iter().map(|s| s.as_str()).collect();
                // `_` is a catch-all default arm; its presence makes
                // the case-of exhaustive regardless of which variants
                // got explicit arms.
                let has_default = used_set.contains("_");
                if !has_default {
                    for v in &en.variants {
                        if !used_set.contains(v.as_str()) {
                            self.warn_here(
                                ValidationCode::UnknownEnumVariant,
                                format!(
                                    "case-of over {enum_name} doesn't cover variant '{v}'",
                                ),
                            );
                        }
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
                    if let Some(inp) = self.file.input(field) {
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
                if !self.file.secrets.iter().any(|s| s == name) {
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
                    if self.file.input(field).is_none() {
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

    /// Validates the `output` clause itself: empty list, unknown type
    /// names, and declared-but-unemitted types. Skipped entirely on
    /// header-less files (`OutputWithoutHeader` already fired) and on
    /// recipes that haven't declared an output yet.
    fn check_output_decl(&mut self) {
        let Some(out) = self.file.output.clone() else {
            return;
        };
        if self.file.recipe_header().is_none() {
            // Header-less files already got `OutputWithoutHeader`; no
            // point also surfacing unknown-type / empty errors that
            // duplicate the diagnostic.
            return;
        }
        if out.types.is_empty() {
            self.err(
                out.span.clone(),
                ValidationCode::EmptyOutput,
                "`output` declared with no types — list at least one (`output T` or `output T1 | T2`)",
            );
            return;
        }
        for name in &out.types {
            if self.catalog.ty(name).is_none() {
                self.err(
                    out.span.clone(),
                    ValidationCode::UnknownType,
                    format!("`output {name}` references an unknown type"),
                );
            }
        }
        let mut emitted: std::collections::HashSet<String> = std::collections::HashSet::new();
        collect_emitted_types(&self.file.body, &mut emitted);
        if let Some(b) = &self.file.browser {
            for cap in &b.captures {
                collect_emitted_types(&cap.body, &mut emitted);
            }
            if let Some(doc) = &b.document_capture {
                collect_emitted_types(&doc.body, &mut emitted);
            }
        }
        for name in &out.types {
            if !emitted.contains(name) {
                self.warn(
                    out.span.clone(),
                    ValidationCode::UnusedInOutput,
                    format!(
                        "`output {name}` is declared but no `emit {name}` exists in the recipe body",
                    ),
                );
            }
        }
    }

    fn check_emit_records(&mut self) {
        // Verify each declared type's field types either resolve to a
        // primitive, an Array of one, an EnumRef to a declared enum, or a
        // Record reference to a declared type.
        for ty in &self.file.types.clone() {
            self.with_span(ty.span.clone(), |v| {
                for f in &ty.fields {
                    v.check_field_type(&f.ty, &format!("type {}.{}", ty.name, f.name));
                }
            });
        }
        for inp in &self.file.inputs.clone() {
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
/// Walk a body recursively (steps, for-loops, captures) and collect
/// every type name reached by an `emit T { … }`. Used by
/// `check_output_decl` to compute the `output` set's coverage.
fn collect_emitted_types(body: &[Statement], out: &mut std::collections::HashSet<String>) {
    for s in body {
        match s {
            Statement::Emit(em) => {
                out.insert(em.type_name.clone());
            }
            Statement::ForLoop { body, .. } => {
                collect_emitted_types(body, out);
            }
            Statement::Step(_) => {}
        }
    }
}

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
        let cat = TypeCatalog::from_file(&r);
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
        let cat = TypeCatalog::from_file(&r);
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
        let cat = TypeCatalog::from_file(&r);
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
        let cat = TypeCatalog::from_file(&r);
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
        let cat = TypeCatalog::from_file(&r);
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
        let cat = TypeCatalog::from_file(&r);
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
        let cat = TypeCatalog::from_file(&r);
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
        let cat = TypeCatalog::from_file(&r);
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
        let cat = TypeCatalog::from_file(&r);
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
        let cat = TypeCatalog::from_file(&r);
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
        let cat = TypeCatalog::from_file(&r);
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
        let cat = TypeCatalog::from_file(&r);
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
        let cat = TypeCatalog::from_file(&r);
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
        let cat = TypeCatalog::from_file(&r);
        let rep = validate(&r, &cat);
        assert!(
            rep.errors()
                .any(|i| i.code == ValidationCode::RefTypeMismatch),
            "expected RefTypeMismatch when $prod leaks out of the loop; got {:?}",
            rep.issues,
        );
    }

    // ---- user-defined functions --------------------------------------

    fn fn_recipe(extra: &str) -> ForageFile {
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
        let cat = TypeCatalog::from_file(&r);
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
        let cat = TypeCatalog::from_file(&r);
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
        let cat = TypeCatalog::from_file(&r);
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
        let cat = TypeCatalog::from_file(&r);
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
        let cat = TypeCatalog::from_file(&r);
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
        let cat = TypeCatalog::from_file(&r);
        let rep = validate(&r, &cat);
        assert!(
            rep.errors()
                .any(|i| i.code == ValidationCode::WrongArity && i.message.contains("two")),
            "expected WrongArity mentioning the fn name; got {:?}",
            rep.issues,
        );
    }

    #[test]
    fn zero_param_fn_called_via_pipe_flagged_as_wrong_arity() {
        // A pipe always carries the head as param 0; a zero-parameter
        // fn has nowhere to bind it. The validator must reject the call
        // before the runtime arity check ever fires.
        let src = r#"
            recipe "bad"
            engine http
            fn answer() { 42 }
            type T { id: Int }
            step list { method "GET" url "https://x.test" }
            for $x in $list[*] {
                emit T { id ← $x.id | answer }
            }
        "#;
        let r = parse(src).unwrap();
        let cat = TypeCatalog::from_file(&r);
        let rep = validate(&r, &cat);
        assert!(
            rep.errors()
                .any(|i| i.code == ValidationCode::WrongArity && i.message.contains("answer")),
            "expected WrongArity mentioning 'answer'; got {:?}",
            rep.issues,
        );
    }

    #[test]
    fn zero_param_fn_called_via_direct_call_is_valid() {
        // `answer()` is the canonical zero-arg form. Validator must
        // accept it — the eval path handles the empty arg list.
        let src = r#"
            recipe "ok"
            engine http
            fn answer() { 42 }
            type T { id: Int }
            emit T { id ← answer() }
        "#;
        let r = parse(src).unwrap();
        let cat = TypeCatalog::from_file(&r);
        let rep = validate(&r, &cat);
        assert!(!rep.has_errors(), "unexpected errors: {:?}", rep.issues);
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
        let cat = TypeCatalog::from_file(&r);
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
        let cat = TypeCatalog::from_file(&r);
        let rep = validate(&r, &cat);
        assert!(!rep.has_errors(), "unexpected errors: {:?}", rep.issues);
    }

    #[test]
    fn user_fn_can_call_other_user_fn_declared_later() {
        // Forward reference: `a` calls `b` declared below it.
        let r = fn_recipe("fn a($x) { $x | b }\nfn b($y) { $y | upper }");
        let cat = TypeCatalog::from_file(&r);
        let rep = validate(&r, &cat);
        assert!(!rep.has_errors(), "unexpected errors: {:?}", rep.issues);
    }

    #[test]
    fn direct_recursion_emits_warning() {
        let r = fn_recipe("fn loopy($x) { $x | loopy }");
        let cat = TypeCatalog::from_file(&r);
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
        let cat = TypeCatalog::from_file(&r);
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
        let cat = TypeCatalog::from_file(&r);
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
        let cat = TypeCatalog::from_file(&r);
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
        let cat = TypeCatalog::from_file(&r);
        let rep = validate(&r, &cat);
        assert!(
            rep.errors()
                .any(|i| i.code == ValidationCode::UnknownVariable && i.message.contains("prod")),
            "expected UnknownVariable for $prod inside fn body; got {:?}",
            rep.issues,
        );
    }

    // ---- file-grammar rules: header / context / shared decls ---------

    #[test]
    fn duplicate_recipe_header_flagged() {
        let src = r#"
            recipe "first"
            engine http

            recipe "second"
            engine http

            type Item { id: String }
        "#;
        let r = parse(src).unwrap();
        let cat = TypeCatalog::from_file(&r);
        let rep = validate(&r, &cat);
        assert!(
            rep.errors()
                .any(|i| i.code == ValidationCode::DuplicateRecipeHeader),
            "expected DuplicateRecipeHeader; got {:?}",
            rep.issues,
        );
    }

    #[test]
    fn statement_without_header_flagged() {
        let src = r#"
            step orphan {
                method "GET"
                url "https://example.com"
            }
        "#;
        let r = parse(src).unwrap();
        let cat = TypeCatalog::from_file(&r);
        let rep = validate(&r, &cat);
        assert!(
            rep.errors()
                .any(|i| i.code == ValidationCode::RecipeContextWithoutHeader),
            "expected RecipeContextWithoutHeader for a stray step; got {:?}",
            rep.issues,
        );
    }

    #[test]
    fn auth_without_header_flagged() {
        let src = r#"
            auth.staticHeader { name: "X-Api-Key", value: "abc" }
        "#;
        let r = parse(src).unwrap();
        let cat = TypeCatalog::from_file(&r);
        let rep = validate(&r, &cat);
        assert!(
            rep.errors()
                .any(|i| i.code == ValidationCode::RecipeContextWithoutHeader
                    && i.message.contains("auth")),
            "expected RecipeContextWithoutHeader for an auth block; got {:?}",
            rep.issues,
        );
    }

    #[test]
    fn browser_without_header_flagged() {
        let src = r#"
            browser {
                initialURL: "https://example.com"
                observe: "example.com"
                paginate browserPaginate.scroll { until: noProgressFor(1) }
            }
        "#;
        let r = parse(src).unwrap();
        let cat = TypeCatalog::from_file(&r);
        let rep = validate(&r, &cat);
        assert!(
            rep.errors()
                .any(|i| i.code == ValidationCode::RecipeContextWithoutHeader
                    && i.message.contains("browser")),
            "expected RecipeContextWithoutHeader for a browser block; got {:?}",
            rep.issues,
        );
    }

    #[test]
    fn expect_without_header_flagged() {
        let src = r#"
            expect { records.where(typeName == "X").count > 0 }
        "#;
        let r = parse(src).unwrap();
        let cat = TypeCatalog::from_file(&r);
        let rep = validate(&r, &cat);
        assert!(
            rep.errors()
                .any(|i| i.code == ValidationCode::RecipeContextWithoutHeader
                    && i.message.contains("expect")),
            "expected RecipeContextWithoutHeader for an expect block; got {:?}",
            rep.issues,
        );
    }

    #[test]
    fn header_less_declarations_file_validates_clean() {
        // A pure declarations file with only `share`d types/enums/fns
        // must pass the validator. No recipe header means none of the
        // recipe-context rules fire.
        let src = r#"
            share type Foo { id: String }
            share enum Mode { A, B }
            share fn double($x) { $x }
        "#;
        let r = parse(src).unwrap();
        let cat = TypeCatalog::from_file(&r);
        let rep = validate(&r, &cat);
        assert!(!rep.has_errors(), "got errors: {:?}", rep.issues);
    }

    #[test]
    fn duplicate_shared_declarations_across_files_flagged() {
        // Two files in a workspace both declare `share type Foo { … }`.
        // The cross-file validator emits `DuplicateSharedDeclaration` on
        // *both* sites so the editor can squiggle both.
        let src_a = r#"
            share type Foo { id: String }
        "#;
        let src_b = r#"
            share type Foo { name: String }
        "#;
        let file_a = parse(src_a).unwrap();
        let file_b = parse(src_b).unwrap();
        let path_a = std::path::PathBuf::from("/ws/a.forage");
        let path_b = std::path::PathBuf::from("/ws/b.forage");
        let by_path = validate_workspace_shared(&[
            WorkspaceFileRef {
                path: &path_a,
                file: &file_a,
            },
            WorkspaceFileRef {
                path: &path_b,
                file: &file_b,
            },
        ]);
        assert!(
            by_path.get(&path_a).is_some_and(|v| v
                .iter()
                .any(|i| i.code == ValidationCode::DuplicateSharedDeclaration)),
            "expected DuplicateSharedDeclaration on file A; got {by_path:?}",
        );
        assert!(
            by_path.get(&path_b).is_some_and(|v| v
                .iter()
                .any(|i| i.code == ValidationCode::DuplicateSharedDeclaration)),
            "expected DuplicateSharedDeclaration on file B; got {by_path:?}",
        );
    }

    #[test]
    fn file_local_decl_does_not_collide_with_shared_decl_elsewhere() {
        // `Foo` is `share`d in file A and file-local in file B. The
        // cross-file pass must not fire `DuplicateSharedDeclaration`
        // because only one is `share`d.
        let src_a = "share type Foo { id: String }\n";
        let src_b = "type Foo { id: String }\n";
        let file_a = parse(src_a).unwrap();
        let file_b = parse(src_b).unwrap();
        let path_a = std::path::PathBuf::from("/ws/a.forage");
        let path_b = std::path::PathBuf::from("/ws/b.forage");
        let by_path = validate_workspace_shared(&[
            WorkspaceFileRef {
                path: &path_a,
                file: &file_a,
            },
            WorkspaceFileRef {
                path: &path_b,
                file: &file_b,
            },
        ]);
        assert!(
            by_path.is_empty(),
            "single share + file-local must not collide; got {by_path:?}",
        );
    }

    #[test]
    fn duplicate_shared_enum_across_files_flagged() {
        let src_a = "share enum Mode { A, B }\n";
        let src_b = "share enum Mode { X, Y }\n";
        let file_a = parse(src_a).unwrap();
        let file_b = parse(src_b).unwrap();
        let path_a = std::path::PathBuf::from("/ws/a.forage");
        let path_b = std::path::PathBuf::from("/ws/b.forage");
        let by_path = validate_workspace_shared(&[
            WorkspaceFileRef {
                path: &path_a,
                file: &file_a,
            },
            WorkspaceFileRef {
                path: &path_b,
                file: &file_b,
            },
        ]);
        assert!(by_path.get(&path_a).is_some_and(|v| v.len() == 1));
        assert!(by_path.get(&path_b).is_some_and(|v| v.len() == 1));
    }

    #[test]
    fn duplicate_shared_fn_across_files_flagged() {
        let src_a = "share fn upper($x) { $x }\n";
        let src_b = "share fn upper($x) { $x }\n";
        let file_a = parse(src_a).unwrap();
        let file_b = parse(src_b).unwrap();
        let path_a = std::path::PathBuf::from("/ws/a.forage");
        let path_b = std::path::PathBuf::from("/ws/b.forage");
        let by_path = validate_workspace_shared(&[
            WorkspaceFileRef {
                path: &path_a,
                file: &file_a,
            },
            WorkspaceFileRef {
                path: &path_b,
                file: &file_b,
            },
        ]);
        assert!(by_path.get(&path_a).is_some_and(|v| v.len() == 1));
        assert!(by_path.get(&path_b).is_some_and(|v| v.len() == 1));
    }

    /// Two files declaring `recipe "dup"` is a cross-file collision —
    /// the recipe namespace is flat across the workspace. Both files
    /// get a `DuplicateRecipeName` diagnostic anchored at their header.
    #[test]
    fn duplicate_recipe_name_across_files_flagged() {
        let src_a = "recipe \"dup\"\nengine http\n";
        let src_b = "recipe \"dup\"\nengine http\n";
        let file_a = parse(src_a).unwrap();
        let file_b = parse(src_b).unwrap();
        let path_a = std::path::PathBuf::from("/ws/a.forage");
        let path_b = std::path::PathBuf::from("/ws/b.forage");
        let by_path = validate_workspace_shared(&[
            WorkspaceFileRef {
                path: &path_a,
                file: &file_a,
            },
            WorkspaceFileRef {
                path: &path_b,
                file: &file_b,
            },
        ]);
        assert!(
            by_path.get(&path_a).is_some_and(|v| v
                .iter()
                .any(|i| i.code == ValidationCode::DuplicateRecipeName)),
            "expected DuplicateRecipeName on file A; got {by_path:?}",
        );
        assert!(
            by_path.get(&path_b).is_some_and(|v| v
                .iter()
                .any(|i| i.code == ValidationCode::DuplicateRecipeName)),
            "expected DuplicateRecipeName on file B; got {by_path:?}",
        );
    }

    /// Distinct recipe names across files don't collide. A header-less
    /// file alongside a recipe file is also fine.
    #[test]
    fn distinct_recipe_names_across_files_do_not_collide() {
        let src_a = "recipe \"alpha\"\nengine http\n";
        let src_b = "recipe \"beta\"\nengine http\n";
        let src_c = "type Shared { id: String }\n";
        let file_a = parse(src_a).unwrap();
        let file_b = parse(src_b).unwrap();
        let file_c = parse(src_c).unwrap();
        let path_a = std::path::PathBuf::from("/ws/a.forage");
        let path_b = std::path::PathBuf::from("/ws/b.forage");
        let path_c = std::path::PathBuf::from("/ws/c.forage");
        let by_path = validate_workspace_shared(&[
            WorkspaceFileRef {
                path: &path_a,
                file: &file_a,
            },
            WorkspaceFileRef {
                path: &path_b,
                file: &file_b,
            },
            WorkspaceFileRef {
                path: &path_c,
                file: &file_c,
            },
        ]);
        assert!(
            by_path.is_empty(),
            "distinct recipe names + header-less file must not collide; got {by_path:?}",
        );
    }

    // ---- output declarations -----------------------------------------

    #[test]
    fn emit_listed_in_output_validates_clean() {
        let src = r#"
            recipe "ok"
            engine http
            output Item
            type Item { id: String }
            step list { method "GET" url "https://x.test" }
            for $i in $list[*] {
                emit Item { id ← $i.id }
            }
        "#;
        let r = parse(src).unwrap();
        let cat = TypeCatalog::from_file(&r);
        let rep = validate(&r, &cat);
        assert!(!rep.has_errors(), "got errors: {:?}", rep.issues);
    }

    #[test]
    fn emit_not_listed_in_output_flagged_as_missing_from_output() {
        let src = r#"
            recipe "bad"
            engine http
            output Product
            type Product { id: String }
            type Variant { id: String }
            step list { method "GET" url "https://x.test" }
            for $p in $list[*] {
                emit Product { id ← $p.id }
                emit Variant { id ← $p.id }
            }
        "#;
        let r = parse(src).unwrap();
        let cat = TypeCatalog::from_file(&r);
        let rep = validate(&r, &cat);
        let issue = rep
            .errors()
            .find(|i| i.code == ValidationCode::MissingFromOutput)
            .expect("MissingFromOutput");
        assert!(
            issue.message.contains("Variant"),
            "expected message to name Variant; got {:?}",
            issue,
        );
    }

    #[test]
    fn multi_type_output_covers_each_listed_emit() {
        let src = r#"
            recipe "multi"
            engine http
            output Product | Variant | PriceObservation
            type Product { id: String }
            type Variant {
                product: Ref<Product>
                id: String
            }
            type PriceObservation {
                product: Ref<Product>
                variant: Ref<Variant>
                price: Double?
            }
            step list { method "GET" url "https://x.test" }
            for $p in $list[*] {
                emit Product { id ← $p.id } as $prod
                emit Variant { product ← $prod, id ← $p.id } as $var
                emit PriceObservation { product ← $prod, variant ← $var, price ← $p.price }
            }
        "#;
        let r = parse(src).unwrap();
        let cat = TypeCatalog::from_file(&r);
        let rep = validate(&r, &cat);
        assert!(!rep.has_errors(), "got errors: {:?}", rep.issues);
    }

    #[test]
    fn empty_output_clause_flagged() {
        // `output` followed by no TypeName parses with an empty list;
        // validator flags it so the author doesn't end up with a silent
        // no-op output signature.
        let src = r#"
            recipe "empty"
            engine http
            output
            type Item { id: String }
            step list { method "GET" url "https://x.test" }
            for $i in $list[*] {
                emit Item { id ← $i.id }
            }
        "#;
        let r = parse(src).unwrap();
        let cat = TypeCatalog::from_file(&r);
        let rep = validate(&r, &cat);
        assert!(
            rep.errors().any(|i| i.code == ValidationCode::EmptyOutput),
            "expected EmptyOutput; got {:?}",
            rep.issues,
        );
    }

    #[test]
    fn output_in_header_less_file_flagged() {
        // `output` belongs to a recipe — a declarations-only file has
        // nothing to sign.
        let src = r#"
            share type Item { id: String }
            output Item
        "#;
        let r = parse(src).unwrap();
        let cat = TypeCatalog::from_file(&r);
        let rep = validate(&r, &cat);
        assert!(
            rep.errors()
                .any(|i| i.code == ValidationCode::OutputWithoutHeader),
            "expected OutputWithoutHeader; got {:?}",
            rep.issues,
        );
    }

    #[test]
    fn output_with_unknown_type_flagged() {
        // An `output T` for a `T` the catalog can't resolve is almost
        // always a typo. Re-uses the existing `UnknownType` code.
        let src = r#"
            recipe "typo"
            engine http
            output Itme
            type Item { id: String }
            step list { method "GET" url "https://x.test" }
            for $i in $list[*] {
                emit Item { id ← $i.id }
            }
        "#;
        let r = parse(src).unwrap();
        let cat = TypeCatalog::from_file(&r);
        let rep = validate(&r, &cat);
        assert!(
            rep.errors()
                .any(|i| i.code == ValidationCode::UnknownType && i.message.contains("Itme")),
            "expected UnknownType for the typo; got {:?}",
            rep.issues,
        );
    }

    #[test]
    fn unused_output_type_emits_warning() {
        // `output T` is declared but no `emit T` exists — warn so the
        // author notices a stale signature without blocking the build.
        let src = r#"
            recipe "stale"
            engine http
            output Item | Stale
            type Item { id: String }
            type Stale { id: String }
            step list { method "GET" url "https://x.test" }
            for $i in $list[*] {
                emit Item { id ← $i.id }
            }
        "#;
        let r = parse(src).unwrap();
        let cat = TypeCatalog::from_file(&r);
        let rep = validate(&r, &cat);
        assert!(
            !rep.has_errors(),
            "UnusedInOutput must not error; got: {:?}",
            rep.issues,
        );
        assert!(
            rep.issues.iter().any(|i| i.code
                == ValidationCode::UnusedInOutput
                && i.severity == Severity::Warning
                && i.message.contains("Stale")),
            "expected UnusedInOutput warning naming 'Stale'; got {:?}",
            rep.issues,
        );
    }

    #[test]
    fn recipe_without_output_decl_skips_missing_from_output_check() {
        // The `output` clause is optional in the AST; when it is absent
        // the validator skips the emit-vs-output check entirely.
        let src = r#"
            recipe "legacy"
            engine http
            type Item { id: String }
            step list { method "GET" url "https://x.test" }
            for $i in $list[*] {
                emit Item { id ← $i.id }
            }
        "#;
        let r = parse(src).unwrap();
        let cat = TypeCatalog::from_file(&r);
        let rep = validate(&r, &cat);
        assert!(!rep.has_errors(), "got errors: {:?}", rep.issues);
    }

    #[test]
    fn missing_from_output_diagnostic_anchors_at_emit_site() {
        let src = "recipe \"anchor\"\nengine http\noutput Product\ntype Product { id: String }\ntype Variant { id: String }\nstep list { method \"GET\" url \"https://x.test\" }\nfor $p in $list[*] {\n    emit Variant { id \u{2190} $p.id }\n}\n";
        let r = parse(src).unwrap();
        let cat = TypeCatalog::from_file(&r);
        let rep = validate(&r, &cat);
        let missing = rep
            .issues
            .iter()
            .find(|i| i.code == ValidationCode::MissingFromOutput)
            .expect("MissingFromOutput");
        assert!(
            src[missing.span.clone()].starts_with("emit Variant"),
            "diagnostic must anchor at the emit; got {:?}",
            &src[missing.span.clone()],
        );
    }
}
