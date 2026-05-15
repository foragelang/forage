//! In-memory document store: URI → source text + parsed AST + diagnostics.
//!
//! Each document is associated with a workspace (discovered via ancestor
//! walk on its file path) so cross-file validation can route through a
//! merged `TypeCatalog`. Workspaces are cached by root so a workspace
//! shared by many open recipes is loaded once and refreshed on edits to
//! its `forage.toml` or any declarations file inside.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use forage_core::ForageFile;
use forage_core::parse::ParseError;
use forage_core::validate;
use forage_core::workspace::{self, TypeCatalog, Workspace, WorkspaceError, WorkspaceFileKind};
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, Url};

use forage_core::LineMap;

use crate::offsets::lsp_range;

pub struct DocStore {
    docs: Mutex<HashMap<Url, Document>>,
    /// Cached workspaces keyed by root path. Documents reference their
    /// workspace by root so a single workspace shared by many open
    /// recipes is parsed once.
    workspaces: Mutex<HashMap<PathBuf, Workspace>>,
}

pub struct Document {
    pub source: String,
    pub line_map: LineMap,
    /// Parsed AST when the buffer parsed cleanly. Present whether or
    /// not the file declares a recipe header — the LSP shows the same
    /// completions for header-less declarations files.
    pub file: Option<ForageFile>,
    pub diagnostics: Vec<Diagnostic>,
    /// Local filesystem path resolved from the document URI, if any.
    /// `None` for `untitled:` or non-file URIs — those validate in
    /// lonely-recipe mode.
    pub path: Option<PathBuf>,
    /// Root of the workspace this document belongs to. `None` when no
    /// `forage.toml` was discovered up the ancestor chain.
    pub workspace_root: Option<PathBuf>,
    /// Whether this file is a recipe or a header-less declarations
    /// file. Cached at upsert time so `did_change` doesn't have to
    /// re-discover the workspace + re-parse on every keystroke. `None`
    /// for non-`file:` URIs that aren't in any workspace.
    pub kind: Option<WorkspaceFileKind>,
}

impl DocStore {
    pub fn new() -> Self {
        Self {
            docs: Mutex::new(HashMap::new()),
            workspaces: Mutex::new(HashMap::new()),
        }
    }

    /// Insert/replace a document and return its diagnostics. Triggers
    /// workspace (re-)discovery if needed.
    pub fn upsert(&self, uri: Url, source: String) -> Vec<Diagnostic> {
        let path = uri.to_file_path().ok();
        let workspace_root = path.as_deref().and_then(workspace::discover).map(|ws| {
            let root = ws.root.clone();
            self.workspaces.lock().unwrap().insert(root.clone(), ws);
            root
        });

        // Snapshot live buffer contents for every other open document
        // in the same workspace so that catalog reads see unsaved edits
        // instead of stale disk content.
        let live_sources = self.live_sources_excluding(&uri, workspace_root.as_ref());
        let doc = self.build(source, path, workspace_root, &live_sources);
        let diagnostics = doc.diagnostics.clone();
        self.docs.lock().unwrap().insert(uri, doc);
        diagnostics
    }

    /// Build a snapshot of `{path -> source}` for every open document
    /// whose URI is a `file:` URL, excluding `skip`. Optionally narrows
    /// to documents in a specific workspace. Paths are canonicalized so
    /// the catalog reader can look them up by the same paths the
    /// workspace stores after `load(...)` canonicalizes them.
    fn live_sources_excluding(
        &self,
        skip: &Url,
        workspace_root: Option<&PathBuf>,
    ) -> HashMap<PathBuf, String> {
        let docs = self.docs.lock().unwrap();
        let mut out = HashMap::new();
        for (uri, doc) in docs.iter() {
            if uri == skip {
                continue;
            }
            let Some(path) = uri.to_file_path().ok() else {
                continue;
            };
            if let Some(root) = workspace_root {
                if doc.workspace_root.as_ref() != Some(root) {
                    continue;
                }
            }
            let key = path.canonicalize().unwrap_or(path);
            out.insert(key, doc.source.clone());
        }
        out
    }

