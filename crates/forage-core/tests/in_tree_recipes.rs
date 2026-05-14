//! Walk the in-tree `recipes/` directory and parse every recipe.forage.
//! Surfaces parser regressions against real recipes.

use std::fs;
use std::path::PathBuf;

use forage_core::workspace::TypeCatalog;
use forage_core::{parse, validate};

fn recipes_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop(); // out of forage-core
    p.pop(); // out of crates
    p.push("recipes");
    p
}

#[test]
fn parses_every_in_tree_recipe() {
    let root = recipes_root();
    assert!(root.exists(), "recipes/ dir missing at {}", root.display());

    let mut failures = Vec::<(String, String)>::new();
    let mut parsed = 0;
    for entry in fs::read_dir(&root).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        let recipe = path.join("recipe.forage");
        if !recipe.exists() {
            continue;
        }
        let slug = path.file_name().unwrap().to_string_lossy().into_owned();
        let source = fs::read_to_string(&recipe).unwrap();
        match parse(&source) {
            Ok(r) => {
                parsed += 1;
                let catalog = TypeCatalog::from_recipe(&r);
                let rep = validate(&r, &catalog);
                for e in rep.errors() {
                    failures.push((
                        slug.clone(),
                        format!("validate: {} ({:?})", e.message, e.code),
                    ));
                }
            }
            Err(e) => failures.push((slug, format!("parse: {e}"))),
        }
    }
    println!("parsed {parsed} recipes cleanly");
    if !failures.is_empty() {
        for (slug, err) in &failures {
            eprintln!("--- {slug}: {err}");
        }
        panic!("{} recipe issues", failures.len());
    }
}
