//! Render `HTTPBody` against the recipe scope to concrete request bytes.

use indexmap::IndexMap;

use crate::error::{HttpError, HttpResult};
use forage_core::ast::*;
use forage_core::{EvalValue, Evaluator, Scope};

pub fn render_body(
    body: &HTTPBody,
    evaluator: &Evaluator<'_>,
    scope: &Scope,
) -> HttpResult<(String, Vec<u8>)> {
    match body {
        HTTPBody::Raw(t) => {
            let s = evaluator.render_template(t, scope)?;
            Ok(("text/plain".into(), s.into_bytes()))
        }
        HTTPBody::Form(kvs) => {
            let mut pairs = Vec::with_capacity(kvs.len());
            for (k, v) in kvs {
                let val = render_body_value(v, evaluator, scope)?;
                pairs.push(urlencode(k) + "=" + &urlencode(&value_as_string(&val)));
            }
            Ok((
                "application/x-www-form-urlencoded".into(),
                pairs.join("&").into_bytes(),
            ))
        }
        HTTPBody::JsonObject(kvs) => {
            let mut o = IndexMap::new();
            for kv in kvs {
                let val = render_body_value(&kv.value, evaluator, scope)?;
                o.insert(kv.key.clone(), value_to_serde(val));
            }
            let v = serde_json::Value::Object(o.into_iter().collect());
            let bytes = serde_json::to_vec(&v)
                .map_err(|e| HttpError::Generic(format!("body serialize: {e}")))?;
            Ok(("application/json".into(), bytes))
        }
    }
}

fn render_body_value(
    bv: &BodyValue,
    evaluator: &Evaluator<'_>,
    scope: &Scope,
) -> HttpResult<EvalValue> {
    match bv {
        BodyValue::Literal(j) => Ok(EvalValue::from(j.clone())),
        BodyValue::TemplateString(t) => {
            let s = evaluator.render_template(t, scope)?;
            Ok(EvalValue::String(s))
        }
        BodyValue::Path(p) => Ok(evaluator.eval_path(p, scope)?),
        BodyValue::Object(kvs) => {
            let mut o = IndexMap::new();
            for kv in kvs {
                o.insert(
                    kv.key.clone(),
                    render_body_value(&kv.value, evaluator, scope)?,
                );
            }
            Ok(EvalValue::Object(o))
        }
        BodyValue::Array(xs) => {
            let mut out = Vec::with_capacity(xs.len());
            for x in xs {
                out.push(render_body_value(x, evaluator, scope)?);
            }
            Ok(EvalValue::Array(out))
        }
        BodyValue::CaseOf {
            scrutinee,
            branches,
        } => {
            let v = evaluator.eval_path(scrutinee, scope)?;
            let label = match &v {
                EvalValue::Bool(b) => b.to_string(),
                EvalValue::String(s) => s.clone(),
                EvalValue::Int(n) => n.to_string(),
                EvalValue::Null => "null".into(),
                _ => {
                    return Err(HttpError::Generic(
                        "case-of: unsupported scrutinee type".into(),
                    ));
                }
            };
            for (l, val) in branches {
                if l == &label {
                    return render_body_value(val, evaluator, scope);
                }
            }
            Err(HttpError::Generic(format!(
                "case-of: no branch for {label:?}"
            )))
        }
    }
}

fn value_as_string(v: &EvalValue) -> String {
    match v {
        EvalValue::String(s) => s.clone(),
        EvalValue::Int(n) => n.to_string(),
        EvalValue::Double(n) => n.to_string(),
        EvalValue::Bool(b) => b.to_string(),
        EvalValue::Null => String::new(),
        _ => serde_json::to_string(&v.clone().into_json()).unwrap_or_default(),
    }
}

fn value_to_serde(v: EvalValue) -> serde_json::Value {
    let j = v.into_json();
    serde_json::to_value(&j).unwrap_or(serde_json::Value::Null)
}

fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push('+'),
            other => {
                out.push('%');
                out.push_str(&format!("{:02X}", other));
            }
        }
    }
    out
}