    pub fn remove(&self, uri: &Url) {
        self.docs.lock().unwrap().remove(uri);
    }

    pub fn with<R>(&self, uri: &Url, f: impl FnOnce(&Document) -> R) -> Option<R> {
        self.docs.lock().unwrap().get(uri).map(f)
    }

    /// Force-reload a workspace from disk and re-validate every open
    /// document that belongs to it. Returns the URIs that were
    /// re-validated alongside their fresh diagnostics so the server can
    /// publish them.
    pub fn refresh_workspace(&self, root: &PathBuf) -> Vec<(Url, Vec<Diagnostic>)> {
        let fresh = match workspace::load(root) {
            Ok(ws) => ws,
            Err(_) => return Vec::new(),
        };
        self.workspaces.lock().unwrap().insert(root.clone(), fresh);

        // Collect the set of docs that live in this workspace, then
        // rebuild each. Take a snapshot of (uri, source, path) to avoid
        // holding the docs lock across `build`.
        let snapshot: Vec<(Url, String, Option<PathBuf>)> = {
            let docs = self.docs.lock().unwrap();
            docs.iter()
                .filter(|(_, d)| d.workspace_root.as_ref() == Some(root))
                .map(|(uri, d)| (uri.clone(), d.source.clone(), d.path.clone()))
                .collect()
        };

        let mut out = Vec::with_capacity(snapshot.len());
        for (uri, source, path) in snapshot {
            let live_sources = self.live_sources_excluding(&uri, Some(root));
            let doc = self.build(source, path, Some(root.clone()), &live_sources);
            let diags = doc.diagnostics.clone();
            self.docs.lock().unwrap().insert(uri.clone(), doc);
            out.push((uri, diags));
        }
        out
    }

    /// Documents whose source file lies inside `root`.
    pub fn docs_in_workspace(&self, root: &PathBuf) -> Vec<Url> {
        self.docs
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, d)| d.workspace_root.as_ref() == Some(root))
            .map(|(uri, _)| uri.clone())
            .collect()
    }

    fn build(
        &self,
        source: String,
        path: Option<PathBuf>,
        workspace_root: Option<PathBuf>,
        live_sources: &HashMap<PathBuf, String>,
    ) -> Document {
        let line_map = LineMap::new(&source);
        let workspaces = self.workspaces.lock().unwrap();
        let workspace = workspace_root
            .as_ref()
            .and_then(|root| workspaces.get(root));
        let kind = path.as_deref().and_then(|p| {
            workspace
                .and_then(|ws| ws.files.iter().find(|f| f.path == p))
                .map(|f| f.kind.clone())
        });
        let (file, diagnostics) =
            build_diagnostics(&source, &line_map, workspace, path.as_deref(), live_sources);
        Document {
            source,
            line_map,
            file,
            diagnostics,
            path,
            workspace_root,
            kind,
        }
    }
}

impl Default for DocStore {
    fn default() -> Self {
        Self::new()
    }
}

