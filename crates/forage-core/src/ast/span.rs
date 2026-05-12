//! Source spans.
//!
//! Chumsky 0.10 uses `Range<usize>` natively, so `Span` is a type alias for
//! it. Line/column resolution from a span happens at the diagnostic
//! rendering layer (ariadne for CLI, LSP `Range` for editors) — the AST
//! itself stays byte-offset-oriented.

pub type Span = std::ops::Range<usize>;
