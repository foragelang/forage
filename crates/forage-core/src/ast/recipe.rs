//! Top-level file shape: `ForageFile`, `RecipeHeader`, statements, expectations.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::ast::auth::AuthStrategy;
use crate::ast::browser::BrowserConfig;
use crate::ast::expr::{Emission, ExtractionExpr};
use crate::ast::http::HTTPStep;
use crate::ast::span::Span;
use crate::ast::types::{EmitsDecl, InputDecl, RecipeEnum, RecipeType};

/// One parsed `.forage` file. The grammar is flat — a file is a sequence
/// of top-level forms (`recipe`, `type`, `enum`, `input`, `secret`, `fn`,
/// `auth`, `browser`, `expect`, statements). The parser groups them into
/// the slots below regardless of source order.
///
/// `recipe_headers` collects every `recipe "<name>" engine <kind>` opener
/// the parser sees. A well-formed file has exactly one (declaring a
/// recipe) or zero (a pure declarations file). The validator emits
/// `DuplicateRecipeHeader` when there are two or more, and
/// `RecipeContextWithoutHeader` when recipe-context forms (auth,
/// browser, expect, statements) appear in a header-less file.
///
/// `body` is the recipe's body: either a sequence of scraping
/// statements (`step` / `for` / `emit`) or a composition expression
/// (`compose A | B | …`). Composition is itself a recipe body kind; a
/// composed recipe shares the same header, the same publishable shape,
/// and the same lifecycle as a scraping recipe — there is no separate
/// `pipeline` citizen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ForageFile {
    pub recipe_headers: Vec<RecipeHeader>,
    pub types: Vec<RecipeType>,
    pub enums: Vec<RecipeEnum>,
    pub inputs: Vec<InputDecl>,
    /// Optional `emits T` / `emits T1 | T2 | …` clause declaring the
    /// types this recipe is contracted to emit. `None` for header-less
    /// files and for recipes that omit the clause; in the latter case
    /// the runtime shape is whatever the body's `emit` statements
    /// produce and the validator skips the declared-vs-actual check.
    pub emits: Option<EmitsDecl>,
    /// Top-level `secret <name>` declarations, in source order.
    pub secrets: Vec<String>,
    /// Top-level `fn <name>(...)` declarations, in source order. These are
    /// user-defined transforms; the validator and evaluator look them up
    /// before falling back to the built-in registry.
    pub functions: Vec<FnDecl>,
    pub auth: Option<AuthStrategy>,
    pub browser: Option<BrowserConfig>,
    pub body: RecipeBody,
    pub expectations: Vec<Expectation>,
    /// Original source text the file was parsed from. Carried so the
    /// engine and debugger can resolve byte-spans to (line, col)
    /// without callers having to thread the source separately. Skipped
    /// during serialization — `source` is a parser artifact attached
    /// to the in-memory AST, not part of the canonical wire shape; any
    /// AST that round-trips through JSON loses it on the way out and
    /// gets `""` back on the way in. Callers that need the source on a
    /// deserialized file re-attach it explicitly.
    ///
    /// `Arc<str>` because every clone of the AST is read-only and the
    /// source is large enough to want to share. A hand-constructed
    /// `ForageFile` (or one round-tripped through JSON) has
    /// `source = ""`, which means `LineMap::new(&forage_file.source)`
    /// gives a degenerate map: every span resolves to `(line=0, col=0)`.
    /// Tests that exercise the debugger should use `parse(&src)`.
    #[serde(skip, default = "empty_arc_str")]
    pub source: Arc<str>,
}

fn empty_arc_str() -> Arc<str> {
    Arc::from("")
}

impl ForageFile {
    pub fn input(&self, name: &str) -> Option<&InputDecl> {
        self.inputs.iter().find(|i| i.name == name)
    }

    /// Every type the recipe may emit, derived from
    /// `Statement::Emit` (top-level and in `for`-loops), nested
    /// `ExtractionExpr::MapTo { emission }` inside extraction
    /// expressions, and the browser config's capture / document-
    /// capture bodies. The single canonical walker — the validator's
    /// emit-vs-`emits` cross-check and the daemon's `derive_schema`
    /// both go through this.
    ///
    /// Empty for composition and for header-less files (neither carries
    /// `emit` statements of their own). For composition recipes the
    /// "what does this recipe emit" question is answered by
    /// `resolved_output_types`, which falls back to the declared
    /// `emits` clause.
    pub fn emit_types(&self) -> std::collections::BTreeSet<String> {
        let mut out = std::collections::BTreeSet::new();
        collect_body_emit_types(&self.body, &mut out);
        if let Some(b) = &self.browser {
            for cap in &b.captures {
                collect_statements_emit_types(&cap.body, &mut out);
            }
            if let Some(doc) = &b.document_capture {
                collect_statements_emit_types(&doc.body, &mut out);
            }
        }
        out
    }

