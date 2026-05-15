//! JSON-LD projection of a `Snapshot`.
//!
//! `Snapshot::to_jsonld` reads the per-type alignment metadata already
//! stamped on `Snapshot.record_types` and produces a JSON-LD document:
//!
//! - The top-level `@context` carries one entry per *aligned* type. The
//!   entry is itself an object: `@id` is the IRI the type aligns with
//!   (first type-level alignment wins when several are declared), and a
//!   nested `@context` maps each field's *recipe-source* name to the
//!   IRI of its field-level alignment. Types without alignments contribute
//!   no `@context` entry.
//! - `@graph` carries every emitted record. Each record has `@type` set
//!   to the bare recipe type name (which the top-level `@context` resolves
//!   to an IRI when the type is aligned); record fields ride through
//!   verbatim under their recipe-source field names. JSON-LD's type-scoped
//!   `@context` mechanic does the term rewriting at parse time so two
//!   different types sharing a field name (e.g. `Product.name` mapped to
//!   `schema.org/name` and `Person.name` mapped to `foaf/name`) resolve
//!   independently.
//! - `_id`, `_ref` / `_type` ride through as-is. They aren't ontology
//!   terms — they're synthetic plumbing the runtime uses to link
//!   records. A consumer that wants stable JSON-LD identifiers can map
//!   `_id` to `@id` via its own framing.
//!
//! Alignment URIs (`ontology/term`) lower into full IRIs through a small
//! curated prefix table for the well-known ontologies the typed-hub
//! program calls out (`schema.org`, `wikidata`, `foaf`, `dublin-core`).
//! For ontologies outside the curated set the writer emits a CURIE
//! (`ontology:term`) that round-trips with intent preserved — a downstream
//! consumer with the full prefix table can resolve it, and the JSON-LD
//! envelope is still well-formed.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::ast::{AlignmentUri, JSONValue};
use crate::snapshot::{Record, RecordType, Snapshot};

/// One JSON-LD `@context` entry for an aligned type. The `id` is the IRI
/// the type itself aligns to (`@id` in JSON-LD). The nested `context`
/// maps each aligned field's recipe-source name to its term IRI; fields
/// with no alignment are absent (JSON-LD treats unmapped keys as
/// non-vocabulary fields, which is the correct ride-through behavior).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JsonLdTypeContext {
    #[serde(rename = "@id")]
    pub id: String,
    /// Per-field IRI mapping; empty map when no field carries an
    /// alignment. The map is serialized as a JSON-LD nested `@context`.
    #[serde(rename = "@context", skip_serializing_if = "IndexMap::is_empty")]
    pub fields: IndexMap<String, String>,
}

/// A JSON-LD document. Two top-level keys: `@context` (term map) and
/// `@graph` (the records). Round-trips through serde so consumers can
/// serialize it directly or wrap it.
///
/// `context` is an `IndexMap<String, JsonLdTypeContext>` keyed by bare
/// type name. Types without alignments contribute no entry — JSON-LD
/// parsers leave those `@type` strings as opaque labels, which is the
/// correct behavior for an un-aligned recipe type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JsonLdDocument {
    #[serde(rename = "@context")]
    pub context: IndexMap<String, JsonLdTypeContext>,
    #[serde(rename = "@graph")]
    pub graph: Vec<JsonLdRecord>,
}

/// One record in `@graph`. `type_name` becomes the JSON-LD `@type`
/// string; `fields` rides through unchanged.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JsonLdRecord {
    #[serde(rename = "@type")]
    pub type_name: String,
    #[serde(flatten)]
    pub fields: IndexMap<String, JSONValue>,
}

impl Snapshot {
    /// Project the snapshot into a JSON-LD document using the alignment
    /// metadata already on `record_types`. Records of un-aligned types
    /// ride through with `@type` set to the bare recipe type name and
    /// no `@context` entry.
    ///
    /// Alignment-to-IRI rewriting goes through `alignment_iri`; the
    /// curated prefix table covers `schema.org`, `wikidata`, `foaf`,
    /// `dublin-core`. Unrecognized ontologies fall through as opaque
    /// CURIEs (`ontology:term`).
    pub fn to_jsonld(&self) -> JsonLdDocument {
        let mut context: IndexMap<String, JsonLdTypeContext> = IndexMap::new();
        for (name, ty) in &self.record_types {
            if let Some(entry) = type_context(ty) {
                context.insert(name.clone(), entry);
            }
        }
        let graph = self.records.iter().map(record_to_jsonld).collect();
        JsonLdDocument { context, graph }
    }
}

