//! HTTP request shape — one step in a recipe's HTTP graph.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ast::expr::{PathExpr, Template};
use crate::ast::json::JSONValue;
use crate::ast::pagination::Pagination;
use crate::ast::span::Span;

/// How the engine should parse a step's response body when binding
/// `$<stepname>`. When `HTTPStep::parse` is `None` the engine falls
/// back to `Content-Type` detection; when `Some(fmt)` the override
/// wins regardless of what the server claims.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum ParseFormat {
    Json,
    Html,
    Xml,
    Text,
}

impl ParseFormat {
    /// Map a normalized (lowercased, `; charset=*` stripped) MIME type
    /// to a `ParseFormat`. Empty / unknown types fall through to
    /// `Text`. Used both at engine time (to pick the parser when the
    /// recipe didn't override) and to populate `StepResponse.format`
    /// when no override is present.
    pub fn from_content_type(mime: &str) -> Self {
        let m = mime.trim();
        if m == "application/json" || m.ends_with("+json") || m.starts_with("application/json") {
            ParseFormat::Json
        } else if m == "text/html" {
            ParseFormat::Html
        } else if m == "application/xml"
            || m == "text/xml"
            || m.ends_with("+xml")
            || m.starts_with("application/xml")
        {
            ParseFormat::Xml
        } else {
            ParseFormat::Text
        }
    }
}

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
    /// Recipe-level override for response-body parsing. When `Some`, the
    /// engine picks the parser based on this value instead of consulting
    /// the `Content-Type` header — useful when a host serves JSON with
    /// `text/plain` or HTML behind `application/octet-stream`.
    #[serde(default)]
    pub parse: Option<ParseFormat>,
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
