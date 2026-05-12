//! wasm-bindgen exports of forage-core for the hub site's web IDE.
//! Compiled via `wasm-pack build --target web`.
//!
//! Filled in during R8.

use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub fn forage_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}