/// Build a `JsonLdTypeContext` for an aligned type. Returns `None`
/// when the type carries no type-level *and* no field-level alignment —
/// the record rides through `@graph` with a bare-name `@type` and no
/// context entry.
fn type_context(ty: &RecordType) -> Option<JsonLdTypeContext> {
    let type_iri = ty.alignments.first().map(alignment_iri);
    let mut field_iris: IndexMap<String, String> = IndexMap::new();
    for f in &ty.fields {
        if let Some(uri) = &f.alignment {
            field_iris.insert(f.name.clone(), alignment_iri(uri));
        }
    }
    match (type_iri, field_iris.is_empty()) {
        (Some(id), _) => Some(JsonLdTypeContext {
            id,
            fields: field_iris,
        }),
        (None, true) => None,
        (None, false) => {
            // Field-level alignments without a type-level alignment: the
            // type itself has no `@id` to resolve to, but the field
            // mappings are still useful. Surface the bare type name as
            // the `@id` so downstream consumers see the recipe-type
            // identity, and carry the field map under `@context`.
            Some(JsonLdTypeContext {
                id: ty.name.clone(),
                fields: field_iris,
            })
        }
    }
}

fn record_to_jsonld(rec: &Record) -> JsonLdRecord {
    let mut fields = IndexMap::with_capacity(rec.fields.len() + 1);
    // Carry `_id` first so it appears next to `@type` in the output.
    fields.insert("_id".to_string(), JSONValue::String(rec.id.clone()));
    for (k, v) in &rec.fields {
        fields.insert(k.clone(), v.clone());
    }
    JsonLdRecord {
        type_name: rec.type_name.clone(),
        fields,
    }
}

/// Lower an `<ontology>/<term>` alignment URI to a JSON-LD IRI.
/// Well-known ontologies expand to their canonical base; unknown
/// ontologies stay as `ontology:term` CURIEs so the alignment intent
/// round-trips even when the writer can't synthesize a full IRI.
pub fn alignment_iri(uri: &AlignmentUri) -> String {
    if let Some(base) = well_known_base(&uri.ontology) {
        return format!("{base}{}", uri.term);
    }
    format!("{}:{}", uri.ontology, uri.term)
}

