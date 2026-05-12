//! Semantic checker over the AST. Produces `Vec<ValidationIssue>` with
//! severities. Validation is best-effort — even if some checks fail,
//! others still run, so the user sees the full picture.
//!
//! Public entry: `validate(recipe: &Recipe) -> ValidationReport`.

use serde::{Deserialize, Serialize};

use crate::ast::*;

/// Top-level entry point.
pub fn validate(recipe: &Recipe) -> ValidationReport {
    let mut v = Validator::new(recipe);
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
    MissingRequiredField,
    UnknownField,
    UnknownEnumVariant,
    MissingBrowserConfig,
    UnexpectedBrowserConfig,
    AuthOnBrowserEngine,
    MissingAuthStep,
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
    issues: Vec<ValidationIssue>,
    /// Variable bindings in scope at the current walking position. Includes
    /// step names (recipe-body-wide), for-loop variables (nested),
    /// htmlPrime-extracted vars (from auth or step.extract.regex.groups).
    known_vars: std::collections::HashSet<String>,
}

impl<'a> Validator<'a> {
    fn new(recipe: &'a Recipe) -> Self {
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
        Self {
            recipe,
            issues: Vec::new(),
            known_vars,
        }
    }

    fn err(&mut self, code: ValidationCode, message: impl Into<String>) {
        self.issues.push(ValidationIssue {
            severity: Severity::Error,
            code,
            message: message.into(),
        });
    }

    fn warn(&mut self, code: ValidationCode, message: impl Into<String>) {
        self.issues.push(ValidationIssue {
            severity: Severity::Warning,
            code,
            message: message.into(),
        });
    }

    fn run(&mut self) {
        self.check_duplicates();
        self.check_engine_consistency();
        self.check_references();
        self.check_emit_records();
    }

    // --- duplicates --------------------------------------------------------

