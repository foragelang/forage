//! Workspace loader + catalog tests.

use std::fs;
use std::path::{Path, PathBuf};

use forage_core::workspace::{
    self, MANIFEST_NAME, TypeCatalog, Workspace, WorkspaceError, WorkspaceFileKind, discover,
};
use forage_core::{parse, validate};

fn write(path: &Path, body: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, body).unwrap();
}

fn workspace_in(tmp: &tempfile::TempDir) -> &Path {
    write(&tmp.path().join(MANIFEST_NAME), "");
    tmp.path()
}

#[test]
fn declarations_file_parses_and_contributes_types() {
    let tmp = tempfile::tempdir().unwrap();
    let root = workspace_in(&tmp);
    write(
        &root.join("cannabis.forage"),
        "type Dispensary { id: String, name: String }\n\
         enum MenuType { Recreational, Medical }\n",
    );
    let recipe_path = root.join("trilogy").join("recipe.forage");
    write(
        &recipe_path,
        "recipe \"trilogy\"\nengine http\n\
         type Item { id: String }\n",
    );
    let ws = workspace::load(root).unwrap();
    let catalog = ws.catalog_from_disk(&recipe_path).unwrap();
    assert!(catalog.ty("Dispensary").is_some());
    assert!(catalog.recipe_enum("MenuType").is_some());
    assert!(catalog.ty("Item").is_some(), "recipe-local type missing");
}

#[test]
fn recipe_local_overrides_shared_type() {
    let tmp = tempfile::tempdir().unwrap();
    let root = workspace_in(&tmp);
    write(
        &root.join("cannabis.forage"),
        "type Product { id: String, name: String }\n",
    );
    let recipe_path = root.join("sweed").join("recipe.forage");
    write(
        &recipe_path,
        "recipe \"sweed\"\nengine http\n\
         type Product { id: String, name: String, terpenes: String? }\n",
    );
    let ws = workspace::load(root).unwrap();
    let catalog = ws.catalog_from_disk(&recipe_path).unwrap();
    let product = catalog.ty("Product").expect("Product");
    assert_eq!(product.fields.len(), 3, "recipe-local override lost");
    assert!(product.fields.iter().any(|f| f.name == "terpenes"));
}

#[test]
fn duplicate_across_declarations_files_is_error() {
    let tmp = tempfile::tempdir().unwrap();
    let root = workspace_in(&tmp);
    write(&root.join("a.forage"), "type Product { id: String }\n");
    write(&root.join("b.forage"), "type Product { id: String }\n");
    let recipe_path = root.join("r").join("recipe.forage");
    write(&recipe_path, "recipe \"r\"\nengine http\n");
    let ws = workspace::load(root).unwrap();
    let err = ws
        .catalog_from_disk(&recipe_path)
        .expect_err("duplicate must fail");
    match err {
        WorkspaceError::DuplicateType { name, paths } => {
            assert_eq!(name, "Product");
            assert_eq!(paths.len(), 2);
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn discover_walks_ancestors_and_returns_none_outside() {
    let tmp = tempfile::tempdir().unwrap();
    let root = workspace_in(&tmp);
    let nested = root.join("a").join("b").join("c");
    fs::create_dir_all(&nested).unwrap();
    let ws = discover(&nested).expect("ancestor walk should find marker");
    assert_eq!(
        ws.root.canonicalize().unwrap(),
        root.canonicalize().unwrap()
    );

    let stranded = tempfile::tempdir().unwrap();
    assert!(
        discover(stranded.path()).is_none(),
        "no marker → no workspace"
    );
}

#[test]
fn lonely_recipe_mode_uses_recipe_local_catalog() {
    // No surrounding `forage.toml`. The validator must still work given
    // a recipe-local catalog built straight from the parsed recipe.
    let src = r#"
        recipe "lonely"
        engine http
        type Item { id: String }
        step list {
            method "GET"
            url "https://example.com"
        }
        for $x in $list.items[*] {
            emit Item { id ← $x.id }
        }
    "#;
    let r = parse(src).unwrap();
    let catalog = TypeCatalog::from_recipe(&r);
    let rep = validate(&r, &catalog);
    assert!(!rep.has_errors(), "lonely recipe errored: {:?}", rep.issues);
}

#[test]
fn workspace_classifies_files_by_kind() {
    let tmp = tempfile::tempdir().unwrap();
    let root = workspace_in(&tmp);
    write(&root.join("shared.forage"), "type T { id: String }\n");
    write(
        &root.join("rec").join("recipe.forage"),
        "recipe \"rec\"\nengine http\n",
    );
    let ws: Workspace = workspace::load(root).unwrap();
    let mut recipes: Vec<&PathBuf> = ws
        .files
        .iter()
        .filter(|f| matches!(f.kind, WorkspaceFileKind::Recipe { .. }))
        .map(|f| &f.path)
        .collect();
    let declarations: Vec<&PathBuf> = ws
        .files
        .iter()
        .filter(|f| matches!(f.kind, WorkspaceFileKind::Declarations))
        .map(|f| &f.path)
        .collect();
    assert_eq!(recipes.len(), 1);
    assert_eq!(declarations.len(), 1);
    recipes.sort();
    assert!(recipes[0].ends_with("rec/recipe.forage"));
    assert!(declarations[0].ends_with("shared.forage"));
}

#[test]
fn workspace_recipe_for_finds_by_slug() {
    let tmp = tempfile::tempdir().unwrap();
    let root = workspace_in(&tmp);
    write(
        &root.join("alpha").join("recipe.forage"),
        "recipe \"alpha\"\nengine http\n",
    );
    let ws = workspace::load(root).unwrap();
    let entry = ws.recipe_for("alpha").expect("recipe by slug");
    assert!(entry.path.ends_with("alpha/recipe.forage"));
}

#[test]
#[serial_test::serial]
fn hub_dep_cache_contributes_types() {
    // Stand up a cache under FORAGE_HUB_CACHE, drop a one-file package
    // with a shared type, point the workspace's [deps] at it, and
    // verify the catalog folds the type in.
    let tmp = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(
        &root.join(MANIFEST_NAME),
        "[deps]\n\"dima/shared-types\" = 3\n",
    );
    let recipe_path = root.join("rec").join("recipe.forage");
    write(&recipe_path, "recipe \"rec\"\nengine http\n");
    let pkg_dir = cache.path().join("dima").join("shared-types").join("3");
    write(
        &pkg_dir.join("cannabis.forage"),
        "type DispensaryFromHub { id: String }\n",
    );

    // `FORAGE_HUB_CACHE` is process-global; restore it after we exit.
    let prev = std::env::var("FORAGE_HUB_CACHE").ok();
    // SAFETY: tests run single-threaded by default for env-var
    // mutation; the harness will serialise this test via the lock on
    // the env var if needed.
    // SAFETY: env mutation is unsafe in Rust 2024; tests run
    // single-threaded per process for this assertion to hold. The
    // restore at the end keeps the var clean across test reruns.
    unsafe { std::env::set_var("FORAGE_HUB_CACHE", cache.path()) };

    let ws = workspace::load(root).unwrap();
    let cat = ws.catalog_from_disk(&recipe_path).unwrap();
    assert!(cat.ty("DispensaryFromHub").is_some());

    match prev {
        Some(v) => unsafe { std::env::set_var("FORAGE_HUB_CACHE", v) },
        None => unsafe { std::env::remove_var("FORAGE_HUB_CACHE") },
    }
}