/// Curated table of ontologies with canonical base IRIs. The first four
/// entries match the program plan's well-known prefix set. New entries
/// are additive; unknown ontologies fall back to CURIE form in
/// `alignment_iri`.
fn well_known_base(ontology: &str) -> Option<&'static str> {
    match ontology {
        "schema.org" => Some("https://schema.org/"),
        // Wikidata terms come in two flavors: entity Q-ids
        // (`wikidata/Q2424752`) resolve under `/entity/`; property P-ids
        // (`wikidata/P112`) resolve under `/prop/direct/`. The bare
        // entity base is the right default — type-level alignments
        // point at entities, and field-level alignments that carry
        // property IDs round-trip the segment verbatim. Downstream
        // consumers that want the property base can rewrite.
        "wikidata" => Some("http://www.wikidata.org/entity/"),
        "foaf" => Some("http://xmlns.com/foaf/0.1/"),
        "dublin-core" => Some("http://purl.org/dc/terms/"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{AlignmentUri, FieldType, JSONValue, RecipeField, RecipeType};
    use crate::snapshot::{Record, Snapshot};

    fn aligned_product_type() -> RecipeType {
        RecipeType {
            name: "Product".into(),
            shared: false,
            extends: None,
            alignments: vec![AlignmentUri {
                ontology: "schema.org".into(),
                term: "Product".into(),
                span: 0..0,
            }],
            fields: vec![
                RecipeField {
                    name: "name".into(),
                    ty: FieldType::String,
                    optional: false,
                    alignment: Some(AlignmentUri {
                        ontology: "schema.org".into(),
                        term: "name".into(),
                        span: 0..0,
                    }),
                },
                RecipeField {
                    name: "sku".into(),
                    ty: FieldType::String,
                    optional: false,
                    alignment: Some(AlignmentUri {
                        ontology: "schema.org".into(),
                        term: "gtin".into(),
                        span: 0..0,
                    }),
                },
            ],
            span: 0..0,
        }
    }

    fn unaligned_type(name: &str) -> RecipeType {
        RecipeType {
            name: name.into(),
            shared: false,
            extends: None,
            alignments: Vec::new(),
            fields: vec![RecipeField {
                name: "label".into(),
                ty: FieldType::String,
                optional: false,
                alignment: None,
            }],
            span: 0..0,
        }
    }

    fn record(ty: &str, id: &str, fields: &[(&str, JSONValue)]) -> Record {
        let mut m = IndexMap::new();
        for (k, v) in fields {
            m.insert((*k).to_string(), v.clone());
        }
        Record {
            id: id.into(),
            type_name: ty.into(),
            fields: m,
        }
    }

    #[test]
    fn aligned_type_emits_context_entry_with_field_iris() {
        let mut snap = Snapshot::new();
        snap.set_record_types(std::iter::once(&aligned_product_type()));
        snap.emit(record(
            "Product",
            "rec-0",
            &[
                ("name", JSONValue::String("Widget".into())),
                ("sku", JSONValue::String("W-1".into())),
            ],
        ));

        let doc = snap.to_jsonld();

        let product_ctx = doc
            .context
            .get("Product")
            .expect("Product entry in @context");
        assert_eq!(product_ctx.id, "https://schema.org/Product");
        assert_eq!(
            product_ctx.fields.get("name").map(String::as_str),
            Some("https://schema.org/name"),
        );
        assert_eq!(
            product_ctx.fields.get("sku").map(String::as_str),
            Some("https://schema.org/gtin"),
        );

        assert_eq!(doc.graph.len(), 1);
        assert_eq!(doc.graph[0].type_name, "Product");
        assert_eq!(
            doc.graph[0].fields.get("name"),
            Some(&JSONValue::String("Widget".into())),
        );
        assert_eq!(
            doc.graph[0].fields.get("_id"),
            Some(&JSONValue::String("rec-0".into())),
        );
    }

    #[test]
    fn unaligned_type_rides_through_without_context_entry() {
        let mut snap = Snapshot::new();
        snap.set_record_types(std::iter::once(&unaligned_type("Note")));
        snap.emit(record(
            "Note",
            "rec-0",
            &[("label", JSONValue::String("anything".into()))],
        ));

        let doc = snap.to_jsonld();

        assert!(doc.context.get("Note").is_none());
        assert_eq!(doc.graph.len(), 1);
        assert_eq!(doc.graph[0].type_name, "Note");
        assert_eq!(
            doc.graph[0].fields.get("label"),
            Some(&JSONValue::String("anything".into())),
        );
    }

    #[test]
    fn type_level_alignment_alone_emits_context_without_field_map() {
        let mut ty = aligned_product_type();
        // Drop field-level alignments — keep only the type-level
        // alignment so the writer's "type-aligned, fields not" branch is
        // exercised.
        for f in &mut ty.fields {
            f.alignment = None;
        }
        let mut snap = Snapshot::new();
        snap.set_record_types(std::iter::once(&ty));

        let doc = snap.to_jsonld();
        let entry = doc
            .context
            .get("Product")
            .expect("Product context entry");
        assert_eq!(entry.id, "https://schema.org/Product");
        assert!(entry.fields.is_empty());
    }

    #[test]
    fn field_level_alignment_alone_emits_context_with_bare_type_id() {
        let ty = RecipeType {
            name: "Article".into(),
            shared: false,
            extends: None,
            alignments: Vec::new(),
            fields: vec![RecipeField {
                name: "title".into(),
                ty: FieldType::String,
                optional: false,
                alignment: Some(AlignmentUri {
                    ontology: "schema.org".into(),
                    term: "headline".into(),
                    span: 0..0,
                }),
            }],
            span: 0..0,
        };
        let mut snap = Snapshot::new();
        snap.set_record_types(std::iter::once(&ty));

        let doc = snap.to_jsonld();
        let entry = doc
            .context
            .get("Article")
            .expect("Article context entry");
        // No type-level alignment — the type identifier rides through
        // bare so the field map still has somewhere to hang off.
        assert_eq!(entry.id, "Article");
        assert_eq!(
            entry.fields.get("title").map(String::as_str),
            Some("https://schema.org/headline"),
        );
    }

    #[test]
    fn distinct_types_share_field_name_with_different_alignments() {
        // The type-scoped context pattern is what keeps these two
        // alignments independent; without it, `name` would be ambiguous
        // at the top level.
        let product = aligned_product_type();
        let person = RecipeType {
            name: "Person".into(),
            shared: false,
            extends: None,
            alignments: vec![AlignmentUri {
                ontology: "foaf".into(),
                term: "Person".into(),
                span: 0..0,
            }],
            fields: vec![RecipeField {
                name: "name".into(),
                ty: FieldType::String,
                optional: false,
                alignment: Some(AlignmentUri {
                    ontology: "foaf".into(),
                    term: "name".into(),
                    span: 0..0,
                }),
            }],
            span: 0..0,
        };
        let mut snap = Snapshot::new();
        snap.set_record_types([&product, &person]);

        let doc = snap.to_jsonld();
        assert_eq!(
            doc.context.get("Product").map(|c| c.fields.get("name").map(String::as_str)),
            Some(Some("https://schema.org/name")),
        );
        assert_eq!(
            doc.context.get("Person").map(|c| c.fields.get("name").map(String::as_str)),
            Some(Some("http://xmlns.com/foaf/0.1/name")),
        );
    }

    #[test]
    fn wikidata_qid_alignment_lowers_to_entity_iri() {
        let ty = RecipeType {
            name: "Beverage".into(),
            shared: false,
            extends: None,
            alignments: vec![AlignmentUri {
                ontology: "wikidata".into(),
                term: "Q40050".into(),
                span: 0..0,
            }],
            fields: Vec::new(),
            span: 0..0,
        };
        let mut snap = Snapshot::new();
        snap.set_record_types(std::iter::once(&ty));

        let doc = snap.to_jsonld();
        let entry = doc.context.get("Beverage").expect("Beverage entry");
        assert_eq!(entry.id, "http://www.wikidata.org/entity/Q40050");
    }

    #[test]
    fn wikidata_pid_alignment_lowers_to_entity_iri() {
        // The curated table lowers every `wikidata/<term>` alignment
        // under `/entity/`, including property P-ids. Consumers that
        // want the `/prop/direct/` base for property-typed fields
        // rewrite downstream. Pin the choice so a future drift fails
        // loudly.
        let pid = AlignmentUri {
            ontology: "wikidata".into(),
            term: "P112".into(),
            span: 0..0,
        };
        assert_eq!(
            alignment_iri(&pid),
            "http://www.wikidata.org/entity/P112",
        );
    }

    #[test]
    fn unknown_ontology_lowers_to_curie() {
        let ty = RecipeType {
            name: "Custom".into(),
            shared: false,
            extends: None,
            alignments: vec![AlignmentUri {
                ontology: "my-internal-vocab".into(),
                term: "WidgetType".into(),
                span: 0..0,
            }],
            fields: vec![RecipeField {
                name: "label".into(),
                ty: FieldType::String,
                optional: false,
                alignment: Some(AlignmentUri {
                    ontology: "my-internal-vocab".into(),
                    term: "displayLabel".into(),
                    span: 0..0,
                }),
            }],
            span: 0..0,
        };
        let mut snap = Snapshot::new();
        snap.set_record_types(std::iter::once(&ty));

        let doc = snap.to_jsonld();
        let entry = doc.context.get("Custom").expect("Custom entry");
        assert_eq!(entry.id, "my-internal-vocab:WidgetType");
        assert_eq!(
            entry.fields.get("label").map(String::as_str),
            Some("my-internal-vocab:displayLabel"),
        );
    }

    #[test]
    fn document_round_trips_through_serde() {
        let mut snap = Snapshot::new();
        snap.set_record_types(std::iter::once(&aligned_product_type()));
        snap.emit(record(
            "Product",
            "rec-0",
            &[("name", JSONValue::String("Widget".into()))],
        ));

        let doc = snap.to_jsonld();
        let j = serde_json::to_string(&doc).unwrap();
        let back: JsonLdDocument = serde_json::from_str(&j).unwrap();
        assert_eq!(back, doc);
    }

    #[test]
    fn serialized_top_level_uses_jsonld_keywords() {
        let mut snap = Snapshot::new();
        snap.set_record_types(std::iter::once(&aligned_product_type()));
        snap.emit(record(
            "Product",
            "rec-0",
            &[("name", JSONValue::String("Widget".into()))],
        ));
        let doc = snap.to_jsonld();
        let j = serde_json::to_string(&doc).unwrap();
        assert!(j.contains("\"@context\""), "got {j}");
        assert!(j.contains("\"@graph\""), "got {j}");
        assert!(j.contains("\"@type\":\"Product\""), "got {j}");
        assert!(j.contains("\"@id\":\"https://schema.org/Product\""), "got {j}");
    }
}
