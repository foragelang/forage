//! Forage Language Server — built on `tower-lsp`. Reuses `forage-core`
//! for parsing + validation; surfaces diagnostics, completion, hover,
//! goto-def, document symbols, formatting.
//!
//! The `intel` module is the host-friendly entry point: pure functions
//! that take a source string + position and return JSON-friendly
//! results, no LSP protocol types. Studio's Tauri commands call it
//! directly; the LSP server in `server` wraps the same functions in
//! LSP `Hover` / `CompletionItem` shapes for editors that talk LSP.

pub mod docstore;
pub mod intel;
pub mod offsets;
pub mod server;

pub use server::ForageLsp;
