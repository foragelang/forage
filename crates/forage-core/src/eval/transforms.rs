//! Registry of built-in transforms.
//!
//! Two categories live side by side:
//! - **Sync transforms** `fn(EvalValue, &[EvalValue]) -> Result<EvalValue>` —
//!   pure data shaping (string, regex, HTML, JSON parsing). The
//!   evaluator's sync path applies them directly.
//! - **Transport-aware async transforms** `(EvalValue, Vec<EvalValue>, &dyn
//!   TransportContext) -> BoxFuture<…>` — fetch over the engine's
//!   `Transport` (so `--replay <fixtures>` covers them just like
//!   step-level requests). The async eval path resolves them; the sync
//!   path errors with [`EvalError::TransformRequiresTransport`].
//!
//! User-defined `fn`s shadow built-ins by name (validator warns).

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::OnceLock;

use crate::ast::FnDecl;
use crate::eval::error::EvalError;
use crate::eval::html;
use crate::eval::value::EvalValue;

pub type TransformFn = fn(EvalValue, &[EvalValue]) -> Result<EvalValue, EvalError>;

/// Future returned by a transport-aware transform.
pub type TransformFuture<'a> =
    Pin<Box<dyn Future<Output = Result<EvalValue, EvalError>> + Send + 'a>>;

/// A transport-aware transform: receives the pipe head, the call args,
/// and a borrowed transport context to issue fetches through. The
/// `Vec<EvalValue>` (rather than `&[…]`) frees the future from
/// borrowing the caller's stack so the dispatch site can move the args
/// into the future.
pub type AsyncTransformFn = for<'a> fn(
    EvalValue,
    Vec<EvalValue>,
    &'a dyn crate::eval::TransportContext,
) -> TransformFuture<'a>;

#[derive(Default)]
pub struct TransformRegistry {
    table: HashMap<String, TransformFn>,
    async_table: HashMap<String, AsyncTransformFn>,
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

    pub fn register_async(&mut self, name: &str, f: AsyncTransformFn) {
        self.async_table.insert(name.into(), f);
    }

    pub fn get(&self, name: &str) -> Option<TransformFn> {
        self.table.get(name).copied()
    }

    pub fn get_async(&self, name: &str) -> Option<AsyncTransformFn> {
        self.async_table.get(name).copied()
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
            async_table: base.async_table.clone(),
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
    r.register("lowercase", lower);
    r.register("uppercase", upper);
    r.register("replace", replace);
    r.register("split", split);

    // --- regex ---
    r.register("match", regex_match);
    r.register("matches", regex_matches);
    r.register("replaceAll", regex_replace_all);

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
        EvalValue::Regex(_) => "regex",
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

// --- regex consumers -------------------------------------------------------

fn require_regex<'a>(name: &str, v: &'a EvalValue) -> Result<&'a regex::Regex, EvalError> {
    match v {
        EvalValue::Regex(r) => Ok(&r.re),
        other => Err(err(
            name,
            format!("expected regex literal, got {}", type_name(other)),
        )),
    }
}

fn regex_match(v: EvalValue, args: &[EvalValue]) -> Result<EvalValue, EvalError> {
    let s = require_string("match", &v)?;
    let pat = args
        .first()
        .ok_or_else(|| err("match", "missing regex argument"))?;
    let re = require_regex("match", pat)?;
    let mut out = indexmap::IndexMap::new();
    match re.captures(&s) {
        Some(caps) => {
            let mut groups: Vec<EvalValue> = Vec::with_capacity(caps.len());
            for i in 0..caps.len() {
                match caps.get(i) {
                    Some(m) => groups.push(EvalValue::String(m.as_str().into())),
                    None => groups.push(EvalValue::Null),
                }
            }
            out.insert("matched".into(), EvalValue::Bool(true));
            out.insert("captures".into(), EvalValue::Array(groups));
        }
        None => {
            out.insert("matched".into(), EvalValue::Bool(false));
            out.insert("captures".into(), EvalValue::Array(Vec::new()));
        }
    }
    Ok(EvalValue::Object(out))
}

fn regex_matches(v: EvalValue, args: &[EvalValue]) -> Result<EvalValue, EvalError> {
    let s = require_string("matches", &v)?;
    let pat = args
        .first()
        .ok_or_else(|| err("matches", "missing regex argument"))?;
    let re = require_regex("matches", pat)?;
    Ok(EvalValue::Bool(re.is_match(&s)))
}

