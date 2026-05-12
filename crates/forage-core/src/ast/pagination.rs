//! HTTP-engine pagination strategies.
//!
//! Browser-engine pagination is in `ast::browser`.

use serde::{Deserialize, Serialize};

use crate::ast::expr::PathExpr;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Pagination {
    /// Send a page-number param; response carries `items` + `total`. Stop
    /// when accumulated items ≥ total. (Sweed.)
    PageWithTotal {
        items_path: PathExpr,
        total_path: PathExpr,
        page_param: String,
        page_size: u32,
        #[serde(default)]
        page_zero_indexed: bool,
    },
    /// Send a page-number param; response carries `items`. Stop when items
    /// shorter than page-size or empty. (Leafbridge.)
    UntilEmpty {
        items_path: PathExpr,
        page_param: String,
        #[serde(default)]
        page_zero_indexed: bool,
    },
    /// Server returns a continuation token; empty/nil cursor terminates.
    Cursor {
        items_path: PathExpr,
        cursor_path: PathExpr,
        cursor_param: String,
    },
}
