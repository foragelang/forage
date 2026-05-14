//! Registry of built-in transforms.
//!
//! Each transform: `fn(input: EvalValue, args: &[EvalValue]) -> Result<EvalValue, EvalError>`.
//! Registered by name; the evaluator's pipe / call forms look up the
//! function and apply it.

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::ast::FnDecl;
use crate::eval::error::EvalError;
use crate::eval::html;
use crate::eval::value::EvalValue;

pub type TransformFn = fn(EvalValue, &[EvalValue]) -> Result<EvalValue, EvalError>;

#[derive(Default)]
pub struct TransformRegistry {
    table: HashMap<String, TransformFn>,
    /// User-defined transforms cloned from the recipe at engine boot.
    /// Layered on top of the built-ins: `get_user_fn` is consulted
    /// before the built-in table, so a recipe-level `fn lower($x) { … }`
    /// would shadow the built-in `lower` (validator surfaces the
    /// shadow as a warning).
    user_fns: HashMap<String, FnDecl>,
}

impl TransformRegistry {
    pub fn register(&mut self, name: &str, f: TransformFn) {
        self.table.insert(name.into(), f);
    }

    pub fn get(&self, name: &str) -> Option<TransformFn> {
        self.table.get(name).copied()
    }

    pub fn get_user_fn(&self, name: &str) -> Option<&FnDecl> {
        self.user_fns.get(name)
    }

    /// Build a registry by layering user-defined functions on top of an
    /// existing one. The base registry's built-in entries are cloned in;
    /// `fns` becomes the user-fn table. The base is conventionally
    /// `default_registry()`, but isolated tests can pass a fresh
    /// registry too.
    pub fn with_user_fns(base: &TransformRegistry, fns: Vec<FnDecl>) -> Self {
        let mut user_fns = HashMap::new();
        for f in fns {
            user_fns.insert(f.name.clone(), f);
        }
        Self {
            table: base.table.clone(),
            user_fns,
        }
    }
}

/// Process-global default registry; lazily initialized.
pub fn default_registry() -> &'static TransformRegistry {
    static REG: OnceLock<TransformRegistry> = OnceLock::new();
    REG.get_or_init(build_default)
}

fn build_default() -> TransformRegistry {
    let mut r = TransformRegistry::default();

    // --- string ---
    r.register("toString", to_string);
    r.register("lower", lower);
    r.register("upper", upper);
    r.register("trim", trim);
    r.register("capitalize", capitalize);
    r.register("titleCase", title_case);

    // --- parsing scalars ---
    r.register("parseInt", parse_int);
    r.register("parseFloat", parse_float);
    r.register("parseBool", parse_bool);

    // --- list / object ---
    r.register("length", length);
    r.register("dedup", dedup);
    r.register("first", first);
    r.register("coalesce", coalesce);
    r.register("default", default_v);

    // --- weight / size normalization (cannabis-domain helpers) ---
    r.register("parseSize", parse_size);
    r.register("normalizeOzToGrams", normalize_oz_to_grams);
    r.register("sizeValue", size_value);
    r.register("sizeUnit", size_unit);
    r.register("normalizeUnitToGrams", normalize_unit_to_grams);
    r.register("prevalenceNormalize", prevalence_normalize);
    r.register("parseJaneWeight", parse_jane_weight);
    r.register("janeWeightUnit", jane_weight_unit);
    r.register("janeWeightKey", jane_weight_key);

    // --- field access (dynamic) ---
    r.register("getField", get_field);

    // --- HTML / JSON parsing ---
    r.register("parseHtml", parse_html);
    r.register("parseJson", parse_json);
    r.register("select", select);
    r.register("text", text);
    r.register("attr", attr);
    r.register("html", html_fn);
    r.register("innerHtml", inner_html);

    r
}

// === implementations ====================================================

