//! Test for `recipe_outline_inner` — exercises the pure-Rust core the
//! wasm wrapper delegates to. The wrapper serializes the result; the
//! shape under test here matches `bindings/RecipeOutline.ts`.

use forage_wasm::recipe_outline_inner;

#[test]
fn recipe_outline_collects_top_level_step_locations() {
    let source = r#"recipe "outline"
engine http
step first {
    method "GET"
    url "https://x"
}
step second {
    method "GET"
    url "https://y"
}"#;
    let steps = recipe_outline_inner(source);
    assert_eq!(steps.len(), 2);
    assert_eq!(steps[0].name, "first");
    assert_eq!(steps[1].name, "second");
    assert!(steps[0].start_line < steps[1].start_line);
}

#[test]
fn recipe_outline_descends_into_for_loops() {
    // Steps inside a `for` body show up in the flattened outline so the
    // editor can render a glyph against each one regardless of nesting.
    let source = r#"recipe "nested"
engine http
type Item { id: String }
step list {
    method "GET"
    url "https://x"
}
for $i in $list.items[*] {
    step inner {
        method "GET"
        url "https://x"
    }
    emit Item { id ← $i.id }
}"#;
    let steps = recipe_outline_inner(source);
    let names: Vec<_> = steps.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, vec!["list", "inner"]);
}

#[test]
fn recipe_outline_is_empty_on_parse_failure() {
    let steps = recipe_outline_inner("this is not a recipe {{{");
    assert!(steps.is_empty());
}
