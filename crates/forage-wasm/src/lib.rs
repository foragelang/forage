//! wasm-bindgen exports of `forage-core` for the hub site's web IDE.
//! Compiled via `wasm-pack build --target web`.

use indexmap::IndexMap;
use serde::Deserialize;
use wasm_bindgen::prelude::*;

use forage_core::ast::Statement;
use forage_core::parse::{KEYWORDS, TYPE_KEYWORDS};
use forage_core::validate::BUILTIN_TRANSFORMS;
use forage_core::{
    EvalValue, LineMap, Snapshot, TypeCatalog, infer_progress_unit, parse as core_parse,
    validate as core_validate,
};
use forage_http::{Engine, ReplayTransport};
use forage_lsp::intel::hover_at;

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
                forage_core::parse::ParseError::InvalidRegex { span, message } => (
                    format!("invalid regex: {message}"),
                    Some((span.start, span.end)),
                ),
                forage_core::parse::ParseError::InvalidRegexFlag { span, flag } => (
                    format!("unknown regex flag '{flag}'"),
                    Some((span.start, span.end)),
                ),
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
    let recipe: forage_core::ForageFile = match serde_json::from_str(recipe_json) {
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
    let catalog = TypeCatalog::from_file(&recipe);
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
            let catalog = TypeCatalog::from_file(&recipe);
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
                forage_core::parse::ParseError::InvalidRegex { span, .. } => Some(span.clone()),
                forage_core::parse::ParseError::InvalidRegexFlag { span, .. } => Some(span.clone()),
                forage_core::parse::ParseError::Lex(_)
                | forage_core::parse::ParseError::UnexpectedEof { .. } => None,
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

/// Step name + 0-based source range. Mirrors `commands::StepLocation`
/// on the Studio side; the hub IDE consumes the same `StepLocation.ts`
/// binding so both surfaces share one canonical shape.
#[derive(serde::Serialize)]
pub struct StepLocation {
    pub name: String,
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
}

/// Pure-Rust core of `recipe_outline`. Returns the list of step
/// locations in source order; empty when parsing fails or the body has
/// no steps. The wasm wrapper just serializes the result.
pub fn recipe_outline_inner(source: &str) -> Vec<StepLocation> {
    let Ok(recipe) = core_parse(source) else {
        return Vec::new();
    };
    let line_map = LineMap::new(source);
    let mut steps = Vec::new();
    collect_step_locations(&recipe.body, &line_map, &mut steps);
    steps
}

/// Structural outline of a recipe — step name + 0-based source range
/// for each `step ...` block, in source order. Mirrors the JSON shape
/// of `apps/studio/src-tauri/src/commands.rs::RecipeOutline` so the
/// hub IDE consumes the same `bindings/RecipeOutline.ts` Studio does.
#[wasm_bindgen]
pub fn recipe_outline(source: &str) -> JsValue {
    let steps = recipe_outline_inner(source);
    serde_wasm_bindgen::to_value(&serde_json::json!({ "steps": steps })).unwrap_or(JsValue::NULL)
}

fn collect_step_locations(body: &[Statement], line_map: &LineMap, out: &mut Vec<StepLocation>) {
    for s in body {
        match s {
            Statement::Step(step) => {
                let r = line_map.range(step.span.clone());
                out.push(StepLocation {
                    name: step.name.clone(),
                    start_line: r.start.line,
                    start_col: r.start.character,
                    end_line: r.end.line,
                    end_col: r.end.character,
                });
            }
            Statement::ForLoop { body, .. } => {
                collect_step_locations(body, line_map, out);
            }
            Statement::Emit(_) => {}
        }
    }
}

/// Snapshot of the language's reserved word + transform inventory.
/// Same shape as `apps/studio/src-tauri/src/commands.rs::LanguageDictionary`
/// so Monaco syntax highlighting / completion can draw from one source
/// in both Studio and the hub IDE.
#[wasm_bindgen]
pub fn language_dictionary() -> JsValue {
    serde_wasm_bindgen::to_value(&serde_json::json!({
        "keywords": KEYWORDS,
        "type_keywords": TYPE_KEYWORDS,
        "transforms": BUILTIN_TRANSFORMS,
    }))
    .unwrap_or(JsValue::NULL)
}

/// Hover info at (line, col), 0-based. Returns the markdown payload
/// when the position is on a recognized identifier (transform / type /
/// input / enum / secret / step name), otherwise `null`. Same shape
/// as `forage_lsp::intel::HoverInfo` since the hub IDE pulls the binding
/// straight out of that crate.
#[wasm_bindgen]
pub fn recipe_hover(source: &str, line: u32, col: u32) -> JsValue {
    match hover_at(source, line, col) {
        Some(info) => serde_wasm_bindgen::to_value(&info).unwrap_or(JsValue::NULL),
        None => JsValue::NULL,
    }
}

/// Infer the progress unit (deepest emit-bearing for-loop scope) for
/// the recipe source. Returns the `ProgressUnit` JSON shape or `null`
/// on parse failure or when no emit-bearing loop exists. The hub IDE
/// scopes the run pane's progress bar to this unit's record types.
#[wasm_bindgen]
pub fn recipe_progress_unit(source: &str) -> JsValue {
    let Ok(recipe) = core_parse(source) else {
        return JsValue::NULL;
    };
    match infer_progress_unit(&recipe) {
        Some(unit) => serde_wasm_bindgen::to_value(&unit).unwrap_or(JsValue::NULL),
        None => JsValue::NULL,
    }
}

/// One declarations file shipped alongside the recipe. The hub IDE
/// passes this in via JS; in Studio terms it's the "decls" tab of the
/// package version artifact.
#[derive(Deserialize)]
pub struct DeclFile {
    /// In-package path (e.g. `cannabis.forage`). Used in error
    /// messages if the file fails to parse.
    pub name: String,
    /// UTF-8 source of the declarations file.
    pub source: String,
}

/// Engine-side errors surfaced to the caller of `run_replay_inner`.
/// JS gets a flat string (`JsValue`); the integration test asserts
/// against a real enum.
#[derive(Debug)]
pub enum ReplayError {
    Parse(String),
    Decl { name: String, message: String },
    NotADeclFile { name: String },
    Validation(Vec<String>),
    Captures(String),
    Run(String),
}

impl std::fmt::Display for ReplayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReplayError::Parse(m) => write!(f, "parse: {m}"),
            ReplayError::Decl { name, message } => write!(f, "decls {name}: {message}"),
            ReplayError::NotADeclFile { name } => write!(
                f,
                "decls {name}: file has a recipe header, not declarations"
            ),
            ReplayError::Validation(messages) => {
                write!(f, "validation failed:\n  {}", messages.join("\n  "))
            }
            ReplayError::Captures(m) => write!(f, "captures: {m}"),
            ReplayError::Run(m) => write!(f, "run: {m}"),
        }
    }
}

