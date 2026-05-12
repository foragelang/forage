//! Top-level recipe shape.

use serde::{Deserialize, Serialize};

use crate::ast::auth::AuthStrategy;
use crate::ast::browser::BrowserConfig;
use crate::ast::expr::{Emission, ExtractionExpr};
use crate::ast::http::HTTPStep;
use crate::ast::span::Span;
use crate::ast::types::{InputDecl, RecipeEnum, RecipeType};

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
    /// Top-level `import <ref>` directives in source order.
    #[serde(default)]
    pub imports: Vec<HubRecipeRef>,
    /// Top-level `secret <name>` declarations, in source order.
    #[serde(default)]
    pub secrets: Vec<String>,
}

impl Recipe {
    pub fn ty(&self, name: &str) -> Option<&RecipeType> {
        self.types.iter().find(|t| t.name == name)
    }
    pub fn recipe_enum(&self, name: &str) -> Option<&RecipeEnum> {
        self.enums.iter().find(|e| e.name == name)
    }
    pub fn input(&self, name: &str) -> Option<&InputDecl> {
        self.inputs.iter().find(|i| i.name == name)
    }
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

/// Unresolved pointer to a recipe on the hub — `hub://author/slug` or
/// `hub://author/slug@v3`. `forage-hub::importer` resolves these.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HubRecipeRef {
    pub author: String,
    pub slug: String,
    #[serde(default)]
    pub version: Option<u32>,
}
