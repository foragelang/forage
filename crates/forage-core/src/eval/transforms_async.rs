//! Built-in transport-aware transforms.
//!
//! These transforms issue HTTP fetches through the engine's
//! `TransportContext`, which routes through the same transport as
//! step-level requests — so `--replay <fixtures>` captures them too.
//! The sync evaluator path rejects them with
//! `EvalError::TransformRequiresTransport`; the engine drives the
//! async eval path.

use indexmap::IndexMap;
use serde_json::Value as JsonValue;

use crate::eval::error::EvalError;
use crate::eval::transforms::{AsyncTransformFn, TransformRegistry};
use crate::eval::value::EvalValue;

/// Default endpoint for entity reconciliation. The `Special:EntityData`
/// pseudo-page on `www.wikidata.org` serves entity JSON without API
/// pagination or rate-limit headers; the `.json` suffix selects the
/// JSON serialization.
const WIKIDATA_ENDPOINT: &str = "https://www.wikidata.org/wiki/Special:EntityData";

pub(crate) fn register_async_builtins(reg: &mut TransformRegistry) {
    reg.register_async("wikidataEntity", wikidata_entity as AsyncTransformFn);
}

/// `wikidataEntity(qid)` — fetch the Wikidata entity for `qid`, flatten
/// the deeply-nested response into a record whose fields are the
/// entity's claims keyed by P-ID, and return it.
///
/// Recipes use it via direct call (`wikidataEntity($company.qid)`) or
/// pipe (`$company.qid | wikidataEntity`). The returned object's
/// fields:
///
/// - `_qid: String` — echo of the requested QID.
/// - `_label: String?` — the entity's English label, if present.
/// - `_description: String?` — the entity's English description.
/// - `P<n>: String` — each claim flattened to its first value's string
///   form (`Q5` for items, ISO time for dates, numeric amount for
///   quantities, etc.). Multiple-value claims are flattened to the
///   first — recipe authors who need the array can reach into `_raw`.
/// - `_raw: Object` — the full entity object from the Wikidata
///   response, untouched, for the rare case a recipe needs the
///   deeply-nested shape.
fn wikidata_entity<'a>(
    head: EvalValue,
    args: Vec<EvalValue>,
    ctx: &'a dyn crate::eval::TransportContext,
) -> crate::eval::TransformFuture<'a> {
    Box::pin(async move {
        let qid = pick_qid(&head, &args)?;
        let url = format!("{}/{}.json", WIKIDATA_ENDPOINT, qid);
        let payload = ctx.fetch_json(&url).await.map_err(|e| match e {
            EvalError::TransportError { msg, .. } => EvalError::TransportError {
                name: "wikidataEntity".into(),
                msg,
            },
            other => other,
        })?;
        flatten_entity(&qid, payload)
    })
}

fn pick_qid(head: &EvalValue, args: &[EvalValue]) -> Result<String, EvalError> {
    // Pipe form (`"Q123" | wikidataEntity`) puts the QID in head with no
    // args. Direct form (`wikidataEntity("Q123")`) passes it positionally
    // and head defaults to scope.current. Accept either.
    let candidate = match args.first() {
        Some(v) => v,
        None => head,
    };
    match candidate {
        EvalValue::String(s) if !s.is_empty() => Ok(s.clone()),
        EvalValue::Null => Err(EvalError::TransformError {
            name: "wikidataEntity".into(),
            msg: "qid is null — the record's wikidata identity field is missing".into(),
        }),
        other => Err(EvalError::TransformError {
            name: "wikidataEntity".into(),
            msg: format!("qid must be a non-empty string, got {other:?}"),
        }),
    }
}

