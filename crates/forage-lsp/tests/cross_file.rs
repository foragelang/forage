//! Cross-file LSP smoke: a recipe that references a `share`d type
//! declared in a sibling file validates clean, and editing the sharing
//! file republishes diagnostics for the recipe.
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
fn recipe_resolves_share_type_declared_in_sibling_file() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(
        &root.join("forage.toml"),
        "description = \"\"\ncategory = \"\"\ntags = []\n",
    );
    let shared_path = root.join("shared.forage");
    write(&shared_path, "share type Item { id: String }\n");
    let recipe_path = root.join("rec.forage");
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
    store.upsert(shared_uri, fs::read_to_string(&shared_path).unwrap());

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
        "recipe with cross-file share type ought to validate clean; got: {errors:?}"
    );
}

#[test]
fn editing_share_decl_revalidates_dependent_recipe() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(
        &root.join("forage.toml"),
        "description = \"\"\ncategory = \"\"\ntags = []\n",
    );
    let shared_path = root.join("shared.forage");
    write(&shared_path, "share type Item { id: String }\n");
    let recipe_path = root.join("rec.forage");
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

    store.upsert(
        shared_uri.clone(),
        fs::read_to_string(&shared_path).unwrap(),
    );
    store.upsert(
        recipe_uri.clone(),
        fs::read_to_string(&recipe_path).unwrap(),
    );

    // Now edit the sharing file: rename `Item` → `Renamed`. The
    // recipe's `emit Item { … }` should now fail validation. The LSP
    // server normally fires `refresh_workspace` after a workspace
    // edit; the test calls it explicitly.
    let edited = "share type Renamed { id: String }\n";
    fs::write(&shared_path, edited).unwrap();
    store.upsert(shared_uri, edited.into());

    // The store records the workspace_root that `discover` returned —
    // walk to it via the same path the docstore took.
    let ws_root = forage_core::workspace::discover(&shared_path)
        .expect("workspace")
        .root;
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

/// Two files declaring `share type Item { … }` collide at the
/// workspace level — `DuplicateSharedDeclaration` fires on both
/// sharing files. The conflict surfaces on the colliding files
/// themselves, not on a dependent recipe (the recipe gets one of the
/// two `Item` definitions and validates against it).
#[test]
fn duplicate_share_type_across_files_surfaces_through_lsp() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(
        &root.join("forage.toml"),
        "description = \"\"\ncategory = \"\"\ntags = []\n",
    );
    let a_path = root.join("a.forage");
    let b_path = root.join("b.forage");
    write(&a_path, "share type Item { id: String }\n");
    write(&b_path, "share type Item { id: String }\n");

    let store = DocStore::new();
    let a_uri = Url::from_file_path(&a_path).unwrap();
    let b_uri = Url::from_file_path(&b_path).unwrap();

    // Opening A surfaces the collision against B (read from disk by
    // the cross-file pass).
    let a_diags = store.upsert(a_uri.clone(), fs::read_to_string(&a_path).unwrap());
    let a_dup: Vec<_> = a_diags
        .iter()
        .filter(|d| {
            d.severity == Some(DiagnosticSeverity::ERROR) && d.message.contains("share type 'Item'")
        })
        .collect();
    assert_eq!(
        a_dup.len(),
        1,
        "expected DuplicateSharedDeclaration on a.forage; got {a_diags:?}",
    );

    // Opening B reciprocates.
    let b_diags = store.upsert(b_uri.clone(), fs::read_to_string(&b_path).unwrap());
    let b_dup: Vec<_> = b_diags
        .iter()
        .filter(|d| {
            d.severity == Some(DiagnosticSeverity::ERROR) && d.message.contains("share type 'Item'")
        })
        .collect();
    assert_eq!(
        b_dup.len(),
        1,
        "expected DuplicateSharedDeclaration on b.forage; got {b_diags:?}",
    );

    // refresh_workspace republishes both — confirm both still carry
    // the collision, matching the LSP server's did_change fan-out.
    let ws_root = forage_core::workspace::discover(&a_path)
        .expect("workspace")
        .root;
    let refreshed = store.refresh_workspace(&ws_root);
    for (uri, diags) in refreshed {
        let dup: Vec<_> = diags
            .iter()
            .filter(|d| {
                d.severity == Some(DiagnosticSeverity::ERROR)
                    && d.message.contains("share type 'Item'")
            })
            .collect();
        assert_eq!(
            dup.len(),
            1,
            "expected DuplicateSharedDeclaration on {uri}; got {diags:?}",
        );
    }
}

