//! Path expressions, templates, extraction expressions, transform calls,
//! and emit blocks.

use serde::{Deserialize, Serialize};

use crate::ast::json::JSONValue;

/// `$.x.y?.z`, `$input.storeId`, `$cat.id`, `$secret.password`, etc.
/// The runtime evaluates these against the current scope to produce a
/// `JSONValue` (or a list of values when `[*]` widens).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PathExpr {
    /// `$` — current value at the binding site.
    Current,
    /// `$input` — recipe input scope.
    Input,
    /// `$secret.<name>` — resolved via the host's secret resolver.
    Secret(String),
    /// `$<name>` — anything else introduced by a `for` binding or step result.
    Variable(String),
    /// `<base>.<field>`
    Field(Box<PathExpr>, String),
    /// `<base>?.<field>` — yields null on missing/null parent.
    OptField(Box<PathExpr>, String),
    /// `<base>[N]`
    Index(Box<PathExpr>, i64),
    /// `<base>[*]` — wildcard, broadens to a list.
    Wildcard(Box<PathExpr>),
}

impl PathExpr {
    /// All `$secret.<name>` references this expression mentions transitively.
    pub fn referenced_secrets(&self) -> Vec<String> {
        let mut out = Vec::new();
        self.collect_secrets(&mut out);
        out
    }

    fn collect_secrets(&self, out: &mut Vec<String>) {
        match self {
            PathExpr::Secret(n) => out.push(n.clone()),
            PathExpr::Current | PathExpr::Input | PathExpr::Variable(_) => {}
            PathExpr::Field(inner, _)
            | PathExpr::OptField(inner, _)
            | PathExpr::Index(inner, _)
            | PathExpr::Wildcard(inner) => inner.collect_secrets(out),
        }
    }
}

/// String template — `"prefix-{$.x}-suffix"` or `"price_{$weight | snake}"`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Template {
    pub parts: Vec<TemplatePart>,
}

impl Template {
    pub fn literal(s: impl Into<String>) -> Self {
        Self {
            parts: vec![TemplatePart::Literal(s.into())],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TemplatePart {
    Literal(String),
    Interp(ExtractionExpr),
}

/// RHS of a field binding. The runtime evaluates these against the current
/// scope to produce a typed value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ExtractionExpr {
    /// A bare path — `$.x.y?.z`.
    Path(PathExpr),
    /// `<expr> | <transform> | <transform>` — left-to-right pipeline.
    Pipe(Box<ExtractionExpr>, Vec<TransformCall>),
    /// `case $x of { A → expr; B → expr }` — switch on the scrutinee's enum value.
    CaseOf {
        scrutinee: PathExpr,
        branches: Vec<(String, ExtractionExpr)>,
    },
    /// `<expr> | map(<emit>)` — map a list to a list of typed records.
    MapTo {
        path: PathExpr,
        emission: Box<Emission>,
    },
    /// Inline literal — `"sweed"`, `42`, `true`.
    Literal(JSONValue),
    /// Template string with interpolations — `"{$.id}:{$weight}"`.
    Template(Template),
    /// Function-call-shaped transform — `coalesce(a, b)`,
    /// `normalizeOzToGrams($variant.unitSize?.unitAbbr)`.
    Call {
        name: String,
        args: Vec<ExtractionExpr>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransformCall {
    pub name: String,
    /// Optional positional args.
    pub args: Vec<ExtractionExpr>,
}

/// `emit Product { name ← $.name; brand ← $.brand?.name }`.
/// Produces one record per execution; the runtime accumulates them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Emission {
    pub type_name: String,
    pub bindings: Vec<FieldBinding>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldBinding {
    pub field_name: String,
    pub expr: ExtractionExpr,
}
