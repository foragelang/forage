//! Forage AST types.
//!
//! Filled in during R1.2. Today: minimal `Recipe` placeholder so the workspace builds.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Recipe {
    pub name: String,
}
