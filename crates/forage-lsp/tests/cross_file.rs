//! Cross-file LSP smoke: a recipe that references a type declared in a
//! sibling declarations file validates clean, and editing the shared
//! declarations file republishes diagnostics for the recipe.
//!
//! Drives `DocStore` directly — the tower-lsp server hands off to the
//! same `upsert` / `refresh_workspace` calls, so testing the store
//! exercises the cross-file path without spinning a real LSP loop.

use std::fs;
use std::path::Path;

use forage_lsp::docstore::DocStore;
use tower_lsp::lsp_types::{DiagnosticSeverity, Url};

fn write(path: &Path, body: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, body).unwrap();
}

#[test]
fn recipe_resolves_type_declared_in_sibling_declarations_file() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(&root.join("forage.toml"), "");
    let shared_path = root.join("shared.forage");
    write(&shared_path, "type Item { id: String }\n");
    let recipe_path = root.join("rec").join("recipe.forage");
    write(
        &recipe_path,
        r#"recipe "rec"
engine http
step list {
    method "GET"
    url "https://example.com"
}
for $x in $list.items[*] {
    emit Item { id ← $x.id }
}
"#,
    );

    let store = DocStore::new();

    let shared_uri = Url::from_file_path(&shared_path).unwrap();
    store.upsert(
        shared_uri,
        fs::read_to_string(&shared_path).unwrap(),
    );

    let recipe_uri = Url::from_file_path(&recipe_path).unwrap();
    let diags = store.upsert(
        recipe_uri.clone(),
        fs::read_to_string(&recipe_path).unwrap(),
    );
    let errors: Vec<_> = diags
        .iter()
        .filter(|d| d.severity == Some(DiagnosticSeverity::ERROR))
        .collect();
    assert!(
        errors.is_empty(),
        "recipe with cross-file type ought to validate clean; got: {errors:?}"
    );
}

#[test]
fn editing_declarations_file_revalidates_dependent_recipe() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(&root.join("forage.toml"), "");
    let shared_path = root.join("shared.forage");
    write(&shared_path, "type Item { id: String }\n");
    let recipe_path = root.join("rec").join("recipe.forage");
    write(
        &recipe_path,
        r#"recipe "rec"
engine http
step list {
    method "GET"
    url "https://example.com"
}
for $x in $list.items[*] {
    emit Item { id ← $x.id }
}
"#,
    );

    let store = DocStore::new();
    let shared_uri = Url::from_file_path(&shared_path).unwrap();
    let recipe_uri = Url::from_file_path(&recipe_path).unwrap();

    store.upsert(shared_uri.clone(), fs::read_to_string(&shared_path).unwrap());
    store.upsert(
        recipe_uri.clone(),
        fs::read_to_string(&recipe_path).unwrap(),
    );

    // Now edit the declarations file: rename `Item` → `Renamed`. The
    // recipe's `emit Item { … }` should now fail validation. The LSP
    // server normally fires `refresh_workspace` after the
    // declarations file edit; the test calls it explicitly.
    let edited = "type Renamed { id: String }\n";
    fs::write(&shared_path, edited).unwrap();
    store.upsert(shared_uri, edited.into());

    // The store records the workspace_root that `discover` returned —
    // walk to it via the same path the docstore took.
    let ws_root =
        forage_core::workspace::discover(&shared_path).expect("workspace").root;
    let refreshed = store.refresh_workspace(&ws_root);
    let (refreshed_uri, diags) = refreshed
        .into_iter()
        .find(|(u, _)| u == &recipe_uri)
        .expect("recipe must be re-validated");
    assert_eq!(refreshed_uri, recipe_uri);
    let errors: Vec<_> = diags
        .iter()
        .filter(|d| d.severity == Some(DiagnosticSeverity::ERROR))
        .collect();
    assert!(
        errors.iter().any(|d| d.message.contains("Item")),
        "expected an UnknownType error referencing 'Item'; got: {diags:?}"
    );
}

