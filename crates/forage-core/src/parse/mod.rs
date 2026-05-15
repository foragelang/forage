//! Forage parser: tokenize then assemble a `ForageFile`.
//!
//! Top-level entry point: `parse(source: &str) -> Result<ForageFile, ParseError>`.
//! The parser accepts any well-formed sequence of top-level forms;
//! semantic constraints (recipe header uniqueness, recipe-context forms
//! requiring a header) live in the validator.

pub mod lexer;
pub mod parser;
pub mod token;

pub use lexer::{LexError, lex};
pub use parser::{ParseError, parse, parse_extraction};
pub use token::{KEYWORDS, TYPE_KEYWORDS, Token, is_keyword};