fn type_name(v: &EvalValue) -> &'static str {
    match v {
        EvalValue::Null => "null",
        EvalValue::Bool(_) => "bool",
        EvalValue::Int(_) => "int",
        EvalValue::Double(_) => "double",
        EvalValue::String(_) => "string",
        EvalValue::Array(_) => "array",
        EvalValue::Object(_) => "object",
        EvalValue::Node(_) => "node",
        EvalValue::NodeList(_) => "nodelist",
        EvalValue::Ref { .. } => "ref",
    }
}

fn err(name: &str, msg: impl Into<String>) -> EvalError {
    EvalError::TransformError {
        name: name.into(),
        msg: msg.into(),
    }
}

fn require_string(name: &str, v: &EvalValue) -> Result<String, EvalError> {
    match v {
        EvalValue::String(s) => Ok(s.clone()),
        EvalValue::Node(s) => Ok(s.clone()),
        EvalValue::Int(n) => Ok(n.to_string()),
        EvalValue::Double(n) => Ok(n.to_string()),
        EvalValue::Bool(b) => Ok(b.to_string()),
        EvalValue::Null => Ok(String::new()),
        _ => Err(err(name, format!("expected string, got {}", type_name(v)))),
    }
}

fn to_string(v: EvalValue, _: &[EvalValue]) -> Result<EvalValue, EvalError> {
    Ok(EvalValue::String(require_string("toString", &v)?))
}

fn lower(v: EvalValue, _: &[EvalValue]) -> Result<EvalValue, EvalError> {
    Ok(EvalValue::String(
        require_string("lower", &v)?.to_lowercase(),
    ))
}

fn upper(v: EvalValue, _: &[EvalValue]) -> Result<EvalValue, EvalError> {
    Ok(EvalValue::String(
        require_string("upper", &v)?.to_uppercase(),
    ))
}

fn trim(v: EvalValue, _: &[EvalValue]) -> Result<EvalValue, EvalError> {
    Ok(EvalValue::String(
        require_string("trim", &v)?.trim().to_string(),
    ))
}

fn capitalize(v: EvalValue, _: &[EvalValue]) -> Result<EvalValue, EvalError> {
    let s = require_string("capitalize", &v)?;
    let mut chars = s.chars();
    let out = match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    };
    Ok(EvalValue::String(out))
}

fn title_case(v: EvalValue, _: &[EvalValue]) -> Result<EvalValue, EvalError> {
    let s = require_string("titleCase", &v)?;
    let out = s
        .split_whitespace()
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                Some(c) => c.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    Ok(EvalValue::String(out))
}

fn parse_int(v: EvalValue, _: &[EvalValue]) -> Result<EvalValue, EvalError> {
    match v {
        EvalValue::Int(n) => Ok(EvalValue::Int(n)),
        EvalValue::Double(n) => Ok(EvalValue::Int(n as i64)),
        EvalValue::String(s) => s
            .trim()
            .parse::<i64>()
            .map(EvalValue::Int)
            .map_err(|_| err("parseInt", format!("not an integer: {s:?}"))),
        EvalValue::Null => Ok(EvalValue::Null),
        other => Err(err(
            "parseInt",
            format!("can't parse {:?}", type_name(&other)),
        )),
    }
}

fn parse_float(v: EvalValue, _: &[EvalValue]) -> Result<EvalValue, EvalError> {
    match v {
        EvalValue::Int(n) => Ok(EvalValue::Double(n as f64)),
        EvalValue::Double(n) => Ok(EvalValue::Double(n)),
        EvalValue::String(s) => s
            .trim()
            .parse::<f64>()
            .map(EvalValue::Double)
            .map_err(|_| err("parseFloat", format!("not a number: {s:?}"))),
        EvalValue::Null => Ok(EvalValue::Null),
        other => Err(err(
            "parseFloat",
            format!("can't parse {:?}", type_name(&other)),
        )),
    }
}

