//! Top-level file shape: `ForageFile`, `RecipeHeader`, statements, expectations.

use serde::{Deserialize, Serialize};

use crate::ast::auth::AuthStrategy;
use crate::ast::browser::BrowserConfig;
use crate::ast::expr::{Emission, ExtractionExpr};
use crate::ast::http::HTTPStep;
use crate::ast::span::Span;
use crate::ast::types::{InputDecl, OutputDecl, RecipeEnum, RecipeType};

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
    /// Recipe output signature, when declared (`output T` or
    /// `output T1 | T2 | …`). `None` for header-less files and for
    /// recipes that haven't been migrated to a typed output yet; the
    /// validator's emit-vs-output check only fires when this is `Some`.
    pub output: Option<OutputDecl>,
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
}

impl ForageFile {
    pub fn input(&self, name: &str) -> Option<&InputDecl> {
        self.inputs.iter().find(|i| i.name == name)
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
