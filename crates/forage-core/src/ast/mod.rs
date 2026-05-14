//! Forage recipe AST.
//!
//! Mirrors the Swift `Sources/Forage/Recipe/*.swift` types in idiomatic Rust.
//! Recipes are pure data — no engine logic, no I/O. The parser produces an
//! AST; the validator checks it; the evaluator interprets it; the engines
//! (forage-http, forage-browser) execute it.

mod auth;
mod browser;
mod expr;
mod http;
mod json;
mod pagination;
mod recipe;
mod span;
mod types;

pub use auth::{
    AuthStrategy, BearerLogin, CookieFormat, CookiePersist, FormLogin, HtmlPrimeVar, SessionAuth,
    SessionKind,
};
pub use browser::{
    AgeGateConfig, BrowserConfig, BrowserPaginateUntil, BrowserPaginationConfig,
    BrowserPaginationMode, CaptureRule, DismissalConfig, DocumentCaptureRule, InteractiveConfig,
};
pub use expr::{
    Emission, ExtractionExpr, FieldBinding, PathExpr, Template, TemplatePart, TransformCall,
};
pub use http::{BodyValue, HTTPBody, HTTPBodyKV, HTTPRequest, HTTPStep, RegexExtract};
pub use json::JSONValue;
pub use pagination::Pagination;
pub use recipe::{
    ComparisonOp, DeclarationsFile, EngineKind, Expectation, ExpectationKind, FnDecl, Recipe,
    Statement, WorkspaceFile,
};
pub use span::Span;
pub use types::{FieldType, InputDecl, RecipeEnum, RecipeField, RecipeType};
