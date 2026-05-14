//! Smoke tests for AST construction + JSON round-trip.

use forage_core::ast::*;

#[test]
fn empty_recipe_serializes() {
    let r = Recipe {
        name: "hello".into(),
        engine_kind: EngineKind::Http,
        types: vec![],
        enums: vec![],
        inputs: vec![],
        auth: None,
        body: vec![],
        browser: None,
        expectations: vec![],
        secrets: vec![],
        functions: vec![],
    };
    let json = serde_json::to_string(&r).unwrap();
    let back: Recipe = serde_json::from_str(&json).unwrap();
    assert_eq!(r, back);
}

#[test]
fn workspace_file_recipe_variant_serializes() {
    let r = Recipe {
        name: "hello".into(),
        engine_kind: EngineKind::Http,
        types: vec![],
        enums: vec![],
        inputs: vec![],
        auth: None,
        body: vec![],
        browser: None,
        expectations: vec![],
        secrets: vec![],
        functions: vec![],
    };
    let wf = WorkspaceFile::Recipe(Box::new(r));
    let json = serde_json::to_string(&wf).unwrap();
    let back: WorkspaceFile = serde_json::from_str(&json).unwrap();
    assert_eq!(wf, back);
}

#[test]
fn workspace_file_declarations_variant_serializes() {
    let d = DeclarationsFile {
        types: vec![RecipeType {
            name: "Dispensary".into(),
            fields: vec![RecipeField {
                name: "id".into(),
                ty: FieldType::String,
                optional: false,
            }],
            span: 0..0,
        }],
        enums: vec![],
    };
    let wf = WorkspaceFile::Declarations(d);
    let json = serde_json::to_string(&wf).unwrap();
    let back: WorkspaceFile = serde_json::from_str(&json).unwrap();
    assert_eq!(wf, back);
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
            },
            RecipeField {
                name: "brand".into(),
                ty: FieldType::String,
                optional: true,
            },
        ],
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
