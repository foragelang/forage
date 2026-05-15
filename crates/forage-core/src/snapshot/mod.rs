//! Record collection + expectation evaluation.
//!
//! The `Snapshot` is the final output of a run: records grouped by type
//! plus a `DiagnosticReport` describing how the run terminated and which
//! expectations went unmet.

mod jsonld;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ast::{AlignmentUri, ComparisonOp, Expectation, ExpectationKind, JSONValue, RecipeType};
use crate::source::LineMap;

pub use jsonld::{JsonLdDocument, JsonLdRecord, JsonLdTypeContext, alignment_iri};

/// One emitted record — a synthetic `_id`, a type name, and the bound
/// fields as plain JSON.
///
/// `_id` is assigned by the engine at emit time as a sequential string
/// (`rec-0`, `rec-1`, …) and is what `Ref<T>` field values point at. It
/// rides through the snapshot and serializes alongside the type-defined
/// fields so downstream consumers (Studio, the output store) can
/// resolve ref pointers without a side channel.
///
/// On the TS side it's exported as `RecipeRecord` to avoid colliding
/// with the built-in `Record<K, V>` utility type — without the rename,
/// the generated file would self-reference and TypeScript would
/// resolve `Record<string, unknown>` to the local declaration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, rename = "RecipeRecord")]
pub struct Record {
    #[serde(rename = "_id")]
    #[ts(rename = "_id")]
    pub id: String,
    #[serde(rename = "typeName")]
    #[ts(rename = "typeName")]
    pub type_name: String,
    #[ts(type = "Record<string, unknown>")]
    pub fields: IndexMap<String, JSONValue>,
}

/// Per-type metadata carried alongside the emitted records. The
/// catalog tells downstream consumers (JSON-LD writers, hub indexers)
/// what each emitted record's type is aligned with — without needing
/// to re-resolve the recipe source. Keyed by the type's name; one
/// entry per `type` declared in the recipe.
///
/// The runtime does not transform values across alignments — this is
/// strictly index data; JSON-LD serialization and hub indexing read it
/// to build `@context` / `@type` entries and per-field term mappings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RecordType {
    pub name: String,
    pub alignments: Vec<AlignmentUri>,
    pub fields: Vec<RecordTypeField>,
}

/// One field's projection of the recipe type into the snapshot. Carries
/// the field's name and its optional ontology alignment so JSON-LD
/// output can map each field key to its term.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RecordTypeField {
    pub name: String,
    pub alignment: Option<AlignmentUri>,
}

impl RecordType {
    /// Project a `RecipeType` (AST) into a `RecordType` (snapshot).
    /// Drops source-position info; keeps only what downstream consumers
    /// need to interpret records.
    pub fn from_recipe_type(ty: &RecipeType) -> Self {
        Self {
            name: ty.name.clone(),
            alignments: ty.alignments.iter().map(strip_span).collect(),
            fields: ty
                .fields
                .iter()
                .map(|f| RecordTypeField {
                    name: f.name.clone(),
                    alignment: f.alignment.as_ref().map(strip_span),
                })
                .collect(),
        }
    }
}

fn strip_span(uri: &AlignmentUri) -> AlignmentUri {
    AlignmentUri {
        ontology: uri.ontology.clone(),
        term: uri.term.clone(),
        span: 0..0,
    }
}

/// Snapshot of a run: every emitted record, in emission order, plus
/// the diagnostic envelope and per-type alignment metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Snapshot {
    pub records: Vec<Record>,
    pub diagnostic: DiagnosticReport,
    /// Type catalog snapshotted at run boundary. Keyed by type name in
    /// declaration order; empty when no types are declared in the
    /// recipe.
    pub record_types: IndexMap<String, RecordType>,
}

impl Snapshot {
    pub fn new() -> Self {
        Self {
            records: Vec::new(),
            diagnostic: DiagnosticReport::default(),
            record_types: IndexMap::new(),
        }
    }

