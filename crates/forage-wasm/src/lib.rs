//! wasm-bindgen exports of `forage-core` for the hub site's web IDE.
//! Compiled via `wasm-pack build --target web`.

use wasm_bindgen::prelude::*;

use forage_core::{TypeCatalog, parse as core_parse, validate as core_validate};

#[wasm_bindgen]
pub fn forage_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Parse a recipe and return JSON: either the AST or a structured error.
///
/// Shape on success:
///   { ok: true, recipe: <Recipe as JSON> }
/// Shape on failure:
///   { ok: false, error: { message, span?: { start, end } } }
#[wasm_bindgen]
pub fn parse_recipe(source: &str) -> JsValue {
    match core_parse(source) {
        Ok(recipe) => {
            let body = serde_json::json!({
                "ok": true,
                "recipe": recipe,
            });
            serde_wasm_bindgen::to_value(&body).unwrap_or(JsValue::NULL)
        }
        Err(e) => {
            let (message, span) = match &e {
                forage_core::parse::ParseError::UnexpectedToken {
                    span,
                    expected,
                    found,
                } => (
                    format!("unexpected {found}, expected {expected}"),
                    Some((span.start, span.end)),
                ),
                forage_core::parse::ParseError::UnexpectedEof { expected } => (
                    format!("unexpected end of input, expected {expected}"),
                    None,
                ),
                forage_core::parse::ParseError::Generic { span, message } => {
                    (message.clone(), Some((span.start, span.end)))
                }
                forage_core::parse::ParseError::Lex(le) => (format!("{le}"), None),
            };
            let body = serde_json::json!({
                "ok": false,
                "error": {
                    "message": message,
                    "span": span.map(|(s, e)| serde_json::json!({ "start": s, "end": e })),
                },
            });
            serde_wasm_bindgen::to_value(&body).unwrap_or(JsValue::NULL)
        }
    }
}

/// Validate a recipe given its AST as JSON. Returns
///   { errors: [...], warnings: [...] }
#[wasm_bindgen]
pub fn validate_recipe(recipe_json: &str) -> JsValue {
    let recipe: forage_core::Recipe = match serde_json::from_str(recipe_json) {
        Ok(r) => r,
        Err(e) => {
            return serde_wasm_bindgen::to_value(&serde_json::json!({
                "errors": [{
                    "code": "InvalidASTJson",
                    "message": format!("{e}"),
                }],
                "warnings": [],
            }))
            .unwrap_or(JsValue::NULL);
        }
    };
    // The wasm IDE has no filesystem reach, so every recipe validates
    // in lonely-recipe mode — the catalog is just its own local types.
    let catalog = TypeCatalog::from_recipe(&recipe);
    let report = core_validate(&recipe, &catalog);
    let errors: Vec<_> = report
        .issues
        .iter()
        .filter(|i| matches!(i.severity, forage_core::Severity::Error))
        .map(|i| {
            serde_json::json!({
                "code": format!("{:?}", i.code),
                "message": i.message,
            })
        })
        .collect();
    let warnings: Vec<_> = report
        .issues
        .iter()
        .filter(|i| matches!(i.severity, forage_core::Severity::Warning))
        .map(|i| {
            serde_json::json!({
                "code": format!("{:?}", i.code),
                "message": i.message,
            })
        })
        .collect();
    serde_wasm_bindgen::to_value(&serde_json::json!({
        "errors": errors,
        "warnings": warnings,
    }))
    .unwrap_or(JsValue::NULL)
}

/// One-shot: parse + validate. Useful for the editor's hot path so the
/// JS side doesn't have to JSON-bridge the AST.
#[wasm_bindgen]
pub fn parse_and_validate(source: &str) -> JsValue {
    match core_parse(source) {
        Ok(recipe) => {
            let catalog = TypeCatalog::from_recipe(&recipe);
            let report = core_validate(&recipe, &catalog);
            let issues: Vec<_> = report
                .issues
                .iter()
                .map(|i| {
                    serde_json::json!({
                        "code": format!("{:?}", i.code),
                        "message": i.message,
                        "severity": match i.severity {
                            forage_core::Severity::Error => "error",
                            forage_core::Severity::Warning => "warning",
                        },
                    })
                })
                .collect();
            serde_wasm_bindgen::to_value(&serde_json::json!({
                "ok": true,
                "issues": issues,
                "recipe": recipe,
            }))
            .unwrap_or(JsValue::NULL)
        }
        Err(e) => {
            let span = match &e {
                forage_core::parse::ParseError::UnexpectedToken { span, .. } => Some(span.clone()),
                forage_core::parse::ParseError::Generic { span, .. } => Some(span.clone()),
                _ => None,
            };
            serde_wasm_bindgen::to_value(&serde_json::json!({
                "ok": false,
                "error": {
                    "message": format!("{e}"),
                    "span": span.map(|s| serde_json::json!({ "start": s.start, "end": s.end })),
                },
            }))
            .unwrap_or(JsValue::NULL)
        }
    }
}
