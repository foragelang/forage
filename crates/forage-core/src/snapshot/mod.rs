//! Record collection + expectation evaluation.
//!
//! The `Snapshot` is the final output of a run: records grouped by type
//! plus a `DiagnosticReport` describing how the run terminated and which
//! expectations went unmet.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ast::{ComparisonOp, Expectation, ExpectationKind, JSONValue};
use crate::source::LineMap;

/// One emitted record — a type name + the bound fields as plain JSON.
///
/// On the TS side it's exported as `RecipeRecord` to avoid colliding
/// with the built-in `Record<K, V>` utility type — without the rename,
/// the generated file would self-reference and TypeScript would
/// resolve `Record<string, unknown>` to the local declaration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, rename = "RecipeRecord")]
pub struct Record {
    #[serde(rename = "typeName")]
    #[ts(rename = "typeName")]
    pub type_name: String,
    #[ts(type = "Record<string, unknown>")]
    pub fields: IndexMap<String, JSONValue>,
}

/// Snapshot of a run: every emitted record, in emission order, plus
/// the diagnostic envelope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Snapshot {
    pub records: Vec<Record>,
    pub diagnostic: DiagnosticReport,
}

impl Snapshot {
    pub fn new() -> Self {
        Self {
            records: Vec::new(),
            diagnostic: DiagnosticReport::default(),
        }
    }

    pub fn emit(&mut self, rec: Record) {
        self.records.push(rec);
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
}
