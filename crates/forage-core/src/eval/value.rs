//! Runtime values flowing through extraction.
//!
//! Distinct from `JSONValue`: `EvalValue` adds `Node` / `NodeList` for
//! HTML elements after `parseHtml`. At emit boundaries Node values are
//! flattened to outerHTML strings before they land in the snapshot.

use indexmap::IndexMap;

use crate::ast::JSONValue;

/// Compiled regex carrier — wraps `regex::Regex` so equality and
/// debug-printing fall back to the source pattern + flags. Regex
/// values flow through `match` / `matches` / `replaceAll` and never
/// escape into snapshot output, so the wrapper doesn't need
/// `Serialize` — and *must not* gain it, or the schema-derivation
/// pass would attempt to write a regex into a record.
#[derive(Debug, Clone)]
pub struct RegexValue {
    pub pattern: String,
    pub flags: String,
    pub re: regex::Regex,
}

impl PartialEq for RegexValue {
    fn eq(&self, other: &Self) -> bool {
        self.pattern == other.pattern && self.flags == other.flags
    }
}

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
    /// Typed reference to a previously-emitted record. Produced by an
    /// `emit T { … } as $v` binding; the engine writes the bound
    /// record's `_id` into `id` and the type name into `target_type`.
    /// Serializes into a snapshot as `{"_ref": <id>, "_type": <type>}`.
    Ref {
        target_type: String,
        id: String,
    },
    /// Pre-compiled regex value, intermediate-only. Produced by a
    /// regex literal `/pattern/flags`; consumed by `match`, `matches`,
    /// `replaceAll`. If one ever lands on an emit field, `into_json`
    /// loudly fails — a regex isn't a snapshot value.
    Regex(RegexValue),
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
            // A ref always points at a real emitted record.
            EvalValue::Ref { .. } => true,
            // A compiled regex is always a meaningful value at the
            // point it's evaluated; truthiness only matters because the
            // language uses it for case-of and conditional branches.
            EvalValue::Regex(_) => true,
        }
    }

    /// Convert to JSONValue. Nodes serialize as outerHTML strings; refs
    /// serialize as a self-describing `{"_ref": id, "_type": type}`
    /// object so consumers can distinguish typed pointers from arbitrary
    /// object fields without an out-of-band schema.
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
            EvalValue::Ref { target_type, id } => {
                let mut out = IndexMap::new();
                out.insert("_ref".into(), JSONValue::String(id));
                out.insert("_type".into(), JSONValue::String(target_type));
                JSONValue::Object(out)
            }
            // A regex landing on a snapshot boundary means the recipe
            // bound a regex literal directly into an emit field. That's
            // a bug (regex values are intermediate); panic with a
            // diagnostic so it surfaces in tests rather than silently
            // becoming garbled JSON.
            EvalValue::Regex(r) => panic!(
                "regex literal /{}/{} reached snapshot serialization — regex values are intermediate only",
                r.pattern, r.flags,
            ),
        }
    }
}

impl From<&crate::snapshot::Record> for EvalValue {
    /// Project a snapshot `Record` back into an `EvalValue::Object`
    /// suitable for binding into a downstream recipe's scope. The
    /// record's synthetic `_id` rides through as a string field so
    /// composed recipes can address upstream records by their stable
    /// id (and `Ref<T>` field values continue to resolve via the same
    /// path).
    fn from(rec: &crate::snapshot::Record) -> Self {
        let mut out: IndexMap<String, EvalValue> = IndexMap::new();
        out.insert(
            "_id".into(),
            EvalValue::String(rec.id.clone()),
        );
        for (k, v) in &rec.fields {
            out.insert(k.clone(), EvalValue::from(v.clone()));
        }
        EvalValue::Object(out)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[should_panic(expected = "intermediate only")]
    fn regex_into_json_panics() {
        // Regex values must never reach the snapshot. If one does, the
        // recipe wired a regex literal into an emit field, which is a
        // bug — panic so the test surfacing the misuse is loud rather
        // than emitting opaque JSON.
        let r = EvalValue::Regex(RegexValue {
            pattern: "abc".into(),
            flags: String::new(),
            re: regex::Regex::new("abc").unwrap(),
        });
        let _ = r.into_json();
    }

    #[test]
    fn ref_serializes_to_self_describing_object() {
        // The wire shape distinguishes "this is a typed pointer" from
        // "this is an arbitrary object with `_ref`/`_type` keys" by
        // convention: only refs land here, because the engine writes
        // them through `EvalValue::Ref::into_json`.
        let r = EvalValue::Ref {
            target_type: "Product".into(),
            id: "rec-3".into(),
        };
        let j = r.into_json();
        let JSONValue::Object(o) = j else {
            panic!("expected object");
        };
        assert_eq!(o.get("_ref"), Some(&JSONValue::String("rec-3".into())));
        assert_eq!(o.get("_type"), Some(&JSONValue::String("Product".into())),);
    }
}
