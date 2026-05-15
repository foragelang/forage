//! Parser tests covering the flat `forage_file := top_level_form*` grammar.
//!
//! - Header-less files parse cleanly; the AST `recipe_headers` slot is empty.
//! - The recipe header is just another top-level form; the parser doesn't
//!   reject second headers, duplicate types, or recipe-context forms in a
//!   header-less file — those are validator concerns now.

use forage_core::parse::parse;

#[test]
fn header_less_file_parses_as_forage_file() {
    let src = r#"
        type Dispensary { id: String, name: String }
        enum MenuType { Recreational, Medical }
    "#;
    let f = parse(src).expect("parse");
    assert!(f.recipe_headers.is_empty());
    assert_eq!(f.types.len(), 1);
    assert_eq!(f.types[0].name, "Dispensary");
    assert_eq!(f.enums.len(), 1);
    assert_eq!(f.enums[0].name, "MenuType");
}

#[test]
fn empty_file_parses() {
    let f = parse("").expect("parse");
    assert!(f.recipe_headers.is_empty());
    assert!(f.types.is_empty());
    assert!(f.enums.is_empty());
}

#[test]
fn share_prefix_marks_types_enums_and_fns() {
    let src = r#"
        share type Dispensary { id: String }
        share enum MenuType { Rec, Med }
        share fn upperId($x) { $x | upper }

        type LocalThing { id: String }
        enum LocalEnum { A, B }
        fn local_fn($x) { $x }
    "#;
    let f = parse(src).expect("parse");
    assert_eq!(f.types.len(), 2);
    let dispensary = f.types.iter().find(|t| t.name == "Dispensary").unwrap();
    let local_thing = f.types.iter().find(|t| t.name == "LocalThing").unwrap();
    assert!(dispensary.shared);
    assert!(!local_thing.shared);

    assert_eq!(f.enums.len(), 2);
    let menu = f.enums.iter().find(|e| e.name == "MenuType").unwrap();
    let local_enum = f.enums.iter().find(|e| e.name == "LocalEnum").unwrap();
    assert!(menu.shared);
    assert!(!local_enum.shared);

    assert_eq!(f.functions.len(), 2);
    let shared_fn = f.functions.iter().find(|fn_| fn_.name == "upperId").unwrap();
    let local_fn = f.functions.iter().find(|fn_| fn_.name == "local_fn").unwrap();
    assert!(shared_fn.shared);
    assert!(!local_fn.shared);
}

#[test]
fn import_keyword_is_no_longer_recognized() {
    // `import` is now an ordinary identifier. The parser refuses
    // identifiers at the top level since they're not a top-level form.
    let src = "import xyz\n";
    let err = parse(src).expect_err("must not parse");
    let msg = format!("{err}");
    assert!(msg.contains("unexpected"), "unexpected error: {msg}");
}

#[test]
fn duplicate_recipe_header_parses_and_keeps_both() {
    // The parser is permissive — a second header is a validator
    // concern. Both headers land in the AST so the validator can anchor
    // a `DuplicateRecipeHeader` issue on the duplicate.
    let src = r#"
        recipe "first"
        engine http

        recipe "second"
        engine http
    "#;
    let f = parse(src).expect("parser accepts duplicate header");
    assert_eq!(f.recipe_headers.len(), 2);
    assert_eq!(f.recipe_headers[0].name, "first");
    assert_eq!(f.recipe_headers[1].name, "second");
}

#[test]
fn share_followed_by_non_decl_is_rejected() {
    let src = "share input limit: Int\n";
    let err = parse(src).expect_err("share applies only to type/enum/fn");
    let msg = format!("{err}");
    assert!(
        msg.contains("type") && msg.contains("enum") && msg.contains("fn"),
        "expected diagnostic mentioning the valid heads after share; got: {msg}",
    );
}
