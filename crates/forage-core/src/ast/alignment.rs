//! Ontology alignment URIs.
//!
//! An alignment is a declaration of correspondence between a recipe-declared
//! type (or field) and an external ontology term — e.g. `schema.org/Product`,
//! `wikidata/Q2424752`. Alignments are *index data*: the runtime carries them
//! through to the snapshot, the hub indexes recipes and types by them, and
//! JSON-LD output translates them into `@context` / `@type`. The runtime
//! does not synthesize values across alignments; semantic translation is a
//! separate concern.
//!
//! Surface syntax is slash-separated: `<ontology>/<term>` where the
//! ontology may contain `.` (e.g. `schema.org`) and the term is one or
//! more dotted segments (`schema.org/offers.price`).

use serde::{Deserialize, Serialize};

use crate::ast::span::Span;

/// A single alignment annotation. The `ontology` is the first
/// slash-separated segment (`schema.org`, `wikidata`, …); `term` is
/// everything after the first `/`, joined by `.`s when the term path
/// has multiple segments (e.g. `offers.price`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlignmentUri {
    pub ontology: String,
    pub term: String,
    #[serde(default)]
    pub span: Span,
}
