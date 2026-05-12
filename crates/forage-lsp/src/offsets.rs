//! Convert byte offsets in source text to LSP line / character positions
//! and vice versa. LSP uses UTF-16 code units; we approximate with UTF-8
//! code units which matches for ASCII-only documents (essentially all
//! `.forage` files). The shim can swap to a proper UTF-16 mapping if
//! that becomes necessary.

use tower_lsp::lsp_types::{Position, Range};

#[derive(Clone, Debug)]
pub struct LineMap {
    /// Byte offsets where each line starts.
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

    pub fn offset_to_position(&self, offset: usize) -> Position {
        let offset = offset.min(self.len);
        let line = match self.line_starts.binary_search(&offset) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        };
        let line_start = self.line_starts[line];
        let character = offset - line_start;
        Position {
            line: line as u32,
            character: character as u32,
        }
    }

    pub fn range_for(&self, span: std::ops::Range<usize>) -> Range {
        Range {
            start: self.offset_to_position(span.start),
            end: self.offset_to_position(span.end),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_resolve_lines_and_columns() {
        let src = "line1\nline two\nthird";
        let lm = LineMap::new(src);
        assert_eq!(
            lm.offset_to_position(0),
            Position {
                line: 0,
                character: 0
            }
        );
        assert_eq!(
            lm.offset_to_position(5),
            Position {
                line: 0,
                character: 5
            }
        );
        assert_eq!(
            lm.offset_to_position(6),
            Position {
                line: 1,
                character: 0
            }
        );
        assert_eq!(
            lm.offset_to_position(15),
            Position {
                line: 2,
                character: 0
            }
        );
    }
}
