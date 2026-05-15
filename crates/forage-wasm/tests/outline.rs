//! Test for `recipe_outline_inner` — exercises the pure-Rust core the
//! wasm wrapper delegates to. The wrapper serializes the result; the
//! shape under test here matches `bindings/RecipeOutline.ts`.

use forage_wasm::{PausePoint, recipe_outline_inner};

fn name_of(p: &PausePoint) -> &str {
    match p {
        PausePoint::Step { name, .. } => name.as_str(),
        PausePoint::Emit { type_name, .. } => type_name.as_str(),
        PausePoint::For { variable, .. } => variable.as_str(),
    }
}

fn start_line_of(p: &PausePoint) -> u32 {
    match p {
        PausePoint::Step { start_line, .. } => *start_line,
        PausePoint::Emit { start_line, .. } => *start_line,
        PausePoint::For { start_line, .. } => *start_line,
    }
}

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
    let points = recipe_outline_inner(source);
    let steps: Vec<_> = points
        .iter()
        .filter(|p| matches!(p, PausePoint::Step { .. }))
        .collect();
    assert_eq!(steps.len(), 2);
    assert_eq!(name_of(steps[0]), "first");
    assert_eq!(name_of(steps[1]), "second");
    assert!(start_line_of(steps[0]) < start_line_of(steps[1]));
}

#[test]
fn recipe_outline_descends_into_for_loops() {
    // Steps inside a `for` body show up in the flattened outline so the
    // editor can render a glyph against each one regardless of nesting.
    // `emit` and `for` are pause-able now too, so they appear in the
    // outline alongside steps.
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
    let points = recipe_outline_inner(source);
    let names: Vec<_> = points.iter().map(name_of).collect();
    // `list` (step), `$i` (for), `inner` (step), `Item` (emit) in source order.
    assert_eq!(names, vec!["list", "i", "inner", "Item"]);
}

#[test]
fn recipe_outline_is_empty_on_parse_failure() {
    let points = recipe_outline_inner("this is not a recipe {{{");
    assert!(points.is_empty());
}