/// The core of `run_replay`, in pure Rust. JS code goes through the
/// `#[wasm_bindgen]` wrapper below; the integration test exercises this
/// directly so we can run on a native tokio runtime.
pub async fn run_replay_inner(
    recipe_source: &str,
    decl_files: &[DeclFile],
    captures_jsonl: &str,
    inputs: IndexMap<String, EvalValue>,
    secrets: IndexMap<String, String>,
) -> Result<Snapshot, ReplayError> {
    let recipe = core_parse(recipe_source).map_err(|e| ReplayError::Parse(e.to_string()))?;

    // Catalog merge order matches Workspace::catalog: workspace-level
    // decl files first, recipe-local last so the recipe shadows
    // anything that collides by name.
    //
    // TODO(typed-hub): like the hub-cache path in workspace::catalog,
    // wasm replay receives decl-file contents without `share` markers
    // and treats every type/enum as visible. The typed-hub program
    // will make exports explicit on this entry point too.
    let mut catalog = TypeCatalog::default();
    for f in decl_files {
        let parsed = core_parse(&f.source).map_err(|e| ReplayError::Decl {
            name: f.name.clone(),
            message: e.to_string(),
        })?;
        if parsed.recipe_header().is_some() {
            return Err(ReplayError::NotADeclFile {
                name: f.name.clone(),
            });
        }
        catalog.merge_all(&parsed);
    }
    catalog.merge_all(&recipe);

    let report = core_validate(&recipe, &catalog);
    if report.has_errors() {
        let messages: Vec<String> = report
            .issues
            .iter()
            .filter(|i| matches!(i.severity, forage_core::Severity::Error))
            .map(|i| format!("{:?}: {}", i.code, i.message))
            .collect();
        return Err(ReplayError::Validation(messages));
    }

    let transport =
        ReplayTransport::from_jsonl(captures_jsonl).map_err(|e| ReplayError::Captures(e.to_string()))?;
    let engine = Engine::new(&transport);
    engine
        .run(&recipe, inputs, secrets)
        .await
        .map_err(|e| ReplayError::Run(e.to_string()))
}

/// Replay a recipe in the browser. Inputs:
///
/// - `recipe_source` — the recipe's `.forage` text.
/// - `decls` — JS array of `{ name, source }`; each `source` is a
///   header-less declarations file whose types/enums merge into the
///   catalog the validator consults.
/// - `captures_jsonl` — one capture per line, the same `Capture`
///   serialization Studio writes to disk.
/// - `inputs` — JSON object of recipe input values. Keys must match the
///   recipe's `input <name>: <Type>` declarations.
/// - `secrets` — JSON object of `{name: string}`. Recipe secrets are
///   substituted from this map; absence is surfaced by the engine.
///
/// Returns the run snapshot as a JS object on success. Errors throw as
/// JS exceptions with the engine's error string.
#[wasm_bindgen]
pub async fn run_replay(
    recipe_source: &str,
    decls: JsValue,
    captures_jsonl: &str,
    inputs: JsValue,
    secrets: JsValue,
) -> Result<JsValue, JsValue> {
    let decl_files: Vec<DeclFile> = if decls.is_undefined() || decls.is_null() {
        Vec::new()
    } else {
        serde_wasm_bindgen::from_value(decls)
            .map_err(|e| JsValue::from_str(&format!("decls: {e}")))?
    };

    let input_map: IndexMap<String, serde_json::Value> = if inputs.is_undefined() || inputs.is_null() {
        IndexMap::new()
    } else {
        serde_wasm_bindgen::from_value(inputs)
            .map_err(|e| JsValue::from_str(&format!("inputs: {e}")))?
    };
    let input_eval: IndexMap<String, EvalValue> = input_map
        .into_iter()
        .map(|(k, v)| (k, EvalValue::from(&v)))
        .collect();

    let secret_map: IndexMap<String, String> = if secrets.is_undefined() || secrets.is_null() {
        IndexMap::new()
    } else {
        serde_wasm_bindgen::from_value(secrets)
            .map_err(|e| JsValue::from_str(&format!("secrets: {e}")))?
    };

    let snapshot =
        run_replay_inner(recipe_source, &decl_files, captures_jsonl, input_eval, secret_map)
            .await
            .map_err(|e| JsValue::from_str(&e.to_string()))?;
    serde_wasm_bindgen::to_value(&snapshot)
        .map_err(|e| JsValue::from_str(&format!("snapshot encode: {e}")))
}