    /// The "what types does this recipe emit" projection used by
    /// every caller that asks the question at recipe granularity:
    /// `RecipeSignature::from_file`, the Studio
    /// `parse_recipe_signature` wire, and the hub publish flow's
    /// input/output role partition. Declared `emits` wins when the
    /// source supplies one; otherwise inferred from the body via
    /// `emit_types`. The two projections must stay in lockstep —
    /// inconsistency between this and `emit_types` is what produced
    /// the bug where hub composition recipes silently dropped out of
    /// type-filtered pickers.
    pub fn resolved_output_types(&self) -> std::collections::BTreeSet<String> {
        match &self.emits {
            Some(decl) => decl.types.iter().cloned().collect(),
            None => self.emit_types(),
        }
    }

    pub fn function(&self, name: &str) -> Option<&FnDecl> {
        self.functions.iter().find(|f| f.name == name)
    }

    /// The recipe header, when the file has one. Validator-clean files
    /// have at most one header; callers that ran the validator first can
    /// rely on that. Returns the first header when several are present
    /// (the validator's `DuplicateRecipeHeader` rule will have surfaced
    /// the duplicates).
    pub fn recipe_header(&self) -> Option<&RecipeHeader> {
        self.recipe_headers.first()
    }

    /// Convenience: the recipe name from the header. `None` for
    /// header-less files.
    pub fn recipe_name(&self) -> Option<&str> {
        self.recipe_header().map(|h| h.name.as_str())
    }

    /// Convenience: the engine kind from the header. `None` for
    /// header-less files.
    pub fn engine_kind(&self) -> Option<EngineKind> {
        self.recipe_header().map(|h| h.engine_kind)
    }
}

/// The `recipe "<name>" engine <kind>` opener. A file has at most one;
/// without it, the file is a pure declarations file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecipeHeader {
    pub name: String,
    pub engine_kind: EngineKind,
    #[serde(default)]
    pub span: Span,
}

/// A user-defined transform — `fn <name>(<$p1>, <$p2>) { <body> }`.
/// The body is a sequence of `let` bindings followed by exactly one
/// trailing expression that is the function's return value. Call sites
/// look identical to built-in transforms.
///
/// `shared = true` (the `share fn …` prefix) makes the fn visible to
/// every other file in the workspace. Without it, the fn is file-scoped.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FnDecl {
    pub name: String,
    /// Parameter names without the leading `$`, in declaration order.
    /// First param is bound to the pipe head at call sites
    /// (`x |> myFn(a)` binds `$p1 = x`, `$p2 = a`).
    pub params: Vec<String>,
    pub body: FnBody,
    pub shared: bool,
    #[serde(default)]
    pub span: crate::ast::span::Span,
}

/// A `fn` body: zero or more `let` bindings followed by a single trailing
/// expression. Each binding adds to the function-local scope; later
/// bindings see earlier ones; the trailing expression sees them all.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FnBody {
    pub bindings: Vec<LetBinding>,
    pub result: crate::ast::expr::ExtractionExpr,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LetBinding {
    pub name: String,
    pub value: crate::ast::expr::ExtractionExpr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EngineKind {
    Http,
    Browser,
}

/// One body statement. Recipes mix steps, emissions, and for-loops at any
/// level of nesting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Statement {
    Step(HTTPStep),
    Emit(Emission),
    ForLoop {
        variable: String,
        collection: ExtractionExpr,
        body: Vec<Statement>,
        /// Source range covering the whole `for $v in … { … }` construct.
        #[serde(default)]
        span: Span,
    },
}

impl Statement {
    /// Source range of this statement. For `Step` / `Emit` this is the
    /// inner node's span; for `ForLoop` it's the explicit `span` field.
    pub fn span(&self) -> &Span {
        match self {
            Statement::Step(s) => &s.span,
            Statement::Emit(e) => &e.span,
            Statement::ForLoop { span, .. } => span,
        }
    }
}

/// A recipe's body. Two kinds:
///
/// - `Scraping`: a sequence of `step` / `for` / `emit` statements that
///   drive the HTTP or browser engine. The historical recipe shape.
/// - `Composition`: a chain of recipe references joined by `|`. The
///   runtime invokes each referenced recipe in turn, feeding the
///   records emitted by stage N as the input to stage N+1.
///
/// `Empty` is the header-less / declarations-only file case and the
/// transient pre-body state. The validator's
/// `RecipeContextWithoutHeader` rule already covers header-less files
/// with body content; `Empty` keeps the AST honest about what's
/// absent.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum RecipeBody {
    #[default]
    Empty,
    Scraping(Vec<Statement>),
    Composition(Composition),
}