fn parse_bool(v: EvalValue, _: &[EvalValue]) -> Result<EvalValue, EvalError> {
    match v {
        EvalValue::Bool(b) => Ok(EvalValue::Bool(b)),
        EvalValue::String(s) => match s.trim().to_lowercase().as_str() {
            "true" | "yes" | "1" => Ok(EvalValue::Bool(true)),
            "false" | "no" | "0" => Ok(EvalValue::Bool(false)),
            _ => Err(err("parseBool", format!("not a bool: {s:?}"))),
        },
        EvalValue::Int(n) => Ok(EvalValue::Bool(n != 0)),
        EvalValue::Null => Ok(EvalValue::Null),
        other => Err(err(
            "parseBool",
            format!("can't parse {:?}", type_name(&other)),
        )),
    }
}

fn length(v: EvalValue, _: &[EvalValue]) -> Result<EvalValue, EvalError> {
    match v {
        EvalValue::String(s) => Ok(EvalValue::Int(s.chars().count() as i64)),
        EvalValue::Array(xs) => Ok(EvalValue::Int(xs.len() as i64)),
        EvalValue::Object(o) => Ok(EvalValue::Int(o.len() as i64)),
        EvalValue::NodeList(xs) => Ok(EvalValue::Int(xs.len() as i64)),
        EvalValue::Null => Ok(EvalValue::Int(0)),
        other => Err(err(
            "length",
            format!("can't get length of {}", type_name(&other)),
        )),
    }
}

fn dedup(v: EvalValue, _: &[EvalValue]) -> Result<EvalValue, EvalError> {
    match v {
        EvalValue::Array(xs) => {
            let mut seen = Vec::<EvalValue>::new();
            for x in xs {
                if !seen.contains(&x) {
                    seen.push(x);
                }
            }
            Ok(EvalValue::Array(seen))
        }
        EvalValue::NodeList(xs) => {
            let mut seen = Vec::<String>::new();
            for x in xs {
                if !seen.contains(&x) {
                    seen.push(x);
                }
            }
            Ok(EvalValue::NodeList(seen))
        }
        other => Err(err(
            "dedup",
            format!("can only dedup arrays, got {}", type_name(&other)),
        )),
    }
}

fn first(v: EvalValue, _: &[EvalValue]) -> Result<EvalValue, EvalError> {
    match v {
        EvalValue::Array(mut xs) if !xs.is_empty() => Ok(xs.swap_remove(0)),
        EvalValue::Array(_) => Ok(EvalValue::Null),
        EvalValue::NodeList(mut xs) if !xs.is_empty() => Ok(EvalValue::Node(xs.swap_remove(0))),
        EvalValue::NodeList(_) => Ok(EvalValue::Null),
        EvalValue::String(s) => Ok(EvalValue::String(
            s.chars().next().map(String::from).unwrap_or_default(),
        )),
        EvalValue::Null => Ok(EvalValue::Null),
        other => Err(err(
            "first",
            format!("can't take first of {}", type_name(&other)),
        )),
    }
}

fn coalesce(v: EvalValue, args: &[EvalValue]) -> Result<EvalValue, EvalError> {
    if !v.is_null() {
        return Ok(v);
    }
    for a in args {
        if !a.is_null() {
            return Ok(a.clone());
        }
    }
    Ok(EvalValue::Null)
}

fn default_v(v: EvalValue, args: &[EvalValue]) -> Result<EvalValue, EvalError> {
    if v.is_null() {
        args.first()
            .cloned()
            .ok_or_else(|| err("default", "missing argument"))
    } else {
        Ok(v)
    }
}

// --- size / weight helpers --------------------------------------------------

fn parse_size_pair(s: &str) -> Option<(f64, String)> {
    let s = s.trim();
    let mut value_str = String::new();
    let mut rest = s;
    for (i, c) in s.char_indices() {
        if c.is_ascii_digit() || c == '.' {
            value_str.push(c);
        } else {
            rest = s[i..].trim();
            break;
        }
    }
    if value_str.is_empty() {
        return None;
    }
    let v: f64 = value_str.parse().ok()?;
    let unit = rest.to_lowercase();
    Some((v, unit))
}

fn parse_size(v: EvalValue, _: &[EvalValue]) -> Result<EvalValue, EvalError> {
    let s = require_string("parseSize", &v)?;
    let Some((val, unit)) = parse_size_pair(&s) else {
        return Ok(EvalValue::Null);
    };
    let mut o = indexmap::IndexMap::new();
    o.insert("value".into(), EvalValue::Double(val));
    o.insert("unit".into(), EvalValue::String(unit));
    Ok(EvalValue::Object(o))
}

