//! HTML node helpers backing `parseHtml`, `select`, `text`, `attr`,
//! `html`, `innerHtml`, `first`.
//!
//! Nodes are stored as outerHTML strings and re-parsed on demand via
//! `scraper::Html::parse_fragment`. The recipes we target are small
//! enough that this stays cheap; we trade microseconds-per-select for
//! a simple value model that round-trips through serde.

use scraper::{Html, Selector};

use crate::eval::error::EvalError;

pub fn select(html: &str, selector: &str) -> Result<Vec<String>, EvalError> {
    let sel = Selector::parse(selector).map_err(|e| EvalError::TransformError {
        name: "select".into(),
        msg: format!("invalid selector '{selector}': {e:?}"),
    })?;
    let parsed = Html::parse_fragment(html);
    let mut out = Vec::new();
    for el in parsed.select(&sel) {
        out.push(el.html());
    }
    Ok(out)
}

pub fn text_of(html: &str) -> String {
    let parsed = Html::parse_fragment(html);
    let mut out = String::new();
    for n in parsed.tree.root().descendants() {
        if let Some(t) = n.value().as_text() {
            out.push_str(&t.text);
        }
    }
    out.trim().to_string()
}

pub fn attr_of(html: &str, name: &str) -> Option<String> {
    let parsed = Html::parse_fragment(html);
    // Find first element under <html> root; scraper wraps fragments in
    // <html><head></head><body>...</body></html>.
    let root = parsed.tree.root();
    for n in root.descendants() {
        if let Some(el) = n.value().as_element() {
            // Skip the scraper-injected html/head/body wrappers; the first
            // "real" element is what we want.
            let tag = el.name();
            if matches!(tag, "html" | "head" | "body") {
                continue;
            }
            return el.attr(name).map(String::from);
        }
    }
    None
}

pub fn inner_html_of(html: &str) -> String {
    let parsed = Html::parse_fragment(html);
    let root = parsed.tree.root();
    for n in root.descendants() {
        if let Some(el) = n.value().as_element() {
            let tag = el.name();
            if matches!(tag, "html" | "head" | "body") {
                continue;
            }
            // Get inner HTML by serializing children.
            let mut out = String::new();
            for child in n.children() {
                if let Some(t) = child.value().as_text() {
                    out.push_str(&t.text);
                } else if let Some(_el) = child.value().as_element() {
                    // Use scraper's html() via ElementRef::wrap.
                    if let Some(eref) = scraper::ElementRef::wrap(child) {
                        out.push_str(&eref.html());
                    }
                }
            }
            return out;
        }
    }
    String::new()
}