    fn check_duplicates(&mut self) {
        let mut seen_types = std::collections::HashSet::new();
        for t in &self.recipe.types {
            if !seen_types.insert(&t.name) {
                self.err(
                    ValidationCode::DuplicateType,
                    format!("type '{}' declared more than once", t.name),
                );
            }
        }
        let mut seen_enums = std::collections::HashSet::new();
        for e in &self.recipe.enums {
            if !seen_enums.insert(&e.name) {
                self.err(
                    ValidationCode::DuplicateEnum,
                    format!("enum '{}' declared more than once", e.name),
                );
            }
        }
        let mut seen_inputs = std::collections::HashSet::new();
        for i in &self.recipe.inputs {
            if !seen_inputs.insert(&i.name) {
                self.err(
                    ValidationCode::DuplicateInput,
                    format!("input '{}' declared more than once", i.name),
                );
            }
        }
        let mut seen_secrets = std::collections::HashSet::new();
        for s in &self.recipe.secrets {
            if !seen_secrets.insert(s) {
                self.err(
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
                    self.err(
                        ValidationCode::UnexpectedBrowserConfig,
                        "HTTP-engine recipe must not declare a `browser { … }` block",
                    );
                }
            }
            EngineKind::Browser => {
                if self.recipe.browser.is_none() {
                    self.err(
                        ValidationCode::MissingBrowserConfig,
                        "browser-engine recipe must declare a `browser { … }` block",
                    );
                }
                if matches!(self.recipe.auth, Some(AuthStrategy::Session(_))) {
                    self.warn(
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
                self.err(
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
        match s {
            Statement::Step(step) => {
                self.check_template(&step.request.url);
                for (_, v) in &step.request.headers {
                    self.check_template(v);
                }
                if let Some(b) = &step.request.body {
                    self.check_body(b);
                }
            }
            Statement::Emit(em) => self.check_emit(em),
            Statement::ForLoop {
                variable,
                collection,
                body,
            } => {
                self.check_extraction(collection);
                let inserted = self.known_vars.insert(variable.clone());
                for s in body {
                    self.check_statement(s);
                }
                if inserted {
                    self.known_vars.remove(variable);
                }
            }
        }
    }

    fn check_emit(&mut self, em: &Emission) {
        if self.recipe.ty(&em.type_name).is_none() {
            self.err(
                ValidationCode::UnknownType,
                format!("emit Type '{}' is not declared", em.type_name),
            );
            return;
        }
        let ty = self.recipe.ty(&em.type_name).unwrap().clone();
        let bound: std::collections::HashSet<&str> =
            em.bindings.iter().map(|b| b.field_name.as_str()).collect();
        for f in &ty.fields {
            if !f.optional && !bound.contains(f.name.as_str()) {
                self.err(
                    ValidationCode::MissingRequiredField,
                    format!("emit {} missing required field '{}'", em.type_name, f.name),
                );
            }
        }
        for b in &em.bindings {
            if ty.field(&b.field_name).is_none() {
                self.err(
                    ValidationCode::UnknownField,
                    format!("type {} has no field '{}'", em.type_name, b.field_name),
                );
            }
            self.check_extraction(&b.expr);
        }
    }

    fn check_extraction(&mut self, e: &ExtractionExpr) {
        match e {
            ExtractionExpr::Path(p) => self.check_path(p),
            ExtractionExpr::Pipe(inner, calls) => {
                self.check_extraction(inner);
                for c in calls {
                    self.check_transform_name(&c.name);
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
                self.check_transform_name(name);
                for a in args {
                    self.check_extraction(a);
                }
            }
            ExtractionExpr::Literal(_) => {}
        }
    }

    fn check_transform_name(&mut self, name: &str) {
        if !BUILTIN_TRANSFORMS.contains(&name) {
            self.err(
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
            if let Some(en) = self.recipe.recipe_enum(&enum_name).cloned() {
                let known: std::collections::HashSet<&str> =
                    en.variants.iter().map(|s| s.as_str()).collect();
                let used: Vec<String> = labels.map(|s| s.to_string()).collect();
                for l in &used {
                    if !known.contains(l.as_str()) {
                        self.err(
                            ValidationCode::UnknownEnumVariant,
                            format!("case label '{l}' is not a variant of enum {enum_name}"),
                        );
                    }
                }
                let used_set: std::collections::HashSet<&str> =
                    used.iter().map(|s| s.as_str()).collect();
                for v in &en.variants {
                    if !used_set.contains(v.as_str()) {
                        self.warn(
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
                    self.err(
                        ValidationCode::UnknownSecret,
                        format!("$secret.{name} references an undeclared secret"),
                    );
                }
            }
            PathExpr::Variable(name) => {
                if !self.known_vars.contains(name) {
                    self.err(
                        ValidationCode::UnknownVariable,
                        format!("$ {name} is an unbound variable"),
                    );
                }
            }
            PathExpr::Field(base, field) | PathExpr::OptField(base, field) => {
                // `$input.X` — check X is declared.
                if let PathExpr::Input = base.as_ref() {
                    if self.recipe.input(field).is_none() {
                        self.err(
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
        for ty in &self.recipe.types {
            for f in &ty.fields {
                self.check_field_type(&f.ty, &format!("type {}.{}", ty.name, f.name));
            }
        }
        for inp in &self.recipe.inputs {
            self.check_field_type(&inp.ty, &format!("input {}", inp.name));
        }
    }

    fn check_field_type(&mut self, t: &FieldType, where_: &str) {
        match t {
            FieldType::Array(inner) => self.check_field_type(inner, where_),
            FieldType::Record(name) => {
                if self.recipe.ty(name).is_none() && self.recipe.recipe_enum(name).is_none() {
                    self.err(
                        ValidationCode::UnknownType,
                        format!("{where_} references unknown type '{name}'"),
                    );
                }
            }
            FieldType::EnumRef(name) => {
                if self.recipe.recipe_enum(name).is_none() {
                    self.err(
                        ValidationCode::UnknownEnum,
                        format!("{where_} references unknown enum '{name}'"),
                    );
                }
            }
            FieldType::String | FieldType::Int | FieldType::Double | FieldType::Bool => {}
        }
    }
}

/// Walk a body recursively and collect every name introduced into scope:
/// step names, `extract.regex` group bindings, nested for-loop variables.
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
            recipe "ok" {
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
            }
        "#;
        let r = parse(src).unwrap();
        let rep = validate(&r);
        assert!(!rep.has_errors(), "got errors: {:?}", rep.issues);
    }

    #[test]
    fn unknown_input_flagged() {
        let src = r#"
            recipe "bad" {
                engine http
                type Item { id: String }
                step list {
                    method "GET"
                    url    "https://example.com/items?limit={$input.notDeclared}"
                }
                for $x in $list.items[*] {
                    emit Item { id ← $x.id }
                }
            }
        "#;
        let r = parse(src).unwrap();
        let rep = validate(&r);
        assert!(rep.has_errors());
        assert!(
            rep.errors()
                .any(|i| matches!(i.code, ValidationCode::UnknownInput))
        );
    }

    #[test]
    fn missing_required_field_flagged() {
        let src = r#"
            recipe "bad" {
                engine http
                type Item { id: String, name: String }
                step list {
                    method "GET"
                    url "https://example.com"
                }
                for $x in $list.items[*] {
                    emit Item { id ← $x.id }
                }
            }
        "#;
        let r = parse(src).unwrap();
        let rep = validate(&r);
        assert!(
            rep.errors()
                .any(|i| matches!(i.code, ValidationCode::MissingRequiredField))
        );
    }

    #[test]
    fn http_recipe_with_browser_block_flagged() {
        let src = r#"
            recipe "bad" {
                engine http
                type Item { id: String }
                browser {
                    initialURL: "x"
                    observe:    "y"
                    paginate browserPaginate.scroll { until: noProgressFor(1) }
                }
            }
        "#;
        let r = parse(src).unwrap();
        let rep = validate(&r);
        assert!(
            rep.errors()
                .any(|i| matches!(i.code, ValidationCode::UnexpectedBrowserConfig))
        );
    }

    #[test]
    fn undeclared_secret_flagged() {
        let src = r#"
            recipe "bad" {
                engine http
                type Item { id: String }
                step list {
                    method "GET"
                    url "https://example.com/{$secret.token}"
                }
                for $x in $list.items[*] {
                    emit Item { id ← $x.id }
                }
            }
        "#;
        let r = parse(src).unwrap();
        let rep = validate(&r);
        assert!(
            rep.errors()
                .any(|i| matches!(i.code, ValidationCode::UnknownSecret))
        );
    }
}
