//! HTTP request shape — one step in a recipe's HTTP graph.

use serde::{Deserialize, Serialize};

use crate::ast::expr::{PathExpr, Template};
use crate::ast::json::JSONValue;
use crate::ast::pagination::Pagination;

/// One HTTP step. Iteration + pagination loops are wrapped around the step
/// by the surrounding `Statement::ForLoop` / `HTTPStep::pagination`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HTTPStep {
    pub name: String,
    pub request: HTTPRequest,
    pub pagination: Option<Pagination>,
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
