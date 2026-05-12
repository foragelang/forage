//! Forage Language Server — built on `tower-lsp`. Reuses `forage-core`
//! for parsing + validation; surfaces diagnostics, completion, hover,
//! goto-def, document symbols, formatting.

pub mod docstore;
pub mod offsets;
pub mod server;

pub use server::ForageLsp;