    /// Populate the type catalog from every type the recipe could
    /// emit. Engines call this at run boundary with the full resolved
    /// `TypeCatalog` (file-local plus workspace-shared plus hub-dep)
    /// so the snapshot carries alignment metadata for any type a `share
    /// type` lets the recipe reference, not just the ones declared in
    /// the recipe file itself.
    pub fn set_record_types<'a>(&mut self, types: impl IntoIterator<Item = &'a RecipeType>) {
        self.record_types.clear();
        for ty in types {
            self.record_types
                .insert(ty.name.clone(), RecordType::from_recipe_type(ty));
        }
    }

    /// Push a record into the snapshot. Callers are responsible for
    /// assigning `_id`; the engine uses `next_record_id` to pull a
    /// sequential id off the snapshot before constructing the record.
    pub fn emit(&mut self, rec: Record) {
        self.records.push(rec);
    }

    /// Allocate the next sequential record id (`rec-0`, `rec-1`, …)
    /// without committing the record. Engines call this when
    /// constructing a record so they can plug the id into both the
    /// `Record._id` field and any `Ref<T>` binding the emit introduces
    /// via `as $v`.
    pub fn next_record_id(&self) -> String {
        format!("rec-{}", self.records.len())
    }

    pub fn count_by_type(&self, type_name: &str) -> usize {
        self.records
            .iter()
            .filter(|r| r.type_name == type_name)
            .count()
    }

    /// Run every expectation against the snapshot; populate
    /// `diagnostic.unmet_expectations`. The optional `LineMap` lets the
    /// snapshot attach a 0-based source line to each diagnostic — when
    /// `None`, lines are dropped (engine callers without source access).
    pub fn evaluate_expectations(
        &mut self,
        expectations: &[Expectation],
        line_map: Option<&LineMap>,
    ) {
        let mut unmet = Vec::new();
        for e in expectations {
            if let Some(message) = self.evaluate_one(&e.kind) {
                let line = line_map.map(|lm| lm.position(e.span.start).line);
                unmet.push(RuntimeDiagnostic { message, line });
            }
        }
        self.diagnostic.unmet_expectations = unmet;
    }

    fn evaluate_one(&self, kind: &ExpectationKind) -> Option<String> {
        match kind {
            ExpectationKind::RecordCount {
                type_name,
                op,
                value,
            } => {
                let actual = self.count_by_type(type_name) as i64;
                let ok = match op {
                    ComparisonOp::Ge => actual >= *value,
                    ComparisonOp::Gt => actual > *value,
                    ComparisonOp::Le => actual <= *value,
                    ComparisonOp::Lt => actual < *value,
                    ComparisonOp::Eq => actual == *value,
                    ComparisonOp::Ne => actual != *value,
                };
                if ok {
                    None
                } else {
                    Some(format!(
                        "records.where(typeName == {type_name:?}).count {op:?} {value} — got {actual}"
                    ))
                }
            }
        }
    }
}

impl Default for Snapshot {
    fn default() -> Self {
        Self::new()
    }
}

/// One diagnostic produced during a run. The message is the
/// human-readable explanation; `line` is the 0-based source line of the
/// recipe construct that produced it (e.g. an `expect` block, a step's
/// `browser { … }` config, or the step header for stalls), if the
/// caller had source access to resolve it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RuntimeDiagnostic {
    pub message: String,
    pub line: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default, TS)]
#[ts(export)]
pub struct DiagnosticReport {
    /// How the run terminated. `"settled"` / `"completed"` is the happy
    /// path; anything else is a clue.
    pub stall_reason: Option<RuntimeDiagnostic>,
    pub unmet_expectations: Vec<RuntimeDiagnostic>,
    pub unfired_capture_rules: Vec<RuntimeDiagnostic>,
    pub unmatched_captures: Vec<RuntimeDiagnostic>,
    pub unhandled_affordances: Vec<RuntimeDiagnostic>,
}