impl RecipeBody {
    /// Statements when the body is scraping; empty slice otherwise.
    /// Callers that iterate over steps / emits accept the empty case as
    /// "this body has no statements to walk."
    pub fn statements(&self) -> &[Statement] {
        match self {
            RecipeBody::Scraping(s) => s,
            RecipeBody::Empty | RecipeBody::Composition(_) => &[],
        }
    }

    /// Composition when the body is one; `None` otherwise.
    pub fn composition(&self) -> Option<&Composition> {
        match self {
            RecipeBody::Composition(c) => Some(c),
            _ => None,
        }
    }
}

/// Walk a `RecipeBody` and accumulate every type referenced by an
/// `emit X { … }`, including emits nested inside `ExtractionExpr::MapTo`
/// inside binding expressions. A no-op on `Empty` and `Composition`
/// bodies, both of which carry zero emit statements of their own.
pub fn collect_body_emit_types(body: &RecipeBody, out: &mut std::collections::BTreeSet<String>) {
    if let RecipeBody::Scraping(stmts) = body {
        collect_statements_emit_types(stmts, out);
    }
}

fn collect_statements_emit_types(
    stmts: &[Statement],
    out: &mut std::collections::BTreeSet<String>,
) {
    for s in stmts {
        match s {
            Statement::Emit(em) => collect_emission_emit_types(em, out),
            Statement::ForLoop { body, .. } => collect_statements_emit_types(body, out),
            Statement::Step(_) => {}
        }
    }
}

fn collect_emission_emit_types(em: &Emission, out: &mut std::collections::BTreeSet<String>) {
    out.insert(em.type_name.clone());
    for binding in &em.bindings {
        collect_expr_emit_types(&binding.expr, out);
    }
}

fn collect_expr_emit_types(expr: &ExtractionExpr, out: &mut std::collections::BTreeSet<String>) {
    match expr {
        ExtractionExpr::Pipe(inner, calls) => {
            collect_expr_emit_types(inner, out);
            for c in calls {
                for a in &c.args {
                    collect_expr_emit_types(a, out);
                }
            }
        }
        ExtractionExpr::CaseOf { branches, .. } => {
            for (_, arm) in branches {
                collect_expr_emit_types(arm, out);
            }
        }
        ExtractionExpr::MapTo { emission, .. } => collect_emission_emit_types(emission, out),
        ExtractionExpr::Call { args, .. } => {
            for a in args {
                collect_expr_emit_types(a, out);
            }
        }
        ExtractionExpr::BinaryOp { lhs, rhs, .. } => {
            collect_expr_emit_types(lhs, out);
            collect_expr_emit_types(rhs, out);
        }
        ExtractionExpr::Unary { operand, .. } => collect_expr_emit_types(operand, out),
        ExtractionExpr::StructLiteral { fields } => {
            for f in fields {
                collect_expr_emit_types(&f.expr, out);
            }
        }
        ExtractionExpr::Index { base, index } => {
            collect_expr_emit_types(base, out);
            collect_expr_emit_types(index, out);
        }
        ExtractionExpr::Path(_)
        | ExtractionExpr::Template(_)
        | ExtractionExpr::Literal(_)
        | ExtractionExpr::RegexLiteral(_) => {}
    }
}

/// A composition body: `compose <ref> ( '|' <ref> )+`. Stages are
/// recipe references resolved at validate time against the workspace's
/// recipe catalog. The runtime walks the chain in order, feeding the
/// records emitted by stage N as the input to stage N+1.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Composition {
    pub stages: Vec<RecipeRef>,
    /// Source range covering the `compose` keyword through the last
    /// stage reference.
    #[serde(default)]
    pub span: Span,
}

/// One stage in a composition: a recipe reference. Bare names
/// (`scrape-amazon`) resolve to workspace-local recipes; namespaced
/// references (`@author/recipe-name`) resolve to hub-dep recipes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecipeRef {
    /// Hub author when the reference is namespaced (`@author/name`);
    /// `None` for workspace-local references.
    pub author: Option<String>,
    /// Recipe name as it appears in the source.
    pub name: String,
    /// Source range covering the reference (`scrape-amazon` or
    /// `@author/name`).
    #[serde(default)]
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Expectation {
    pub kind: ExpectationKind,
    /// Byte range of the `expect { … }` block in the recipe source.
    /// Used by `Snapshot::evaluate_expectations` to attach a source
    /// line to every unmet-expectation diagnostic so UIs can jump to it.
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ExpectationKind {
    /// `records.where(typeName == "X").count <op> N`
    RecordCount {
        type_name: String,
        op: ComparisonOp,
        value: i64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ComparisonOp {
    #[serde(rename = ">=")]
    Ge,
    #[serde(rename = ">")]
    Gt,
    #[serde(rename = "<=")]
    Le,
    #[serde(rename = "<")]
    Lt,
    #[serde(rename = "==")]
    Eq,
    #[serde(rename = "!=")]
    Ne,
}
