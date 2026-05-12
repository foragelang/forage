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

struct Validator<'a> {
    recipe: &'a Recipe,
    issues: Vec<ValidationIssue>,
}

impl<'a> Validator<'a> {
    fn new(recipe: &'a Recipe) -> Self {
        Self {
            recipe,
            issues: Vec::new(),
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
            for cap in &b.captures {
                self.check_extraction(&cap.iter_path);
                for s in &cap.body.clone() {
                    self.check_statement(s);
                }
            }
            if let Some(doc) = &b.document_capture {
                self.check_extraction(&doc.iter_path);
                for s in &doc.body.clone() {
                    self.check_statement(s);
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
                collection, body, ..
            } => {
                self.check_extraction(collection);
                for s in body {
                    self.check_statement(s);
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
            ExtractionExpr::Call { args, .. } => {
                for a in args {
                    self.check_extraction(a);
                }
            }
            ExtractionExpr::Literal(_) => {}
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
            PathExpr::Current | PathExpr::Input | PathExpr::Variable(_) => {}
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