fn normalize_oz_to_grams(v: EvalValue, args: &[EvalValue]) -> Result<EvalValue, EvalError> {
    // Called as `parseSize | normalizeOzToGrams` (no args) OR
    // `$.value | normalizeOzToGrams($variant.unit)` (value + unit arg).
    match (&v, args.first()) {
        // Single-arg form: object with {value, unit}.
        (EvalValue::Object(o), None) => {
            let val = o.get("value").cloned().unwrap_or(EvalValue::Null);
            let unit = o.get("unit").cloned().unwrap_or(EvalValue::Null);
            let new_val = to_grams(&val, &unit);
            let new_unit = match unit {
                EvalValue::String(u) if u == "oz" || u == "ounce" => EvalValue::String("g".into()),
                u => u,
            };
            let mut out = indexmap::IndexMap::new();
            out.insert("value".into(), new_val);
            out.insert("unit".into(), new_unit);
            Ok(EvalValue::Object(out))
        }
        // Two-arg form: (value, unit).
        (val, Some(unit)) => Ok(to_grams(val, unit)),
        _ => Ok(EvalValue::Null),
    }
}

fn to_grams(val: &EvalValue, unit: &EvalValue) -> EvalValue {
    let n = match val {
        EvalValue::Int(n) => *n as f64,
        EvalValue::Double(n) => *n,
        EvalValue::Null => return EvalValue::Null,
        _ => return EvalValue::Null,
    };
    let u = match unit {
        EvalValue::String(s) => s.to_lowercase(),
        _ => return EvalValue::Double(n),
    };
    let grams = match u.as_str() {
        "oz" | "ounce" | "ounces" => n * 28.0,
        "g" | "gram" | "grams" => n,
        "lb" | "lbs" | "pound" | "pounds" => n * 453.592,
        "mg" | "milligram" | "milligrams" => n / 1000.0,
        _ => return EvalValue::Double(n),
    };
    EvalValue::Double(grams)
}

fn size_value(v: EvalValue, _: &[EvalValue]) -> Result<EvalValue, EvalError> {
    if let EvalValue::Object(o) = &v {
        if let Some(val) = o.get("value") {
            return Ok(val.clone());
        }
    }
    Ok(EvalValue::Null)
}

fn size_unit(v: EvalValue, _: &[EvalValue]) -> Result<EvalValue, EvalError> {
    if let EvalValue::Object(o) = &v {
        if let Some(u) = o.get("unit") {
            return Ok(u.clone());
        }
    }
    Ok(EvalValue::Null)
}

fn normalize_unit_to_grams(v: EvalValue, _: &[EvalValue]) -> Result<EvalValue, EvalError> {
    let s = match &v {
        EvalValue::String(s) => s.to_lowercase(),
        EvalValue::Null => return Ok(EvalValue::Null),
        _ => return Ok(v),
    };
    let out = match s.as_str() {
        "oz" | "ounce" | "ounces" => "g",
        "g" | "gram" | "grams" => "g",
        "mg" | "milligram" | "milligrams" => "g",
        other => other,
    };
    Ok(EvalValue::String(out.into()))
}

fn prevalence_normalize(v: EvalValue, _: &[EvalValue]) -> Result<EvalValue, EvalError> {
    let s = match &v {
        EvalValue::String(s) => s.to_lowercase(),
        EvalValue::Null => return Ok(EvalValue::Null),
        _ => return Ok(v),
    };
    let out = match s.as_str() {
        "indica" | "indica-dominant" | "indica dominant" => "INDICA",
        "sativa" | "sativa-dominant" | "sativa dominant" => "SATIVA",
        "hybrid" => "HYBRID",
        "cbd" => "CBD",
        _ => return Ok(EvalValue::String(s)),
    };
    Ok(EvalValue::String(out.into()))
}

