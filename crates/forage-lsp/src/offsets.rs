//! Bridge between forage-core's plain `LineMap` and the LSP's `Range`.
//!
//! The line/column resolution itself lives in `forage_core::source` so
//! Studio (Tauri commands) and the LSP share the same logic. This module
//! is just the LSP-specific projection: forage-core `Position` → LSP
//! `Position`, forage-core `Range` → LSP `Range`.

use forage_core::source::LineMap;
use tower_lsp::lsp_types::{Position, Range};

/// LSP-flavored wrapper around `forage_core::LineMap` with conversion to
/// `tower_lsp::lsp_types::Range`.
pub fn lsp_range(lm: &LineMap, span: std::ops::Range<usize>) -> Range {
    let r = lm.range(span);
    Range {
        start: Position {
            line: r.start.line,
            character: r.start.character,
        },
        end: Position {
            line: r.end.line,
            character: r.end.character,
        },
    }
}
