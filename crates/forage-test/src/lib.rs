//! Test-vector harness for the cross-implementation parity fixtures
//! bundled in `crates/forage-test/fixtures/`.
//!
//! Owns the fixture-path resolution and the typed model of
//! `expected.json` so consumer test crates (`forage-core`,
//! `forage-http`) don't redeclare overlapping `Deserialize` structs or
//! hardcode their own path literals. Add new fields to the structs
//! below and every consumer picks them up.
//!
//! ```no_run
//! let manifest = forage_test::load_expected();
//! for entry in &manifest.recipes {
//!     let src = forage_test::load_recipe_source(&entry.file);
//!     // ...
//! }
//! ```
//!
//! Fixture sources live as `.forage` files; the `expected.json`
//! manifest declares per-fixture descriptors (parse summary, type
//! catalog, validation outcome, optional runtime snapshot) that the
//! Rust parser / validator / HTTP engine are asserted against.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use serde::Deserialize;

/// Directory containing the bundled `.forage` fixtures + `expected.json`.
pub fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

/// Read and parse `fixtures/expected.json`.
///
/// Panics on I/O or JSON failure — this is a test harness, not a
/// library; a missing or malformed manifest is a build-tree bug.
pub fn load_expected() -> ExpectedFile {
    let path = fixtures_dir().join("expected.json");
    let raw = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

/// Read a fixture `.forage` source by filename (e.g. `"01-minimal.forage"`).
///
/// Panics if the file does not exist or is unreadable.
pub fn load_recipe_source(file: &str) -> String {
    let path = fixtures_dir().join(file);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// Top-level shape of `expected.json`.
#[derive(Deserialize)]
pub struct ExpectedFile {
    pub recipes: Vec<RecipeExpect>,
}

/// One entry in `expected.json`. Every field besides `file` is
/// optional — fixtures opt in to the dimensions they exercise.
#[derive(Deserialize)]
pub struct RecipeExpect {
    pub file: String,
    #[serde(default = "default_true")]
    pub parses: bool,
    #[serde(default)]
    pub summary: Option<Summary>,
    #[serde(default)]
    pub types: Option<Vec<TypeExpect>>,
    #[serde(default)]
    pub enums: Option<Vec<EnumExpect>>,
    #[serde(default)]
    pub secrets: Option<Vec<String>>,
    /// Number of top-level `fn` declarations the recipe must carry.
    #[serde(default, rename = "functionCount")]
    pub function_count: Option<usize>,
    #[serde(default)]
    pub validation: Option<ValExpect>,
    /// Runtime snapshot: drives the HTTP engine against the declared
    /// fixtures and compares emitted records. Only populated for
    /// fixtures that test end-to-end execution.
    #[serde(default, rename = "runSnapshot")]
    pub run_snapshot: Option<RunSnapshot>,
}

fn default_true() -> bool {
    true
}

#[derive(Deserialize)]
pub struct Summary {
    pub name: String,
    #[serde(rename = "engineKind")]
    pub engine_kind: String,
    #[serde(rename = "typeCount")]
    pub type_count: usize,
    #[serde(rename = "enumCount")]
    pub enum_count: usize,
    #[serde(rename = "inputCount")]
    pub input_count: usize,
    #[serde(rename = "stepNames")]
    pub step_names: Vec<String>,
    #[serde(rename = "expectationCount")]
    pub expectation_count: usize,
}

#[derive(Deserialize)]
pub struct TypeExpect {
    pub name: String,
    #[serde(rename = "fieldNames")]
    pub field_names: Vec<String>,
    #[serde(rename = "requiredFieldCount", default)]
    pub required_field_count: usize,
}

#[derive(Deserialize)]
pub struct EnumExpect {
    pub name: String,
    pub variants: Vec<String>,
}

#[derive(Deserialize)]
pub struct ValExpect {
    #[serde(rename = "errorCount", default)]
    pub error_count: Option<usize>,
    #[serde(rename = "errorCountMin", default)]
    pub error_count_min: Option<usize>,
}

#[derive(Deserialize)]
pub struct RunSnapshot {
    #[serde(default)]
    pub inputs: HashMap<String, serde_json::Value>,
    #[serde(default, rename = "httpFixtures")]
    pub http_fixtures: Vec<HttpFixture>,
    pub records: Vec<ExpectedRecord>,
}

#[derive(Deserialize)]
pub struct HttpFixture {
    pub url: String,
    pub method: String,
    pub status: u16,
    pub body: String,
}

#[derive(Deserialize)]
pub struct ExpectedRecord {
    #[serde(rename = "typeName")]
    pub type_name: String,
    pub fields: serde_json::Map<String, serde_json::Value>,
}
