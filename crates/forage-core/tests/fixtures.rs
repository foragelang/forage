//! Cross-implementation parity test vectors.
//!
//! Loads the fixture manifest through the `forage-test` harness,
//! parses each `.forage` source, runs the validator, and verifies the
//! structural summary matches the expected descriptor. Any future
//! implementation that wants to claim parity has to clear the same
//! manifest.

use forage_core::ast::*;
use forage_core::workspace::TypeCatalog;
use forage_core::{parse, validate};
use forage_test::{ExpectedFile, Summary};

#[test]
fn fixtures_match_expected() {
    let exp: ExpectedFile = forage_test::load_expected();

    let mut failures = Vec::<String>::new();

    for r in &exp.recipes {
        let src = forage_test::load_recipe_source(&r.file);
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
        // Production goes through `TypeCatalog`, not direct lookup on
        // the recipe. Mirror that here so the test exercises the same
        // path as the runtime.
        let catalog = TypeCatalog::from_file(&recipe);
        if let Some(types) = &r.types {
            for te in types {
                let Some(ty) = catalog.ty(&te.name) else {
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
                let Some(en) = catalog.recipe_enum(&ee.name) else {
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
        if let Some(want) = r.function_count {
            if recipe.functions.len() != want {
                failures.push(format!(
                    "{}: functionCount {} != expected {}",
                    r.file,
                    recipe.functions.len(),
                    want,
                ));
            }
        }

        if let Some(v) = &r.validation {
            let rep = validate(&recipe, &catalog, &forage_core::RecipeSignatures::default());
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

fn check_summary(file: &str, recipe: &ForageFile, expected: &Summary, failures: &mut Vec<String>) {
    let recipe_name = recipe.recipe_name().unwrap_or("");
    if recipe_name != expected.name {
        failures.push(format!(
            "{}: name {:?} != expected {:?}",
            file, recipe_name, expected.name
        ));
    }
    let ek = match recipe.engine_kind() {
        Some(EngineKind::Http) => "http",
        Some(EngineKind::Browser) => "browser",
        None => "<none>",
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
        .statements()
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
}