impl DiagnosticReport {
    pub fn has_content(&self) -> bool {
        self.stall_reason.is_some()
            || !self.unmet_expectations.is_empty()
            || !self.unfired_capture_rules.is_empty()
            || !self.unmatched_captures.is_empty()
            || !self.unhandled_affordances.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(t: &str, fields: &[(&str, JSONValue)]) -> Record {
        let mut m = IndexMap::new();
        for (k, v) in fields {
            m.insert((*k).to_string(), v.clone());
        }
        Record {
            id: String::new(),
            type_name: t.into(),
            fields: m,
        }
    }

    #[test]
    fn emit_and_count() {
        let mut s = Snapshot::new();
        s.emit(rec("Item", &[("id", JSONValue::String("a".into()))]));
        s.emit(rec("Item", &[("id", JSONValue::String("b".into()))]));
        s.emit(rec("Other", &[]));
        assert_eq!(s.count_by_type("Item"), 2);
        assert_eq!(s.count_by_type("Other"), 1);
        assert_eq!(s.count_by_type("Missing"), 0);
    }

    #[test]
    fn expectation_passes_when_met() {
        let mut s = Snapshot::new();
        for i in 0..5 {
            s.emit(rec("Item", &[("id", JSONValue::Int(i))]));
        }
        let exp = vec![Expectation {
            kind: ExpectationKind::RecordCount {
                type_name: "Item".into(),
                op: ComparisonOp::Ge,
                value: 5,
            },
            span: 0..0,
        }];
        s.evaluate_expectations(&exp, None);
        assert!(s.diagnostic.unmet_expectations.is_empty());
    }

    #[test]
    fn expectation_fails_when_unmet() {
        let mut s = Snapshot::new();
        s.emit(rec("Item", &[("id", JSONValue::Int(0))]));
        let exp = vec![Expectation {
            kind: ExpectationKind::RecordCount {
                type_name: "Item".into(),
                op: ComparisonOp::Ge,
                value: 5,
            },
            span: 0..0,
        }];
        s.evaluate_expectations(&exp, None);
        assert_eq!(s.diagnostic.unmet_expectations.len(), 1);
    }

    #[test]
    fn unmet_expectation_carries_line_when_line_map_provided() {
        let src = "recipe \"x\"\nengine http\nstep s { method \"GET\" url \"https://example.test\" }\nexpect { records.where(typeName == \"Item\").count >= 5 }\n";
        let lm = LineMap::new(src);
        let mut s = Snapshot::new();
        let expect_start = src.find("expect").unwrap();
        let expect_end = src.find('}').unwrap();
        let exp = vec![Expectation {
            kind: ExpectationKind::RecordCount {
                type_name: "Item".into(),
                op: ComparisonOp::Ge,
                value: 5,
            },
            span: expect_start..expect_end,
        }];
        s.evaluate_expectations(&exp, Some(&lm));
        assert_eq!(s.diagnostic.unmet_expectations.len(), 1);
        // `expect` starts on the 4th line (0-based index 3).
        assert_eq!(s.diagnostic.unmet_expectations[0].line, Some(3));
    }

    #[test]
    fn snapshot_round_trips_through_json() {
        let mut s = Snapshot::new();
        s.emit(rec("Item", &[("id", JSONValue::String("a".into()))]));
        let j = serde_json::to_string(&s).unwrap();
        let back: Snapshot = serde_json::from_str(&j).unwrap();
        assert_eq!(back.records.len(), 1);
        assert_eq!(back.records[0].type_name, "Item");
    }

    #[test]
    fn record_id_round_trips_through_json() {
        let mut s = Snapshot::new();
        s.emit(Record {
            id: "rec-7".into(),
            type_name: "Variant".into(),
            fields: IndexMap::new(),
        });
        let j = serde_json::to_string(&s.records[0]).unwrap();
        // `_id` is the canonical wire name; what `Ref<T>` field values
        // point at. Keeping the underscore-prefix avoids colliding with
        // a recipe author's own `id: String` field.
        assert!(j.contains("\"_id\":\"rec-7\""), "got {j}");
        let back: Record = serde_json::from_str(&j).unwrap();
        assert_eq!(back.id, "rec-7");
        assert_eq!(back.type_name, "Variant");
    }

    #[test]
    fn record_types_round_trip_alignment_metadata() {
        use crate::ast::{FieldType, RecipeField};

        let recipe_type = crate::ast::RecipeType {
            name: "Product".into(),
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
                    name: "price".into(),
                    ty: FieldType::Double,
                    optional: false,
                    alignment: Some(AlignmentUri {
                        ontology: "schema.org".into(),
                        term: "offers.price".into(),
                        span: 0..0,
                    }),
                },
            ],
            shared: false,
            alignments: vec![
                AlignmentUri {
                    ontology: "schema.org".into(),
                    term: "Product".into(),
                    span: 0..0,
                },
                AlignmentUri {
                    ontology: "wikidata".into(),
                    term: "Q2424752".into(),
                    span: 0..0,
                },
            ],
            extends: None,
            span: 0..0,
        };

        let mut s = Snapshot::new();
        s.set_record_types(std::iter::once(&recipe_type));

        let j = serde_json::to_string(&s).unwrap();
        let back: Snapshot = serde_json::from_str(&j).unwrap();

        let product = back
            .record_types
            .get("Product")
            .expect("Product entry in record_types");
        assert_eq!(product.alignments.len(), 2);
        assert_eq!(product.alignments[0].ontology, "schema.org");
        assert_eq!(product.alignments[0].term, "Product");
        assert_eq!(product.alignments[1].ontology, "wikidata");
        assert_eq!(product.alignments[1].term, "Q2424752");
        assert_eq!(product.fields.len(), 2);
        assert_eq!(product.fields[0].name, "name");
        assert_eq!(
            product.fields[0].alignment.as_ref().unwrap().term,
            "name"
        );
        assert_eq!(product.fields[1].name, "price");
        assert_eq!(
            product.fields[1].alignment.as_ref().unwrap().term,
            "offers.price"
        );
    }

    #[test]
    fn snapshot_with_no_types_has_empty_record_types() {
        let s = Snapshot::new();
        assert!(s.record_types.is_empty());
    }

    #[test]
    fn next_record_id_is_sequential() {
        let mut s = Snapshot::new();
        assert_eq!(s.next_record_id(), "rec-0");
        s.emit(Record {
            id: s.next_record_id(),
            type_name: "X".into(),
            fields: IndexMap::new(),
        });
        assert_eq!(s.next_record_id(), "rec-1");
        s.emit(Record {
            id: s.next_record_id(),
            type_name: "X".into(),
            fields: IndexMap::new(),
        });
        assert_eq!(s.next_record_id(), "rec-2");
    }
}
