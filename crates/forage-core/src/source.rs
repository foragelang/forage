//! Utilities for navigating recipe source text alongside the AST.
//!
//! Spans on AST nodes (added in `ast::span::Span`) are byte ranges. UIs
//! and editors typically want line/column. `LineMap` is the common
//! conversion utility, shared by `forage-lsp` (for LSP `Range` payloads)
//! and Studio (for surfacing precise positions in TS).
//!
//! Lines and columns are 0-based. Column counts UTF-8 bytes, not
//! grapheme clusters or UTF-16 code units — the LSP's UTF-16 contract is
//! the LSP layer's concern. Recipes are almost always ASCII so the
//! distinction rarely matters in practice.

use serde::{Deserialize, Serialize};

/// 0-based (line, column) position in a source buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Position {
    pub line: u32,
    pub character: u32,
}

/// 0-based start/end positions; half-open, matching LSP's Range.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Range {
    pub start: Position,
    pub end: Position,
}

/// Precomputed line-start byte offsets for a source buffer. Construct
/// once per document edit, then query repeatedly.
#[derive(Clone, Debug)]
pub struct LineMap {
    line_starts: Vec<usize>,
    len: usize,
}

impl LineMap {
    pub fn new(source: &str) -> Self {
        let mut starts = vec![0usize];
        for (i, b) in source.bytes().enumerate() {
            if b == b'\n' {
                starts.push(i + 1);
            }
        }
        Self {
            line_starts: starts,
            len: source.len(),
        }
    }

    pub fn position(&self, offset: usize) -> Position {
        let offset = offset.min(self.len);
        let line = match self.line_starts.binary_search(&offset) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        };
        let line_start = self.line_starts[line];
        Position {
            line: line as u32,
            character: (offset - line_start) as u32,
        }
    }

    pub fn range(&self, span: std::ops::Range<usize>) -> Range {
        Range {
            start: self.position(span.start),
            end: self.position(span.end),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn position_resolves_lines_and_columns() {
        let src = "line1\nline two\nthird";
        let lm = LineMap::new(src);
        assert_eq!(lm.position(0), Position { line: 0, character: 0 });
        assert_eq!(lm.position(5), Position { line: 0, character: 5 });
        assert_eq!(lm.position(6), Position { line: 1, character: 0 });
        assert_eq!(lm.position(15), Position { line: 2, character: 0 });
    }

    #[test]
    fn range_covers_a_span() {
        let src = "abc\ndefgh\nij";
        let lm = LineMap::new(src);
        let r = lm.range(4..9);
        assert_eq!(r.start, Position { line: 1, character: 0 });
        assert_eq!(r.end, Position { line: 1, character: 5 });
    }

    #[test]
    fn offset_past_end_clamps() {
        let src = "abc";
        let lm = LineMap::new(src);
        assert_eq!(lm.position(99), Position { line: 0, character: 3 });
    }
}
