//! Workspace loader + catalog tests.

use std::fs;
use std::path::{Path, PathBuf};

use forage_core::workspace::{self, MANIFEST_NAME, TypeCatalog, Workspace, discover};
use forage_core::{parse, validate};

fn write(path: &Path, body: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, body).unwrap();
}

fn workspace_in(tmp: &tempfile::TempDir) -> &Path {
    // Minimal valid manifest for tests that don't care about its
    // contents — required fields present with empty values, no name.
    write(
        &tmp.path().join(MANIFEST_NAME),
        "description = \"\"\ncategory = \"\"\ntags = []\n",
    );
    tmp.path()
}

#[test]
fn share_decls_reach_focal_recipe() {
    let tmp = tempfile::tempdir().unwrap();
    let root = workspace_in(&tmp);
    write(
        &root.join("cannabis.forage"),
        "share type Dispensary { id: String, name: String }\n\
         share enum MenuType { Recreational, Medical }\n",
    );
    let recipe_path = root.join("trilogy.forage");
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
        "share type Product { id: String, name: String }\n",
    );
    let recipe_path = root.join("sweed.forage");
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

/// A bare (non-`share`d) `type Foo` in a sibling file is private to
/// that file. The focal recipe must not see it — only `share`d
/// declarations cross file boundaries.
#[test]
fn bare_type_in_sibling_is_invisible_to_recipe() {
    let tmp = tempfile::tempdir().unwrap();
    let root = workspace_in(&tmp);
    write(
        &root.join("cannabis.forage"),
        "type LocalThing { id: String }\n\
         share type Dispensary { id: String }\n",
    );
    let recipe_path = root.join("sweed.forage");
    write(&recipe_path, "recipe \"sweed\"\nengine http\n");
    let ws = workspace::load(root).unwrap();
    let catalog = ws.catalog_from_disk(&recipe_path).unwrap();
    assert!(
        catalog.ty("Dispensary").is_some(),
        "share type should reach the recipe",
    );
    assert!(
        catalog.ty("LocalThing").is_none(),
        "bare type must stay private to its declaring file",
    );
}

/// A file with `share type Foo` and another with bare `type Foo` is
/// not a collision: the bare declaration is private, and only the
/// `share`d one crosses file boundaries.
#[test]
fn share_and_bare_with_same_name_coexist() {
    let tmp = tempfile::tempdir().unwrap();
    let root = workspace_in(&tmp);
    write(
        &root.join("a.forage"),
        "share type Product { id: String, name: String }\n",
    );
    write(&root.join("b.forage"), "type Product { id: String }\n");
    let recipe_path = root.join("r.forage");
    write(&recipe_path, "recipe \"r\"\nengine http\n");
    let ws = workspace::load(root).unwrap();
    let catalog = ws.catalog_from_disk(&recipe_path).unwrap();
    let product = catalog.ty("Product").expect("share Product reaches recipe");
    assert_eq!(
        product.fields.len(),
        2,
        "the share type wins; bare Product is invisible",
    );
}

/// Bare `type Foo` declared in two sibling files is no longer a
/// catalog-level error — each is file-scoped to its file. The focal
/// recipe sees neither.
#[test]
fn duplicate_bare_types_across_files_are_not_an_error() {
    let tmp = tempfile::tempdir().unwrap();
    let root = workspace_in(&tmp);
    write(&root.join("a.forage"), "type Product { id: String }\n");
    write(&root.join("b.forage"), "type Product { id: String }\n");
    let recipe_path = root.join("r.forage");
    write(&recipe_path, "recipe \"r\"\nengine http\n");
    let ws = workspace::load(root).unwrap();
    let catalog = ws
        .catalog_from_disk(&recipe_path)
        .expect("bare-type duplicates are file-local, not a catalog error");
    assert!(catalog.ty("Product").is_none());
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
    let catalog = TypeCatalog::from_file(&r);
    let rep = validate(&r, &catalog, &forage_core::RecipeSignatures::default());
    assert!(!rep.has_errors(), "lonely recipe errored: {:?}", rep.issues);
}

/// A file with a `recipe "<name>" engine <kind>` header surfaces in
/// `recipes()`; a header-less file does not. The discriminator is the
/// parsed content's `recipe_header().is_some()` — file location is
/// incidental.
#[test]
fn recipes_iterator_filters_by_header_presence() {
    let tmp = tempfile::tempdir().unwrap();
    let root = workspace_in(&tmp);
    write(&root.join("shared.forage"), "type T { id: String }\n");
    write(&root.join("rec.forage"), "recipe \"rec\"\nengine http\n");
    let ws: Workspace = workspace::load(root).unwrap();
    let mut recipes: Vec<&Path> = ws.recipes().map(|r| r.path).collect();
    recipes.sort();
    assert_eq!(recipes.len(), 1);
    assert!(recipes[0].ends_with("rec.forage"));
    // Header-less file is still present in `files` so the catalog can
    // pick up its share declarations; it just doesn't surface in
    // `recipes()`.
    assert_eq!(ws.files.len(), 2);
    let header_less: Vec<&PathBuf> = ws
        .files
        .iter()
        .filter(|e| e.parsed.as_ref().is_ok_and(|f| f.recipe_header().is_none()))
        .map(|e| &e.path)
        .collect();
    assert_eq!(header_less.len(), 1);
    assert!(header_less[0].ends_with("shared.forage"));
}

#[test]
fn recipe_by_name_finds_recipe() {
    let tmp = tempfile::tempdir().unwrap();
    let root = workspace_in(&tmp);
    write(
        &root.join("alpha.forage"),
        "recipe \"alpha\"\nengine http\n",
    );
    let ws = workspace::load(root).unwrap();
    let recipe = ws.recipe_by_name("alpha").expect("recipe by name");
    assert!(recipe.path.ends_with("alpha.forage"));
    assert_eq!(recipe.name(), "alpha");
}

#[test]
#[serial_test::serial]
fn hub_cached_type_pins_contribute_types() {
    // Stand up a type cache under FORAGE_HUB_CACHE, drop a type source
    // body at the expected `<cache>/types/<author>/<Name>/<v>.forage`
    // location, pin it from the lockfile's `[types]` table, and verify
    // the catalog folds the type in.
    let tmp = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(
        &root.join(MANIFEST_NAME),
        "description = \"\"\ncategory = \"\"\ntags = []\n",
    );
    write(
        &root.join(workspace::LOCKFILE_NAME),
        "[types.\"dima/DispensaryFromHub\"]\nversion = 3\nhash = \"\"\n",
    );
    let recipe_path = root.join("rec.forage");
    write(&recipe_path, "recipe \"rec\"\nengine http\n");
    let cached = workspace::type_cache_file(cache.path(), "dima", "DispensaryFromHub", 3);
    write(
        &cached,
        "share type DispensaryFromHub {\n    id: String\n}\n",
    );

    let prev = std::env::var("FORAGE_HUB_CACHE").ok();
    // SAFETY: env mutation is unsafe in Rust 2024; the serial attribute
    // serialises this test against any other test that mutates the
    // same var.
    unsafe { std::env::set_var("FORAGE_HUB_CACHE", cache.path()) };

    let ws = workspace::load(root).unwrap();
    let cat = ws.catalog_from_disk(&recipe_path).unwrap();
    assert!(cat.ty("DispensaryFromHub").is_some());

    match prev {
        Some(v) => unsafe { std::env::set_var("FORAGE_HUB_CACHE", v) },
        None => unsafe { std::env::remove_var("FORAGE_HUB_CACHE") },
    }
}
