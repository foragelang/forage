//! Linked-module shape: validation's structured output.
//!
//! Where parsing produces a [`ForageFile`] and validation reports
//! issues, **linking** resolves a recipe against its workspace and
//! produces the closure the runtime needs to execute it without doing
//! any more name resolution:
//!
//! - [`LinkedRecipe`] is one node in the closure: a parsed file plus
//!   the precomputed set of types it emits (declared `emits` if
//!   present, else the inferred set from the body's `emit X { … }`
//!   statements; composition bodies inherit from their terminal stage,
//!   resolved at link time).
//! - [`LinkedModule`] is the closure: a root recipe, every peer recipe
//!   the root's composition stages reach (transitively), and the
//!   unified type catalog visible to every node.
//!
//! Consumers:
//! - The runtime walks `module.stages` to dispatch composition stages
//!   without re-resolving names.
//! - The daemon serializes the whole `LinkedModule` per deployed
//!   version; the closure on disk fully determines the run-time
//!   behavior of a deployed composition.
//! - `derive_schema` reads the root recipe's emit set (or chases the
//!   composition's terminal stage through `module.stages`) to build
//!   per-type output tables.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::ast::ForageFile;
use crate::workspace::SerializableCatalog;

/// One recipe inside a linked module. Carries the parsed
/// [`ForageFile`] plus a precomputed resolved emit set (declared
/// `emits` if present, else inferred from the body). Composition
/// stages in the body still reference peers by name; the lookup
/// goes through the enclosing [`LinkedModule::stages`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LinkedRecipe {
    pub file: ForageFile,
    /// Recipe's declared or inferred emit types. For a scraping body
    /// this is the recipe's own `emit X { … }` statements (or the
    /// declared `emits` clause when one is present); for a composition
    /// body this is empty unless the source declared `emits` directly.
    /// The composition's "terminal" emit set is recovered by walking
    /// the chain through [`LinkedModule::stages`].
    pub emit_types: BTreeSet<String>,
}

impl LinkedRecipe {
    /// Project a parsed file into a linked node. Pure: doesn't touch
    /// the workspace or other recipes. Used by the linker after it has
    /// verified the file validates clean.
    pub fn from_file(file: ForageFile) -> Self {
        let emit_types = file.resolved_output_types();
        Self { file, emit_types }
    }
}

/// A linked module: a recipe plus every transitively-referenced
/// peer recipe, every shared type, and the unified type catalog —
/// the closure the runtime needs to execute the recipe without
/// doing any more name resolution.
///
/// The serialized form is what `Daemon::deploy` writes per version.
/// On read-back the runtime consumes the module directly; no parse,
/// no workspace re-discovery, no per-stage name lookup.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkedModule {
    /// The recipe this module is rooted at (the one the user
    /// invoked, deployed, or asked to run).
    pub root: LinkedRecipe,
    /// Stage recipes referenced transitively by the root, keyed by
    /// recipe header name. A composition stage's `RecipeRef.name` is
    /// the key into this map. Empty for non-composition roots.
    pub stages: BTreeMap<String, LinkedRecipe>,
    /// Unified type catalog visible to every recipe in the module —
    /// shared types from the root's workspace plus every stage's own
    /// types, deduplicated by name. Serializes through the wire-stable
    /// `SerializableCatalog` since `TypeCatalog`'s in-memory shape
    /// isn't part of the on-disk contract.
    pub catalog: SerializableCatalog,
}

impl LinkedModule {
    /// Look up a stage by recipe name. Returns the root itself when
    /// the lookup matches the root's own name — the composition
    /// runner doesn't need a special case for self-references because
    /// the validator's `ComposeCycle` rule already rejects them.
    pub fn stage(&self, name: &str) -> Option<&LinkedRecipe> {
        if self.root.file.recipe_name().is_some_and(|n| n == name) {
            return Some(&self.root);
        }
        self.stages.get(name)
    }
}
