//! Top-level recipe shape.

use serde::{Deserialize, Serialize};

use crate::ast::auth::AuthStrategy;
use crate::ast::browser::BrowserConfig;
use crate::ast::expr::{Emission, ExtractionExpr};
use crate::ast::http::HTTPStep;
use crate::ast::span::Span;
use crate::ast::types::{InputDecl, RecipeEnum, RecipeType};

/// One parsed `.forage` file. Either a full `Recipe` (file begins with
/// `recipe "<name>"`) or a header-less `DeclarationsFile` (a sharable
/// type/enum bundle that workspaces fold into the catalog). These two
/// shapes are structurally disjoint: a `Recipe` carries the entire
/// body (steps, for-loops, emits, auth/browser config, expectations),
/// while `DeclarationsFile` carries only types and enums. Boxing the
/// heavyweight variant keeps the discriminator compact without
/// obscuring that asymmetry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkspaceFile {
    Recipe(Box<Recipe>),
    Declarations(DeclarationsFile),
}

/// A header-less `.forage` file: only `type` and `enum` declarations.
/// Inside a workspace these contribute names to the shared
/// `TypeCatalog`; outside one they're meaningless and the loader will
/// reject the workspace if it discovers them in lonely-recipe mode.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct DeclarationsFile {
    #[serde(default)]
    pub types: Vec<RecipeType>,
    #[serde(default)]
    pub enums: Vec<RecipeEnum>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Recipe {
    pub name: String,
    pub engine_kind: EngineKind,
    #[serde(default)]
    pub types: Vec<RecipeType>,
    #[serde(default)]
    pub enums: Vec<RecipeEnum>,
    #[serde(default)]
    pub inputs: Vec<InputDecl>,
    #[serde(default)]
    pub auth: Option<AuthStrategy>,
    #[serde(default)]
    pub body: Vec<Statement>,
    #[serde(default)]
    pub browser: Option<BrowserConfig>,
    #[serde(default)]
    pub expectations: Vec<Expectation>,
    /// Top-level `secret <name>` declarations, in source order.
    #[serde(default)]
    pub secrets: Vec<String>,
    /// Top-level `fn <name>(...)` declarations, in source order. These
    /// are user-defined transforms; the validator and evaluator look
    /// them up before falling back to the built-in registry.
    pub functions: Vec<FnDecl>,
}

impl Recipe {
    pub fn input(&self, name: &str) -> Option<&InputDecl> {
        self.inputs.iter().find(|i| i.name == name)
    }

    pub fn function(&self, name: &str) -> Option<&FnDecl> {
        self.functions.iter().find(|f| f.name == name)
    }
}

/// A user-defined transform — `fn <name>(<$p1>, <$p2>) { <body> }`. The
/// body is a single `ExtractionExpr` from the existing grammar; call
/// sites look identical to built-in transforms.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FnDecl {
    pub name: String,
    /// Parameter names without the leading `$`, in declaration order.
    /// First param is bound to the pipe head at call sites
    /// (`x |> myFn(a)` binds `$p1 = x`, `$p2 = a`).
    pub params: Vec<String>,
    pub body: crate::ast::expr::ExtractionExpr,
    #[serde(default)]
    pub span: crate::ast::span::Span,
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
