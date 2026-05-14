//! Path expressions, templates, extraction expressions, transform calls,
//! and emit blocks.

use serde::{Deserialize, Serialize};

use crate::ast::json::JSONValue;
use crate::ast::span::Span;

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

/// `emit Product { name ← $.name; brand ← $.brand?.name } as $p`.
/// Produces one record per execution; the runtime accumulates them.
///
/// The optional `bind_name` (post-fix `as $ident`) introduces a
/// scope-local binding of type `Ref<T>` so that subsequent emits in the
/// same lexical scope can link back to this record. The leading `$` is
/// stripped — `bind_name` carries just the identifier text.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Emission {
    pub type_name: String,
    pub bindings: Vec<FieldBinding>,
    /// `emit T { … } as $v` — the identifier after `as`, without the
    /// leading `$`. `None` when the emit isn't bound.
    #[serde(default)]
    pub bind_name: Option<String>,
    /// Source range from `emit` keyword through the closing `}` (or
    /// through the `$ident` suffix when an `as` binding is present).
    /// Populated by the parser; default (`0..0`) when constructed by hand.
    #[serde(default)]
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldBinding {
    pub field_name: String,
    pub expr: ExtractionExpr,
}