fn build_diagnostics(
    source: &str,
    line_map: &LineMap,
    workspace: Option<&Workspace>,
    path: Option<&std::path::Path>,
    live_sources: &HashMap<PathBuf, String>,
) -> (Option<ForageFile>, Vec<Diagnostic>) {
    let mut diagnostics = Vec::new();
    let _ = path; // path only matters when validating against catalogs of other files
    let parsed = match forage_core::parse::parse(source) {
        Ok(p) => p,
        Err(e) => {
            diagnostics.push(parse_error_diagnostic(&e, source, line_map));
            return (None, diagnostics);
        }
    };
    let catalog = match build_catalog(&parsed, workspace, live_sources) {
        Ok(c) => c,
        Err(e) => {
            diagnostics.push(workspace_error_diagnostic(&e, line_map));
            return (Some(parsed), diagnostics);
        }
    };
    let report = validate(&parsed, &catalog);
    for issue in &report.issues {
        let severity = match issue.severity {
            forage_core::Severity::Error => DiagnosticSeverity::ERROR,
            forage_core::Severity::Warning => DiagnosticSeverity::WARNING,
        };
        // `0..0` is the validator's convention for "no specific
        // location" (file-wide invariants like engine mismatches);
        // anchor those at the start of the file. Everything else
        // squiggles at the actual construct.
        diagnostics.push(Diagnostic {
            range: lsp_range(line_map, issue.span.clone()),
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
    (Some(parsed), diagnostics)
}

fn build_catalog(
    file: &ForageFile,
    workspace: Option<&Workspace>,
    live_sources: &HashMap<PathBuf, String>,
) -> Result<TypeCatalog, WorkspaceError> {
    // When the document lives in a workspace, route through the
    // workspace catalog so other files contribute their types.
    // Otherwise fall back to file-local — covers untitled buffers and
    // lonely-file mode.
    if let Some(ws) = workspace {
        return ws.catalog(file, |p| {
            if let Some(src) = live_sources.get(p) {
                Ok(src.clone())
            } else {
                std::fs::read_to_string(p)
            }
        });
    }
    Ok(TypeCatalog::from_file(file))
}

fn workspace_error_diagnostic(e: &WorkspaceError, line_map: &LineMap) -> Diagnostic {
    // Span the entire document so the user sees the failure at the
    // file level — workspace errors aren't anchored to a specific
    // token in this buffer.
    Diagnostic {
        range: lsp_range(line_map, 0..line_map.len()),
        severity: Some(DiagnosticSeverity::ERROR),
        source: Some("forage".into()),
        message: format!("{e}"),
        ..Default::default()
    }
}

fn parse_error_diagnostic(e: &ParseError, _source: &str, line_map: &LineMap) -> Diagnostic {
    let (range, msg) = match e {
        ParseError::UnexpectedToken {
            span,
            expected,
            found,
        } => (
            lsp_range(line_map, span.clone()),
            format!("unexpected {found}, expected {expected}"),
        ),
        ParseError::UnexpectedEof { expected } => (
            lsp_range(line_map, 0..0),
            format!("unexpected end of input, expected {expected}"),
        ),
        ParseError::Generic { span, message } => {
            (lsp_range(line_map, span.clone()), message.clone())
        }
        ParseError::InvalidRegex { span, message } => (
            lsp_range(line_map, span.clone()),
            format!("invalid regex: {message}"),
        ),
        ParseError::InvalidRegexFlag { span, flag } => (
            lsp_range(line_map, span.clone()),
            format!("unknown regex flag '{flag}'"),
        ),
        ParseError::Lex(le) => (lsp_range(line_map, 0..0), format!("{le}")),
    };
    Diagnostic {
        range,
        severity: Some(DiagnosticSeverity::ERROR),
        source: Some("forage".into()),
        message: msg,
        ..Default::default()
    }
}

/// Tell the LSP that a declarations file (or `forage.toml`) inside this
/// workspace was edited externally. Used by file-watcher events.
pub fn workspace_root_for(uri: &Url) -> Option<PathBuf> {
    let path = uri.to_file_path().ok()?;
    workspace::discover(&path).map(|ws| ws.root)
}

impl DocStore {
    /// What kind of file does this URI represent, according to the
    /// most recent upsert? `None` for never-seen URIs and for URIs
    /// outside any workspace. Reads from the cached classification on
    /// `Document` so `did_change` doesn't re-scan the workspace tree
    /// on every keystroke.
    pub fn workspace_kind_for(&self, uri: &Url) -> Option<WorkspaceFileKind> {
        self.docs
            .lock()
            .unwrap()
            .get(uri)
            .and_then(|d| d.kind.clone())
    }
}