/// Live-buffer reads: when a sibling file has unsaved edits in the
/// editor, the LSP must validate the dependent recipe against the
/// in-memory buffer rather than re-reading stale disk content.
#[test]
fn catalog_uses_live_buffer_for_unsaved_share_decls() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(
        &root.join("forage.toml"),
        "description = \"\"\ncategory = \"\"\ntags = []\n",
    );
    let shared_path = root.join("shared.forage");
    // Disk says `OldName`, but the editor will hold `NewName`.
    write(&shared_path, "share type OldName { id: String }\n");
    let recipe_path = root.join("rec.forage");
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
    // Open the sharing buffer with the *new* name — disk still has
    // the old name, but the recipe validator should see the live
    // buffer.
    let shared_uri = Url::from_file_path(&shared_path).unwrap();
    store.upsert(shared_uri, "share type NewName { id: String }\n".into());

    let recipe_uri = Url::from_file_path(&recipe_path).unwrap();
    let diags = store.upsert(recipe_uri, fs::read_to_string(&recipe_path).unwrap());
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
/// diagnostic on *its own buffer*, not on a sibling recipe. The
/// validator emits `DuplicateType` for any file that redeclares the
/// same type name.
#[test]
fn declarations_file_validates_its_own_duplicates() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(
        &root.join("forage.toml"),
        "description = \"\"\ncategory = \"\"\ntags = []\n",
    );
    let shared_path = root.join("shared.forage");
    write(
        &shared_path,
        "type Item { id: String }\n\
         type Item { name: String }\n",
    );

    let store = DocStore::new();
    let shared_uri = Url::from_file_path(&shared_path).unwrap();
    let diags = store.upsert(shared_uri, fs::read_to_string(&shared_path).unwrap());
    let errors: Vec<_> = diags
        .iter()
        .filter(|d| d.severity == Some(DiagnosticSeverity::ERROR))
        .collect();
    assert!(
        errors
            .iter()
            .any(|d| d.message.contains("Item") && d.message.contains("declared more than once")),
        "expected DuplicateType error on declarations file; got: {diags:?}"
    );
}

/// Two files in the same workspace both declare `share fn upper(...)`.
/// The cross-file pass must surface `DuplicateSharedDeclaration` on
/// both of them — the LSP's per-file diagnostics carry workspace-wide
/// share collisions, not just file-local validator output. Covers the
/// fn namespace; the `share type` case is covered separately.
#[test]
fn duplicate_share_fn_across_files_surfaces_through_lsp() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(
        &root.join("forage.toml"),
        "description = \"\"\ncategory = \"\"\ntags = []\n",
    );
    let a_path = root.join("a.forage");
    let b_path = root.join("b.forage");
    write(&a_path, "share fn upper($x) { $x }\n");
    write(&b_path, "share fn upper($x) { $x }\n");

    let store = DocStore::new();
    let a_uri = Url::from_file_path(&a_path).unwrap();
    let b_uri = Url::from_file_path(&b_path).unwrap();

    // Opening A on its own surfaces a collision (b.forage is read from
    // disk during the cross-file pass).
    let a_diags = store.upsert(a_uri.clone(), fs::read_to_string(&a_path).unwrap());
    let a_dup: Vec<_> = a_diags
        .iter()
        .filter(|d| {
            d.severity == Some(DiagnosticSeverity::ERROR) && d.message.contains("share fn 'upper'")
        })
        .collect();
    assert_eq!(
        a_dup.len(),
        1,
        "expected DuplicateSharedDeclaration on a.forage; got {a_diags:?}"
    );

    // Opening B then surfaces the collision on its side too.
    let b_diags = store.upsert(b_uri.clone(), fs::read_to_string(&b_path).unwrap());
    let b_dup: Vec<_> = b_diags
        .iter()
        .filter(|d| {
            d.severity == Some(DiagnosticSeverity::ERROR) && d.message.contains("share fn 'upper'")
        })
        .collect();
    assert_eq!(
        b_dup.len(),
        1,
        "expected DuplicateSharedDeclaration on b.forage; got {b_diags:?}"
    );

    // refresh_workspace republishes both — confirm both still carry
    // the collision, matching the LSP server's did_change fan-out.
    let ws_root = forage_core::workspace::discover(&a_path)
        .expect("workspace")
        .root;
    let refreshed = store.refresh_workspace(&ws_root);
    for (uri, diags) in refreshed {
        let dup: Vec<_> = diags
            .iter()
            .filter(|d| {
                d.severity == Some(DiagnosticSeverity::ERROR)
                    && d.message.contains("share fn 'upper'")
            })
            .collect();
        assert_eq!(
            dup.len(),
            1,
            "expected DuplicateSharedDeclaration on {uri}; got {diags:?}",
        );
    }
}

/// A declarations file referencing an unknown record type should
/// surface an unresolved-reference diagnostic against the workspace
/// catalog.
#[test]
fn declarations_file_flags_unknown_record_references() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(
        &root.join("forage.toml"),
        "description = \"\"\ncategory = \"\"\ntags = []\n",
    );
    let shared_path = root.join("shared.forage");
    write(&shared_path, "type Outer { other: Missing }\n");

    let store = DocStore::new();
    let shared_uri = Url::from_file_path(&shared_path).unwrap();
    let diags = store.upsert(shared_uri, fs::read_to_string(&shared_path).unwrap());
    let errors: Vec<_> = diags
        .iter()
        .filter(|d| d.severity == Some(DiagnosticSeverity::ERROR))
        .collect();
    assert!(
        errors.iter().any(|d| d.message.contains("Missing")),
        "expected unknown-type diagnostic for 'Missing'; got: {diags:?}"
    );
}