fn parse_jane_weight(v: EvalValue, _: &[EvalValue]) -> Result<EvalValue, EvalError> {
    let s = require_string("parseJaneWeight", &v)?.to_lowercase();
    let val = match s.as_str() {
        "half gram" => 0.5,
        "gram" => 1.0,
        "two gram" => 2.0,
        "eighth ounce" => 3.5,
        "quarter ounce" => 7.0,
        "half ounce" => 14.0,
        "ounce" => 28.0,
        "each" => return Ok(EvalValue::Null),
        _ => return Ok(EvalValue::Null),
    };
    Ok(EvalValue::Double(val))
}

fn jane_weight_unit(v: EvalValue, _: &[EvalValue]) -> Result<EvalValue, EvalError> {
    let s = require_string("janeWeightUnit", &v)?.to_lowercase();
    let unit = match s.as_str() {
        "each" => "EA",
        _ => "g",
    };
    Ok(EvalValue::String(unit.into()))
}

fn jane_weight_key(v: EvalValue, _: &[EvalValue]) -> Result<EvalValue, EvalError> {
    let s = require_string("janeWeightKey", &v)?.to_lowercase();
    Ok(EvalValue::String(s.replace(' ', "_")))
}

fn get_field(v: EvalValue, args: &[EvalValue]) -> Result<EvalValue, EvalError> {
    let name = args
        .first()
        .ok_or_else(|| err("getField", "missing field name arg"))?;
    let name_str = require_string("getField", name)?;
    match v {
        EvalValue::Object(o) => Ok(o.get(name_str.as_str()).cloned().unwrap_or(EvalValue::Null)),
        EvalValue::Null => Ok(EvalValue::Null),
        _ => Ok(EvalValue::Null),
    }
}

// --- HTML --------------------------------------------------------------

fn parse_html(v: EvalValue, _: &[EvalValue]) -> Result<EvalValue, EvalError> {
    let s = require_string("parseHtml", &v)?;
    Ok(EvalValue::Node(s))
}

fn parse_json(v: EvalValue, _: &[EvalValue]) -> Result<EvalValue, EvalError> {
    let s = require_string("parseJson", &v)?;
    let parsed: serde_json::Value =
        serde_json::from_str(&s).map_err(|e| err("parseJson", e.to_string()))?;
    Ok((&parsed).into())
}

fn select(v: EvalValue, args: &[EvalValue]) -> Result<EvalValue, EvalError> {
    let sel = args
        .first()
        .ok_or_else(|| err("select", "missing selector"))?;
    let sel = require_string("select", sel)?;
    let nodes: Vec<String> = match v {
        EvalValue::Node(h) => html::select(&h, &sel)?,
        EvalValue::NodeList(hs) => {
            let mut out = Vec::new();
            for h in hs {
                out.extend(html::select(&h, &sel)?);
            }
            out
        }
        EvalValue::String(s) => html::select(&s, &sel)?,
        other => {
            return Err(err(
                "select",
                format!("can't select on {}", type_name(&other)),
            ));
        }
    };
    Ok(EvalValue::NodeList(nodes))
}

fn text(v: EvalValue, _: &[EvalValue]) -> Result<EvalValue, EvalError> {
    match v {
        EvalValue::Node(h) => Ok(EvalValue::String(html::text_of(&h))),
        EvalValue::NodeList(hs) => {
            let parts: Vec<String> = hs.iter().map(|h| html::text_of(h)).collect();
            Ok(EvalValue::String(parts.join(" ")))
        }
        EvalValue::String(s) => Ok(EvalValue::String(s)),
        EvalValue::Null => Ok(EvalValue::Null),
        other => Err(err(
            "text",
            format!("can't text() on {}", type_name(&other)),
        )),
    }
}

