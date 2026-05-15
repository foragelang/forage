//! Smoke tests for AST construction + JSON round-trip.

use forage_core::ast::*;

fn empty_file() -> ForageFile {
    ForageFile {
        recipe_headers: vec![RecipeHeader {
            name: "hello".into(),
            engine_kind: EngineKind::Http,
            span: 0..0,
        }],
        types: vec![],
        enums: vec![],
        inputs: vec![],
        emits: None,
        secrets: vec![],
        functions: vec![],
        auth: None,
        browser: None,
        body: RecipeBody::Empty,
        expectations: vec![],
    }
}

#[test]
fn forage_file_with_recipe_header_serializes() {
    let r = empty_file();
    let json = serde_json::to_string(&r).unwrap();
    let back: ForageFile = serde_json::from_str(&json).unwrap();
    assert_eq!(r, back);
}

#[test]
fn header_less_forage_file_serializes() {
    let f = ForageFile {
        recipe_headers: Vec::new(),
        types: vec![RecipeType {
            name: "Dispensary".into(),
            fields: vec![RecipeField {
                name: "id".into(),
                ty: FieldType::String,
                optional: false,
                alignment: None,
            }],
            shared: true,
            alignments: vec![],
            extends: None,
            span: 0..0,
        }],
        enums: vec![],
        inputs: vec![],
        emits: None,
        secrets: vec![],
        functions: vec![],
        auth: None,
        browser: None,
        body: RecipeBody::Empty,
        expectations: vec![],
    };
    let json = serde_json::to_string(&f).unwrap();
    let back: ForageFile = serde_json::from_str(&json).unwrap();
    assert_eq!(f, back);
}

#[test]
fn typed_record_with_optional_field() {
    let ty = RecipeType {
        name: "Product".into(),
        fields: vec![
            RecipeField {
                name: "id".into(),
                ty: FieldType::String,
                optional: false,
                alignment: None,
            },
            RecipeField {
                name: "brand".into(),
                ty: FieldType::String,
                optional: true,
                alignment: None,
            },
        ],
        shared: false,
        alignments: vec![],
        extends: None,
        span: 0..0,
    };
    assert!(ty.field("id").is_some());
    assert!(ty.field("brand").is_some());
    assert!(ty.field("nope").is_none());
}

#[test]
fn path_expr_secrets() {
    let p = PathExpr::Field(Box::new(PathExpr::Secret("apiKey".into())), "value".into());
    assert_eq!(p.referenced_secrets(), vec!["apiKey".to_string()]);

    let p = PathExpr::Index(Box::new(PathExpr::Variable("xs".into())), 0);
    assert!(p.referenced_secrets().is_empty());
}

#[test]
fn template_literal_constructor() {
    let t = Template::literal("hello");
    assert_eq!(t.parts.len(), 1);
    matches!(t.parts[0], TemplatePart::Literal(_));
}

#[test]
fn jsonvalue_from_conversions() {
    assert_eq!(JSONValue::from(true), JSONValue::Bool(true));
    assert_eq!(JSONValue::from(42i64), JSONValue::Int(42));
    assert_eq!(JSONValue::from(2.5), JSONValue::Double(2.5));
    assert_eq!(JSONValue::from("hi"), JSONValue::String("hi".into()));
}
