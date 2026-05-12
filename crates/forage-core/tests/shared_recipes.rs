//! Shared-recipes test vector harness.
//!
//! Loads `Tests/shared-recipes/*.forage` + `expected.json`, parses each
//! recipe, runs the validator, and verifies the structural summary
//! matches the expected descriptor. This is the same harness the Swift
//! and TypeScript implementations conform to; passing here means the
//! Rust parser/validator stays in lockstep with them.

use std::fs;
use std::path::PathBuf;

use forage_core::ast::*;
use forage_core::{parse, validate};
use serde::Deserialize;

#[derive(Deserialize)]
struct ExpectedFile {
    recipes: Vec<RecipeExpect>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct RecipeExpect {
    file: String,
    #[serde(default = "default_true")]
    parses: bool,
    #[serde(default)]
    summary: Option<Summary>,
    #[serde(default)]
    types: Option<Vec<TypeExpect>>,
    #[serde(default)]
    enums: Option<Vec<EnumExpect>>,
    #[serde(default)]
    secrets: Option<Vec<String>>,
    #[serde(default)]
    validation: Option<ValExpect>,
}

fn default_true() -> bool {
    true
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct Summary {
    name: String,
    #[serde(rename = "engineKind")]
    engine_kind: String,
    #[serde(rename = "typeCount")]
    type_count: usize,
    #[serde(rename = "enumCount")]
    enum_count: usize,
    #[serde(rename = "inputCount")]
    input_count: usize,
    #[serde(rename = "stepNames")]
    step_names: Vec<String>,
    #[serde(rename = "expectationCount")]
    expectation_count: usize,
    #[serde(rename = "importCount")]
    import_count: usize,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct TypeExpect {
    name: String,
    #[serde(rename = "fieldNames")]
    field_names: Vec<String>,
    #[serde(rename = "requiredFieldCount", default)]
    required_field_count: usize,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct EnumExpect {
    name: String,
    variants: Vec<String>,
}

#[derive(Deserialize)]
struct ValExpect {
    #[serde(rename = "errorCount", default)]
    error_count: Option<usize>,
    #[serde(rename = "errorCountMin", default)]
    error_count_min: Option<usize>,
}

fn root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p.push("Tests");
    p.push("shared-recipes");
    p
}

#[test]
fn shared_recipes_match_expected() {
    let dir = root();
    let expected_path = dir.join("expected.json");
    let raw = fs::read_to_string(&expected_path).unwrap();
    let exp: ExpectedFile = serde_json::from_str(&raw).unwrap();

    let mut failures = Vec::<String>::new();

    for r in &exp.recipes {
        let path = dir.join(&r.file);
        let src = fs::read_to_string(&path).unwrap();
        let parsed = parse(&src);

        if !r.parses {
            if parsed.is_ok() {
                failures.push(format!("{}: expected parse failure, but parsed", r.file));
            }
            continue;
        }
        let recipe = match parsed {
            Ok(r) => r,
            Err(e) => {
                failures.push(format!("{}: expected parse, got: {e}", r.file));
                continue;
            }
        };

        if let Some(s) = &r.summary {
            check_summary(&r.file, &recipe, s, &mut failures);
        }
        if let Some(types) = &r.types {
            for te in types {
                let Some(ty) = recipe.ty(&te.name) else {
                    failures.push(format!("{}: missing type {}", r.file, te.name));
                    continue;
                };
                let names: Vec<&String> = ty.fields.iter().map(|f| &f.name).collect();
                let want: Vec<&String> = te.field_names.iter().collect();
                if names != want {
                    failures.push(format!(
                        "{}: type {} field names {:?} != expected {:?}",
                        r.file, te.name, names, want
                    ));
                }
                let req = ty.fields.iter().filter(|f| !f.optional).count();
                if req != te.required_field_count {
                    failures.push(format!(
                        "{}: type {} required field count {} != expected {}",
                        r.file, te.name, req, te.required_field_count
                    ));
                }
            }
        }
        if let Some(enums) = &r.enums {
            for ee in enums {
                let Some(en) = recipe.recipe_enum(&ee.name) else {
                    failures.push(format!("{}: missing enum {}", r.file, ee.name));
                    continue;
                };
                if en.variants != ee.variants {
                    failures.push(format!(
                        "{}: enum {} variants {:?} != expected {:?}",
                        r.file, ee.name, en.variants, ee.variants
                    ));
                }
            }
        }
        if let Some(secrets) = &r.secrets {
            if &recipe.secrets != secrets {
                failures.push(format!(
                    "{}: secrets {:?} != expected {:?}",
                    r.file, recipe.secrets, secrets
                ));
            }
        }

        if let Some(v) = &r.validation {
            let rep = validate(&recipe);
            let errs = rep.errors().count();
            if let Some(want) = v.error_count {
                if errs != want {
                    failures.push(format!(
                        "{}: validation errorCount {} != expected {}",
                        r.file, errs, want
                    ));
                }
            }
            if let Some(min) = v.error_count_min {
                if errs < min {
                    failures.push(format!(
                        "{}: validation errorCount {} < min {}",
                        r.file, errs, min
                    ));
                }
            }
        }
    }

    if !failures.is_empty() {
        for f in &failures {
            eprintln!("--- {f}");
        }
        panic!("{} failures", failures.len());
    }
}

fn check_summary(file: &str, recipe: &Recipe, expected: &Summary, failures: &mut Vec<String>) {
    if recipe.name != expected.name {
        failures.push(format!(
            "{}: name {:?} != expected {:?}",
            file, recipe.name, expected.name
        ));
    }
    let ek = match recipe.engine_kind {
        EngineKind::Http => "http",
        EngineKind::Browser => "browser",
    };
    if ek != expected.engine_kind {
        failures.push(format!(
            "{}: engineKind {:?} != expected {:?}",
            file, ek, expected.engine_kind
        ));
    }
    if recipe.types.len() != expected.type_count {
        failures.push(format!(
            "{}: typeCount {} != {}",
            file,
            recipe.types.len(),
            expected.type_count
        ));
    }
    if recipe.enums.len() != expected.enum_count {
        failures.push(format!(
            "{}: enumCount {} != {}",
            file,
            recipe.enums.len(),
            expected.enum_count
        ));
    }
    if recipe.inputs.len() != expected.input_count {
        failures.push(format!(
            "{}: inputCount {} != {}",
            file,
            recipe.inputs.len(),
            expected.input_count
        ));
    }
    let step_names: Vec<String> = recipe
        .body
        .iter()
        .filter_map(|s| match s {
            Statement::Step(st) => Some(st.name.clone()),
            _ => None,
        })
        .collect();
    if step_names != expected.step_names {
        failures.push(format!(
            "{}: stepNames {:?} != expected {:?}",
            file, step_names, expected.step_names
        ));
    }
    if recipe.expectations.len() != expected.expectation_count {
        failures.push(format!(
            "{}: expectationCount {} != {}",
            file,
            recipe.expectations.len(),
            expected.expectation_count
        ));
    }
    if recipe.imports.len() != expected.import_count {
        failures.push(format!(
            "{}: importCount {} != {}",
            file,
            recipe.imports.len(),
            expected.import_count
        ));
    }
}