fn regex_replace_all(v: EvalValue, args: &[EvalValue]) -> Result<EvalValue, EvalError> {
    let s = require_string("replaceAll", &v)?;
    let pat = args
        .first()
        .ok_or_else(|| err("replaceAll", "missing regex argument"))?;
    let re = require_regex("replaceAll", pat)?;
    let replacement = args
        .get(1)
        .ok_or_else(|| err("replaceAll", "missing replacement string"))?;
    let rep = require_string("replaceAll", replacement)?;
    Ok(EvalValue::String(re.replace_all(&s, rep.as_str()).into_owned()))
}

// --- string built-ins ------------------------------------------------------

fn replace(v: EvalValue, args: &[EvalValue]) -> Result<EvalValue, EvalError> {
    let s = require_string("replace", &v)?;
    let from = args
        .first()
        .ok_or_else(|| err("replace", "missing 'from' argument"))?;
    let from = require_string("replace", from)?;
    let to = args
        .get(1)
        .ok_or_else(|| err("replace", "missing 'to' argument"))?;
    let to = require_string("replace", to)?;
    Ok(EvalValue::String(s.replace(from.as_str(), to.as_str())))
}

fn split(v: EvalValue, args: &[EvalValue]) -> Result<EvalValue, EvalError> {
    let s = require_string("split", &v)?;
    let sep = args
        .first()
        .ok_or_else(|| err("split", "missing separator"))?;
    let sep = require_string("split", sep)?;
    let parts: Vec<EvalValue> = s
        .split(sep.as_str())
        .map(|p| EvalValue::String(p.into()))
        .collect();
    Ok(EvalValue::Array(parts))
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
    fn string_built_ins_round_trip() {
        // The five string transforms cover the substitution & casing
        // surface the cannabis transforms used to wrap; pinning their
        // shape here so a recipe author can mix them with user-defined
        // fns without surprises.
        let r = default_registry();
        let lc = r.get("lowercase").unwrap();
        let uc = r.get("uppercase").unwrap();
        let tr = r.get("trim").unwrap();
        let rp = r.get("replace").unwrap();
        let sp = r.get("split").unwrap();
        assert_eq!(
            lc(EvalValue::String("HiYa".into()), &[]).unwrap(),
            EvalValue::String("hiya".into()),
        );
        assert_eq!(
            uc(EvalValue::String("HiYa".into()), &[]).unwrap(),
            EvalValue::String("HIYA".into()),
        );
        assert_eq!(
            tr(EvalValue::String("  hello  ".into()), &[]).unwrap(),
            EvalValue::String("hello".into()),
        );
        assert_eq!(
            rp(
                EvalValue::String("a-b-c".into()),
                &[
                    EvalValue::String("-".into()),
                    EvalValue::String(":".into()),
                ],
            )
            .unwrap(),
            EvalValue::String("a:b:c".into()),
        );
        assert_eq!(
            sp(
                EvalValue::String("a,b,c".into()),
                &[EvalValue::String(",".into())]
            )
            .unwrap(),
            EvalValue::Array(vec![
                EvalValue::String("a".into()),
                EvalValue::String("b".into()),
                EvalValue::String("c".into()),
            ]),
        );
    }

    #[test]
    fn regex_match_extracts_groups() {
        let r = default_registry();
        let m = r.get("match").unwrap();
        let pat = crate::eval::value::RegexValue {
            pattern: r"(\d+)\s*(oz|g)".into(),
            flags: String::new(),
            re: regex::Regex::new(r"(\d+)\s*(oz|g)").unwrap(),
        };
        let res = m(
            EvalValue::String("1 oz".into()),
            &[EvalValue::Regex(pat)],
        )
        .unwrap();
        let EvalValue::Object(o) = res else {
            panic!("expected object");
        };
        assert_eq!(o.get("matched"), Some(&EvalValue::Bool(true)));
        let EvalValue::Array(caps) = o.get("captures").unwrap() else {
            panic!("captures should be array");
        };
        assert_eq!(caps[0], EvalValue::String("1 oz".into()));
        assert_eq!(caps[1], EvalValue::String("1".into()));
        assert_eq!(caps[2], EvalValue::String("oz".into()));
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