/// Walk the response shape and build a flattened record. Handles the
/// missing-entity case (`{"entities": {"Q…": {"missing": ""}}}` —
/// what Wikidata serves for deleted or never-existed QIDs) by
/// surfacing a typed error so recipe authors see the QID that failed.
fn flatten_entity(qid: &str, payload: EvalValue) -> Result<EvalValue, EvalError> {
    let entities = match &payload {
        EvalValue::Object(o) => o.get("entities"),
        _ => None,
    };
    let entity = match entities {
        Some(EvalValue::Object(o)) => match o.get(qid) {
            Some(EvalValue::Object(e)) => e,
            _ => {
                return Err(EvalError::TransformError {
                    name: "wikidataEntity".into(),
                    msg: format!("entity {qid} not present in response"),
                });
            }
        },
        _ => {
            return Err(EvalError::TransformError {
                name: "wikidataEntity".into(),
                msg: "response missing `entities` object".into(),
            });
        }
    };
    if entity.contains_key("missing") {
        return Err(EvalError::TransformError {
            name: "wikidataEntity".into(),
            msg: format!("entity {qid} is missing on wikidata"),
        });
    }

    let mut out: IndexMap<String, EvalValue> = IndexMap::new();
    out.insert("_qid".into(), EvalValue::String(qid.into()));
    if let Some(label) = pick_en("labels", entity) {
        out.insert("_label".into(), EvalValue::String(label));
    }
    if let Some(desc) = pick_en("descriptions", entity) {
        out.insert("_description".into(), EvalValue::String(desc));
    }

    if let Some(EvalValue::Object(claims)) = entity.get("claims") {
        for (pid, claim_list) in claims {
            if let Some(flat) = flatten_claim_list(claim_list) {
                out.insert(pid.clone(), flat);
            }
        }
    }

    // Preserve the untouched entity for power-user paths.
    out.insert("_raw".into(), EvalValue::Object(entity.clone()));

    Ok(EvalValue::Object(out))
}

fn pick_en(group: &str, entity: &IndexMap<String, EvalValue>) -> Option<String> {
    let labels = match entity.get(group)? {
        EvalValue::Object(o) => o,
        _ => return None,
    };
    let en = match labels.get("en")? {
        EvalValue::Object(o) => o,
        _ => return None,
    };
    match en.get("value")? {
        EvalValue::String(s) => Some(s.clone()),
        _ => None,
    }
}

/// Flatten a claim list to a single first-value scalar. Returns `None`
/// for empty / unrecognized shapes so the recipe author isn't forced
/// to handle a "value present but unrepresentable" case — they reach
/// into `_raw` if they need the full structure.
fn flatten_claim_list(v: &EvalValue) -> Option<EvalValue> {
    let arr = match v {
        EvalValue::Array(xs) => xs,
        _ => return None,
    };
    let first = arr.first()?;
    let snak = match first {
        EvalValue::Object(o) => o.get("mainsnak")?,
        _ => return None,
    };
    let snak = match snak {
        EvalValue::Object(o) => o,
        _ => return None,
    };
    // `snaktype != "value"` (novalue, somevalue) carries no usable
    // value — represent it as Null so the field is still discoverable
    // on the record but doesn't crash a downstream `| upper` pipe.
    match snak.get("snaktype") {
        Some(EvalValue::String(s)) if s != "value" => return Some(EvalValue::Null),
        _ => {}
    }
    let datatype = match snak.get("datatype") {
        Some(EvalValue::String(s)) => s.as_str(),
        _ => "",
    };
    let value = snak.get("datavalue")?;
    let value = match value {
        EvalValue::Object(o) => o.get("value")?,
        _ => return None,
    };
    Some(flatten_datavalue(datatype, value))
}

/// Map a Wikidata `datavalue.value` (whose inner shape depends on
/// `datatype`) to a scalar suitable for a recipe's emit binding. The
/// rule of thumb: collapse to the string that a recipe author would
/// most likely want to compare or display. Authors who need a richer
/// projection drop into `_raw`.
fn flatten_datavalue(datatype: &str, v: &EvalValue) -> EvalValue {
    match datatype {
        // Plain string-valued datatypes: the value is already a string.
        "string" | "external-id" | "commonsMedia" | "url" | "math" | "musical-notation"
        | "geo-shape" | "tabular-data" => match v {
            EvalValue::String(s) => EvalValue::String(s.clone()),
            other => other.clone(),
        },
        // Item / property references: pull `id` (the Q… or P… id).
        "wikibase-item" | "wikibase-property" | "wikibase-lexeme" | "wikibase-form"
        | "wikibase-sense" => match v {
            EvalValue::Object(o) => match o.get("id") {
                Some(EvalValue::String(s)) => EvalValue::String(s.clone()),
                _ => EvalValue::Null,
            },
            _ => EvalValue::Null,
        },
        // Time: ISO-ish string in `time` field.
        "time" => match v {
            EvalValue::Object(o) => match o.get("time") {
                Some(EvalValue::String(s)) => EvalValue::String(s.clone()),
                _ => EvalValue::Null,
            },
            _ => EvalValue::Null,
        },
        // Quantity: numeric `amount` (signed string like "+50000000").
        "quantity" => match v {
            EvalValue::Object(o) => match o.get("amount") {
                Some(EvalValue::String(s)) => EvalValue::String(s.clone()),
                _ => EvalValue::Null,
            },
            _ => EvalValue::Null,
        },
        // Monolingual text: just the `text`.
        "monolingualtext" => match v {
            EvalValue::Object(o) => match o.get("text") {
                Some(EvalValue::String(s)) => EvalValue::String(s.clone()),
                _ => EvalValue::Null,
            },
            _ => EvalValue::Null,
        },
        // Globe coordinate: "lat,lon" — lossy but useful for diff /
        // equality checks. Authors who need precision reach for `_raw`.
        "globe-coordinate" => match v {
            EvalValue::Object(o) => {
                let lat = o.get("latitude").cloned().unwrap_or(EvalValue::Null);
                let lon = o.get("longitude").cloned().unwrap_or(EvalValue::Null);
                let (a, b) = (json_num(&lat), json_num(&lon));
                match (a, b) {
                    (Some(la), Some(lo)) => EvalValue::String(format!("{la},{lo}")),
                    _ => EvalValue::Null,
                }
            }
            _ => EvalValue::Null,
        },
        // Unknown datatype: pass the inner value through. Recipes can
        // still poke at it via `_raw`; failing soft here keeps a single
        // unrecognized claim from breaking the whole reconciliation.
        _ => v.clone(),
    }
}

