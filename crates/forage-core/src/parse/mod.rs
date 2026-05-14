//! Forage parser: tokenize then assemble a `Recipe` (chumsky-based).
//!
//! Top-level entry point: `parse(source: &str) -> (Option<Recipe>, Vec<ParseError>)`.
//! On clean parse, returns `(Some(recipe), vec![])`. On partial parse, returns
//! `(Some(partial), errors)` — chumsky's recovery means we get the best
//! AST we can plus a list of errors with spans for the LSP / CLI diagnostics.

pub mod lexer;
pub mod parser;
pub mod token;

pub use lexer::{LexError, lex};
pub use parser::{ParseError, parse, parse_workspace_file};
pub use token::{KEYWORDS, TYPE_KEYWORDS, Token, is_keyword};
