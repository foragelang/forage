//! HTTP request shape — one step in a recipe's HTTP graph.

use serde::{Deserialize, Serialize};

use crate::ast::expr::{PathExpr, Template};
use crate::ast::json::JSONValue;
use crate::ast::pagination::Pagination;
use crate::ast::span::Span;

/// One HTTP step. Iteration + pagination loops are wrapped around the step
/// by the surrounding `Statement::ForLoop` / `HTTPStep::pagination`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HTTPStep {
    pub name: String,
    pub request: HTTPRequest,
    #[serde(default)]
    pub pagination: Option<Pagination>,
    /// Regex extract block: pull named groups out of the response body and
    /// bind them as scope variables for subsequent steps. Commonly used
    /// with `auth.htmlPrime` to pull a nonce out of an HTML page.
    #[serde(default)]
    pub extract: Option<RegexExtract>,
    /// Source range from `step` keyword through the closing `}`. Populated
    /// by the parser; defaults to `0..0` when an `HTTPStep` is constructed
    /// by hand (tests, fixtures). LSP + Studio use it to anchor diagnostics
    /// and breakpoint markers at the actual step location.
    #[serde(default)]
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RegexExtract {
    pub pattern: String,
    /// Names for each capture group, in order; group N (1-based) binds to
    /// the Nth name in this list.
    pub groups: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HTTPRequest {
    pub method: String,
    pub url: Template,
    pub headers: Vec<(String, Template)>,
    pub body: Option<HTTPBody>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum HTTPBody {
    /// JSON-encoded body. The runtime renders `BodyValue` against the scope.
    JsonObject(Vec<HTTPBodyKV>),
    /// `application/x-www-form-urlencoded` body.
    Form(Vec<(String, BodyValue)>),
    /// Raw text body rendered from a template.
    Raw(Template),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HTTPBodyKV {
    pub key: String,
    pub value: BodyValue,
}

/// Value position in a JSON body. `TemplateString` is for `"{$x}"` interpolation;
/// `Literal` is for numeric/boolean/null constants.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum BodyValue {
    TemplateString(Template),
    Literal(JSONValue),
    /// `key: $input.x` — substitute the resolved value.
    Path(PathExpr),
    Object(Vec<HTTPBodyKV>),
    Array(Vec<BodyValue>),
    /// `case $x of { A → ...; B → ... }` inside a body.
    CaseOf {
        scrutinee: PathExpr,
        branches: Vec<(String, BodyValue)>,
    },
}
