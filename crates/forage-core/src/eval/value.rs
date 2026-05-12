//! Runtime values flowing through extraction.
//!
//! Distinct from `JSONValue`: `EvalValue` adds `Node` / `NodeList` for
//! HTML elements after `parseHtml`. At emit boundaries Node values are
//! flattened to outerHTML strings before they land in the snapshot.

use indexmap::IndexMap;

use crate::ast::JSONValue;

#[derive(Debug, Clone, PartialEq)]
pub enum EvalValue {
    Null,
    Bool(bool),
    Int(i64),
    Double(f64),
    String(String),
    Array(Vec<EvalValue>),
    Object(IndexMap<String, EvalValue>),
    /// HTML fragment — outerHTML stored as a String, re-parsed on demand.
    Node(String),
    /// `select(...)` result — many node fragments.
    NodeList(Vec<String>),
}

impl EvalValue {
    pub fn is_null(&self) -> bool {
        matches!(self, EvalValue::Null)
    }

    pub fn is_truthy(&self) -> bool {
        match self {
            EvalValue::Null => false,
            EvalValue::Bool(b) => *b,
            EvalValue::Int(n) => *n != 0,
            EvalValue::Double(n) => *n != 0.0,
            EvalValue::String(s) => !s.is_empty(),
            EvalValue::Array(xs) => !xs.is_empty(),
            EvalValue::Object(o) => !o.is_empty(),
            EvalValue::Node(_) => true,
            EvalValue::NodeList(xs) => !xs.is_empty(),
        }
    }

    /// Convert to JSONValue. Nodes serialize as outerHTML strings.
    pub fn into_json(self) -> JSONValue {
        match self {
            EvalValue::Null => JSONValue::Null,
            EvalValue::Bool(b) => JSONValue::Bool(b),
            EvalValue::Int(n) => JSONValue::Int(n),
            EvalValue::Double(n) => JSONValue::Double(n),
            EvalValue::String(s) => JSONValue::String(s),
            EvalValue::Array(xs) => {
                JSONValue::Array(xs.into_iter().map(|x| x.into_json()).collect())
            }
            EvalValue::Object(o) => {
                let mut out = IndexMap::new();
                for (k, v) in o {
                    out.insert(k, v.into_json());
                }
                JSONValue::Object(out)
            }
            EvalValue::Node(html) => JSONValue::String(html),
            EvalValue::NodeList(xs) => {
                JSONValue::Array(xs.into_iter().map(JSONValue::String).collect())
            }
        }
    }
}

impl From<JSONValue> for EvalValue {
    fn from(v: JSONValue) -> Self {
        match v {
            JSONValue::Null => EvalValue::Null,
            JSONValue::Bool(b) => EvalValue::Bool(b),
            JSONValue::Int(n) => EvalValue::Int(n),
            JSONValue::Double(n) => EvalValue::Double(n),
            JSONValue::String(s) => EvalValue::String(s),
            JSONValue::Array(xs) => EvalValue::Array(xs.into_iter().map(EvalValue::from).collect()),
            JSONValue::Object(o) => {
                let mut out = IndexMap::new();
                for (k, v) in o {
                    out.insert(k, EvalValue::from(v));
                }
                EvalValue::Object(out)
            }
        }
    }
}

impl From<&serde_json::Value> for EvalValue {
    fn from(v: &serde_json::Value) -> Self {
        match v {
            serde_json::Value::Null => EvalValue::Null,
            serde_json::Value::Bool(b) => EvalValue::Bool(*b),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    EvalValue::Int(i)
                } else if let Some(f) = n.as_f64() {
                    EvalValue::Double(f)
                } else {
                    EvalValue::Null
                }
            }
            serde_json::Value::String(s) => EvalValue::String(s.clone()),
            serde_json::Value::Array(xs) => {
                EvalValue::Array(xs.iter().map(EvalValue::from).collect())
            }
            serde_json::Value::Object(map) => {
                let mut out = IndexMap::new();
                for (k, v) in map {
                    out.insert(k.clone(), EvalValue::from(v));
                }
                EvalValue::Object(out)
            }
        }
    }
}

impl From<serde_json::Value> for EvalValue {
    fn from(v: serde_json::Value) -> Self {
        (&v).into()
    }
}