fn json_num(v: &EvalValue) -> Option<f64> {
    match v {
        EvalValue::Int(n) => Some(*n as f64),
        EvalValue::Double(n) => Some(*n),
        EvalValue::String(s) => s.parse().ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::{Evaluator, NoTransport, Scope, TransportContext, default_registry};
    use crate::ast::{ExtractionExpr, JSONValue};

    fn entity_payload() -> JsonValue {
        serde_json::json!({
            "entities": {
                "Q24851740": {
                    "id": "Q24851740",
                    "type": "item",
                    "labels": {
                        "en": { "language": "en", "value": "Stripe" }
                    },
                    "descriptions": {
                        "en": { "language": "en", "value": "American payments company" }
                    },
                    "claims": {
                        "P112": [
                            {
                                "mainsnak": {
                                    "snaktype": "value",
                                    "property": "P112",
                                    "datatype": "wikibase-item",
                                    "datavalue": {
                                        "value": { "id": "Q5111731" },
                                        "type": "wikibase-entityid"
                                    }
                                }
                            }
                        ],
                        "P571": [
                            {
                                "mainsnak": {
                                    "snaktype": "value",
                                    "property": "P571",
                                    "datatype": "time",
                                    "datavalue": {
                                        "value": { "time": "+2010-01-01T00:00:00Z" },
                                        "type": "time"
                                    }
                                }
                            }
                        ],
                        "P159": [
                            {
                                "mainsnak": {
                                    "snaktype": "value",
                                    "property": "P159",
                                    "datatype": "wikibase-item",
                                    "datavalue": {
                                        "value": { "id": "Q62" },
                                        "type": "wikibase-entityid"
                                    }
                                }
                            }
                        ],
                        "P1128": [
                            {
                                "mainsnak": {
                                    "snaktype": "value",
                                    "property": "P1128",
                                    "datatype": "quantity",
                                    "datavalue": {
                                        "value": { "amount": "+8100", "unit": "1" },
                                        "type": "quantity"
                                    }
                                }
                            }
                        ]
                    }
                }
            }
        })
    }

    struct FixedTransport(JsonValue);

    #[async_trait::async_trait]
    impl TransportContext for FixedTransport {
        async fn fetch_json(&self, _url: &str) -> Result<EvalValue, EvalError> {
            Ok((&self.0).into())
        }
    }

    fn build_registry() -> TransformRegistry {
        let base = default_registry();
        let mut reg = TransformRegistry::with_user_fns(base, Vec::new());
        register_async_builtins(&mut reg);
        reg
    }

    #[tokio::test]
    async fn wikidata_entity_flattens_named_claims() {
        let reg = build_registry();
        let ev = Evaluator::new(&reg);
        let expr = ExtractionExpr::Call {
            name: "wikidataEntity".into(),
            args: vec![ExtractionExpr::Literal(JSONValue::String("Q24851740".into()))],
        };
        let transport = FixedTransport(entity_payload());
        let v = ev
            .eval_extraction_async(&expr, &Scope::new(), &transport)
            .await
            .unwrap();
        let EvalValue::Object(o) = v else {
            panic!("expected object, got {v:?}");
        };
        assert_eq!(o.get("_qid"), Some(&EvalValue::String("Q24851740".into())));
        assert_eq!(o.get("_label"), Some(&EvalValue::String("Stripe".into())));
        assert_eq!(o.get("P112"), Some(&EvalValue::String("Q5111731".into())));
        assert_eq!(
            o.get("P571"),
            Some(&EvalValue::String("+2010-01-01T00:00:00Z".into())),
        );
        assert_eq!(o.get("P159"), Some(&EvalValue::String("Q62".into())));
        assert_eq!(o.get("P1128"), Some(&EvalValue::String("+8100".into())));
        assert!(o.contains_key("_raw"), "_raw should pass through");
    }

    #[tokio::test]
    async fn pipe_form_carries_qid_in_head() {
        let reg = build_registry();
        let ev = Evaluator::new(&reg);
        let expr = ExtractionExpr::Pipe(
            Box::new(ExtractionExpr::Literal(JSONValue::String("Q24851740".into()))),
            vec![crate::ast::TransformCall {
                name: "wikidataEntity".into(),
                args: vec![],
            }],
        );
        let transport = FixedTransport(entity_payload());
        let v = ev
            .eval_extraction_async(&expr, &Scope::new(), &transport)
            .await
            .unwrap();
        let EvalValue::Object(o) = v else {
            panic!("expected object");
        };
        assert_eq!(o.get("_qid"), Some(&EvalValue::String("Q24851740".into())));
    }

    #[tokio::test]
    async fn missing_entity_surfaces_typed_error() {
        let reg = build_registry();
        let ev = Evaluator::new(&reg);
        let payload = serde_json::json!({
            "entities": { "Q999999999": { "missing": "" } }
        });
        let transport = FixedTransport(payload);
        let expr = ExtractionExpr::Call {
            name: "wikidataEntity".into(),
            args: vec![ExtractionExpr::Literal(JSONValue::String(
                "Q999999999".into(),
            ))],
        };
        let err = ev
            .eval_extraction_async(&expr, &Scope::new(), &transport)
            .await
            .unwrap_err();
        assert!(
            matches!(err, EvalError::TransformError { ref name, ref msg }
                if name == "wikidataEntity" && msg.contains("missing")),
            "got {err:?}",
        );
    }

    #[tokio::test]
    async fn null_qid_surfaces_typed_error() {
        let reg = build_registry();
        let ev = Evaluator::new(&reg);
        let transport = FixedTransport(entity_payload());
        let expr = ExtractionExpr::Call {
            name: "wikidataEntity".into(),
            args: vec![ExtractionExpr::Literal(JSONValue::Null)],
        };
        let err = ev
            .eval_extraction_async(&expr, &Scope::new(), &transport)
            .await
            .unwrap_err();
        assert!(
            matches!(err, EvalError::TransformError { ref name, ref msg }
                if name == "wikidataEntity" && msg.contains("null")),
            "got {err:?}",
        );
    }

    #[test]
    fn sync_path_rejects_wikidata_entity() {
        let reg = build_registry();
        let ev = Evaluator::new(&reg);
        let expr = ExtractionExpr::Call {
            name: "wikidataEntity".into(),
            args: vec![ExtractionExpr::Literal(JSONValue::String("Q1".into()))],
        };
        let err = ev.eval_extraction(&expr, &Scope::new()).unwrap_err();
        assert!(
            matches!(err, EvalError::TransformRequiresTransport { ref name }
                if name == "wikidataEntity"),
            "got {err:?}",
        );
        // Pure sentinel — exercise NoTransport in a no-op call to keep
        // it in the unused-warning shield.
        let _ = NoTransport;
    }

    #[test]
    fn novalue_claim_collapses_to_null() {
        // Wikidata records a property with `snaktype: "novalue"` when
        // the entity is asserted to have no value for that property.
        // Flatten should surface Null (the field is meaningful, but
        // the value is absent).
        let raw = serde_json::json!([
            {
                "mainsnak": {
                    "snaktype": "novalue",
                    "property": "P999",
                    "datatype": "wikibase-item"
                }
            }
        ]);
        let v: EvalValue = (&raw).into();
        let out = flatten_claim_list(&v).expect("flatten");
        assert_eq!(out, EvalValue::Null);
    }
}
