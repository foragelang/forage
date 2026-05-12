//! In-memory document store: URI → source text + parsed AST + diagnostics.

use std::collections::HashMap;
use std::sync::Mutex;

use forage_core::Recipe;
use forage_core::parse::ParseError;
use forage_core::{parse, validate};
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, Url};

use crate::offsets::LineMap;

pub struct DocStore {
    docs: Mutex<HashMap<Url, Document>>,
}

pub struct Document {
    pub source: String,
    pub line_map: LineMap,
    pub recipe: Option<Recipe>,
    pub diagnostics: Vec<Diagnostic>,
}

impl Document {
    fn build(source: String) -> Self {
        let line_map = LineMap::new(&source);
        let (recipe, diagnostics) = build_diagnostics(&source, &line_map);
        Self {
            source,
            line_map,
            recipe,
            diagnostics,
        }
    }
}

impl DocStore {
    pub fn new() -> Self {
        Self {
            docs: Mutex::new(HashMap::new()),
        }
    }

    pub fn upsert(&self, uri: Url, source: String) -> Vec<Diagnostic> {
        let doc = Document::build(source);
        let diagnostics = doc.diagnostics.clone();
        self.docs.lock().unwrap().insert(uri, doc);
        diagnostics
    }

    pub fn remove(&self, uri: &Url) {
        self.docs.lock().unwrap().remove(uri);
    }

    pub fn with<R>(&self, uri: &Url, f: impl FnOnce(&Document) -> R) -> Option<R> {
        self.docs.lock().unwrap().get(uri).map(f)
    }
}

impl Default for DocStore {
    fn default() -> Self {
        Self::new()
    }
}

fn build_diagnostics(source: &str, line_map: &LineMap) -> (Option<Recipe>, Vec<Diagnostic>) {
    let mut diagnostics = Vec::new();
    let recipe = match parse(source) {
        Ok(r) => Some(r),
        Err(e) => {
            diagnostics.push(parse_error_diagnostic(&e, source, line_map));
            None
        }
    };
    if let Some(r) = &recipe {
        let report = validate(r);
        for issue in &report.issues {
            let severity = match issue.severity {
                forage_core::Severity::Error => DiagnosticSeverity::ERROR,
                forage_core::Severity::Warning => DiagnosticSeverity::WARNING,
            };
            // The validator doesn't carry spans yet, so we anchor each
            // issue at the start of the recipe. R7 followup will thread
            // spans through the validator.
            diagnostics.push(Diagnostic {
                range: line_map.range_for(0..0),
                severity: Some(severity),
                code: Some(tower_lsp::lsp_types::NumberOrString::String(format!(
                    "{:?}",
                    issue.code
                ))),
                source: Some("forage".into()),
                message: issue.message.clone(),
                ..Default::default()
            });
        }
    }
    (recipe, diagnostics)
}

fn parse_error_diagnostic(e: &ParseError, _source: &str, line_map: &LineMap) -> Diagnostic {
    let (range, msg) = match e {
        ParseError::UnexpectedToken {
            span,
            expected,
            found,
        } => (
            line_map.range_for(span.clone()),
            format!("unexpected {found}, expected {expected}"),
        ),
        ParseError::UnexpectedEof { expected } => (
            line_map.range_for(0..0),
            format!("unexpected end of input, expected {expected}"),
        ),
        ParseError::Generic { span, message } => {
            (line_map.range_for(span.clone()), message.clone())
        }
        ParseError::Lex(le) => (line_map.range_for(0..0), format!("{le}")),
    };
    Diagnostic {
        range,
        severity: Some(DiagnosticSeverity::ERROR),
        source: Some("forage".into()),
        message: msg,
        ..Default::default()
    }
}
