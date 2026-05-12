//! Runtime JSON values flowing through extraction.
//!
//! Mirrors the Swift `JSONValue` minus the `.node` variant — HTML nodes
//! are handled by the evaluator, not the AST. `JSONValue` is used here
//! for *literal* values inside recipe source (numbers, strings, bools,
//! nested objects/arrays in body literals).

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum JSONValue {
    Null,
    Bool(bool),
    Int(i64),
    Double(f64),
    String(String),
    Array(Vec<JSONValue>),
    Object(IndexMap<String, JSONValue>),
}

impl JSONValue {
    pub fn is_null(&self) -> bool {
        matches!(self, JSONValue::Null)
    }
}

impl From<bool> for JSONValue {
    fn from(b: bool) -> Self {
        JSONValue::Bool(b)
    }
}

impl From<i64> for JSONValue {
    fn from(n: i64) -> Self {
        JSONValue::Int(n)
    }
}

impl From<f64> for JSONValue {
    fn from(n: f64) -> Self {
        JSONValue::Double(n)
    }
}

impl From<String> for JSONValue {
    fn from(s: String) -> Self {
        JSONValue::String(s)
    }
}

impl From<&str> for JSONValue {
    fn from(s: &str) -> Self {
        JSONValue::String(s.to_string())
    }
}