fn attr(v: EvalValue, args: &[EvalValue]) -> Result<EvalValue, EvalError> {
    let name = args
        .first()
        .ok_or_else(|| err("attr", "missing attribute name"))?;
    let name = require_string("attr", name)?;
    match v {
        EvalValue::Node(h) => Ok(html::attr_of(&h, &name)
            .map(EvalValue::String)
            .unwrap_or(EvalValue::Null)),
        EvalValue::NodeList(hs) => {
            let mut out = Vec::new();
            for h in hs {
                if let Some(s) = html::attr_of(&h, &name) {
                    out.push(EvalValue::String(s));
                } else {
                    out.push(EvalValue::Null);
                }
            }
            Ok(EvalValue::Array(out))
        }
        EvalValue::Null => Ok(EvalValue::Null),
        other => Err(err(
            "attr",
            format!("can't attr() on {}", type_name(&other)),
        )),
    }
}

fn html_fn(v: EvalValue, _: &[EvalValue]) -> Result<EvalValue, EvalError> {
    match v {
        EvalValue::Node(h) => Ok(EvalValue::String(h)),
        EvalValue::NodeList(hs) => Ok(EvalValue::Array(
            hs.into_iter().map(EvalValue::String).collect(),
        )),
        other => Ok(other),
    }
}

fn inner_html(v: EvalValue, _: &[EvalValue]) -> Result<EvalValue, EvalError> {
    match v {
        EvalValue::Node(h) => Ok(EvalValue::String(html::inner_html_of(&h))),
        EvalValue::Null => Ok(EvalValue::Null),
        other => Err(err(
            "innerHtml",
            format!("can't innerHtml on {}", type_name(&other)),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_has_core_transforms() {
        let r = default_registry();
        assert!(r.get("toString").is_some());
        assert!(r.get("dedup").is_some());
        assert!(r.get("titleCase").is_some());
        assert!(r.get("parseHtml").is_some());
        assert!(r.get("coalesce").is_some());
    }

    #[test]
    fn title_case_works() {
        let f = default_registry().get("titleCase").unwrap();
        let v = f(EvalValue::String("hello world".into()), &[]).unwrap();
        assert_eq!(v, EvalValue::String("Hello World".into()));
    }

    #[test]
    fn coalesce_returns_first_non_null() {
        let f = default_registry().get("coalesce").unwrap();
        let v = f(
            EvalValue::Null,
            &[
                EvalValue::Null,
                EvalValue::String("hi".into()),
                EvalValue::Int(7),
            ],
        )
        .unwrap();
        assert_eq!(v, EvalValue::String("hi".into()));
    }

    #[test]
    fn jane_weight_parses() {
        let f = default_registry().get("parseJaneWeight").unwrap();
        assert_eq!(
            f(EvalValue::String("eighth ounce".into()), &[]).unwrap(),
            EvalValue::Double(3.5)
        );
        assert_eq!(
            f(EvalValue::String("ounce".into()), &[]).unwrap(),
            EvalValue::Double(28.0)
        );
    }

    #[test]
    fn html_select_text() {
        let parse_html_fn = default_registry().get("parseHtml").unwrap();
        let select_fn = default_registry().get("select").unwrap();
        let text_fn = default_registry().get("text").unwrap();

        let html =
            EvalValue::String(r#"<div class="row"><span class="title">Hello</span></div>"#.into());
        let node = parse_html_fn(html, &[]).unwrap();
        let nodes = select_fn(node, &[EvalValue::String(".title".into())]).unwrap();
        let t = text_fn(nodes, &[]).unwrap();
        assert_eq!(t, EvalValue::String("Hello".into()));
    }

    #[test]
    fn html_attr_works() {
        let parse_html_fn = default_registry().get("parseHtml").unwrap();
        let select_fn = default_registry().get("select").unwrap();
        let attr_fn = default_registry().get("attr").unwrap();
        let first_fn = default_registry().get("first").unwrap();

        let html = EvalValue::String(r#"<a href="/foo">link</a>"#.into());
        let node = parse_html_fn(html, &[]).unwrap();
        let nodes = select_fn(node, &[EvalValue::String("a".into())]).unwrap();
        let one = first_fn(nodes, &[]).unwrap();
        let v = attr_fn(one, &[EvalValue::String("href".into())]).unwrap();
        assert_eq!(v, EvalValue::String("/foo".into()));
    }
}