/// When the workspace catalog itself can't be built — typically because
/// two declarations files declare the same type — the recipe must
/// surface that as a diagnostic instead of silently falling back to its
/// own recipe-local types. The user has to see the conflict to fix it.
#[test]
fn workspace_error_surfaces_as_diagnostic_on_dependent_recipe() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(&root.join("forage.toml"), "");
    write(&root.join("a.forage"), "type Item { id: String }\n");
    write(&root.join("b.forage"), "type Item { id: String }\n");
    let recipe_path = root.join("rec").join("recipe.forage");
    write(
        &recipe_path,
        r#"recipe "rec"
engine http
step list {
    method "GET"
    url "https://example.com"
}
"#,
    );

    let store = DocStore::new();
    let recipe_uri = Url::from_file_path(&recipe_path).unwrap();
    let diags = store.upsert(
        recipe_uri,
        fs::read_to_string(&recipe_path).unwrap(),
    );
    let errors: Vec<_> = diags
        .iter()
        .filter(|d| d.severity == Some(DiagnosticSeverity::ERROR))
        .collect();
    assert!(
        errors.iter().any(|d| d.message.contains("Item")
            && d.message.to_lowercase().contains("multiple")),
        "expected duplicate-type workspace error to surface as a diagnostic; got: {diags:?}"
    );
}

/// Live-buffer reads: when a sibling declarations file has unsaved
/// edits in the editor, the LSP must validate the dependent recipe
/// against the in-memory buffer rather than re-reading stale disk
/// content.
#[test]
fn catalog_uses_live_buffer_for_unsaved_declarations() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(&root.join("forage.toml"), "");
    let shared_path = root.join("shared.forage");
    // Disk says `OldName`, but the editor will hold `NewName`.
    write(&shared_path, "type OldName { id: String }\n");
    let recipe_path = root.join("rec").join("recipe.forage");
    write(
        &recipe_path,
        r#"recipe "rec"
engine http
step list {
    method "GET"
    url "https://example.com"
}
for $x in $list.items[*] {
    emit NewName { id ← $x.id }
}
"#,
    );

    let store = DocStore::new();
    // Open the declarations buffer with the *new* name — disk still
    // has the old name, but we expect the recipe validator to see the
    // live buffer.
    let shared_uri = Url::from_file_path(&shared_path).unwrap();
    store.upsert(shared_uri, "type NewName { id: String }\n".into());

    let recipe_uri = Url::from_file_path(&recipe_path).unwrap();
    let diags = store.upsert(
        recipe_uri,
        fs::read_to_string(&recipe_path).unwrap(),
    );
    let errors: Vec<_> = diags
        .iter()
        .filter(|d| d.severity == Some(DiagnosticSeverity::ERROR))
        .collect();
    assert!(
        errors.is_empty(),
        "live buffer should expose NewName; got errors: {errors:?}"
    );
}

/// A declarations file with a duplicated type name surfaces a
/// diagnostic on *its own buffer*, not on a sibling recipe.
/// Duplicate detection lives in the parser, so the diagnostic
/// arrives via the parse-error path.
#[test]
fn declarations_file_validates_its_own_duplicates() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(&root.join("forage.toml"), "");
    let shared_path = root.join("shared.forage");
    write(
        &shared_path,
        "type Item { id: String }\n\
         type Item { name: String }\n",
    );

    let store = DocStore::new();
    let shared_uri = Url::from_file_path(&shared_path).unwrap();
    let diags = store.upsert(
        shared_uri,
        fs::read_to_string(&shared_path).unwrap(),
    );
    let errors: Vec<_> = diags
        .iter()
        .filter(|d| d.severity == Some(DiagnosticSeverity::ERROR))
        .collect();
    assert!(
        errors
            .iter()
            .any(|d| d.message.contains("Item") && d.message.contains("duplicate declaration")),
        "expected duplicate-declaration error on declarations file; got: {diags:?}"
    );
}

/// A declarations file referencing an unknown record type should
/// surface an unresolved-reference diagnostic against the workspace
/// catalog.
#[test]
fn declarations_file_flags_unknown_record_references() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(&root.join("forage.toml"), "");
    let shared_path = root.join("shared.forage");
    write(
        &shared_path,
        "type Outer { other: Missing }\n",
    );

    let store = DocStore::new();
    let shared_uri = Url::from_file_path(&shared_path).unwrap();
    let diags = store.upsert(
        shared_uri,
        fs::read_to_string(&shared_path).unwrap(),
    );
    let errors: Vec<_> = diags
        .iter()
        .filter(|d| d.severity == Some(DiagnosticSeverity::ERROR))
        .collect();
    assert!(
        errors.iter().any(|d| d.message.contains("Missing")),
        "expected unknown-type diagnostic for 'Missing'; got: {diags:?}"
    );
}
