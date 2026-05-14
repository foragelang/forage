//! Parser smoke tests over a handful of representative recipes.

use forage_core::ast::*;
use forage_core::parse;

const TINY_HTTP: &str = r#"
recipe "tiny"
engine http

type Item {
    id: String,
    name: String,
}

input limit: Int

step list {
    method "GET"
    url    "https://example.com/items?limit={$input.limit}"
}

for $i in $list.items[*] {
    emit Item {
        id   ← $i.id
        name ← $i.name
    }
}

expect { records.where(typeName == "Item").count >= 1 }
"#;

#[test]
fn parses_tiny_http_recipe() {
    let r = parse(TINY_HTTP).expect("parse");
    assert_eq!(r.name, "tiny");
    assert_eq!(r.engine_kind, EngineKind::Http);
    assert_eq!(r.types.len(), 1);
    assert_eq!(r.types[0].name, "Item");
    assert_eq!(r.types[0].fields.len(), 2);
    assert_eq!(r.inputs.len(), 1);
    assert_eq!(r.inputs[0].name, "limit");
    assert!(matches!(r.inputs[0].ty, FieldType::Int));
    assert_eq!(r.body.len(), 2); // step + for
    assert_eq!(r.expectations.len(), 1);
}

const TINY_BROWSER: &str = r#"
recipe "tiny-browser"
engine browser

type Film {
    title: String,
    url:   String?,
}

browser {
    initialURL: "https://example.com"
    observe:    "example.com"
    paginate browserPaginate.scroll {
        until: noProgressFor(2)
        maxIterations: 5
        iterationDelay: 1.5
    }
    captures.document {
        for $poster in $ {
            emit Film {
                title ← $poster.title
                url   ← $poster.url
            }
        }
    }
}

expect { records.where(typeName == "Film").count > 0 }
"#;

#[test]
fn parses_tiny_browser_recipe() {
    let r = parse(TINY_BROWSER).expect("parse");
    assert_eq!(r.engine_kind, EngineKind::Browser);
    let b = r.browser.expect("browser block");
    assert_eq!(b.observe, "example.com");
    assert_eq!(b.pagination.mode, BrowserPaginationMode::Scroll);
    assert_eq!(b.pagination.max_iterations, 5);
    assert!(b.document_capture.is_some());
}

#[test]
fn template_interpolation_renders_to_parts() {
    let r = parse(TINY_HTTP).expect("parse");
    let Statement::Step(step) = &r.body[0] else {
        panic!("expected step")
    };
    // url template: "https://example.com/items?limit={$input.limit}"
    let parts = &step.request.url.parts;
    assert!(parts.len() >= 2);
    // Last part should be an interpolation referring to $input.limit.
    assert!(matches!(parts.last(), Some(TemplatePart::Interp(_))));
}

#[test]
fn import_keyword_is_rejected() {
    // `import` is no longer a keyword; deps live in `forage.toml`.
    // The lexer treats it as an identifier now, which the parser
    // refuses at the top level.
    let src = r#"
        import alice/zen-leaf
        recipe "uses-import"
        engine http
    "#;
    assert!(parse(src).is_err());
}

#[test]
fn ast_nodes_carry_byte_spans() {
    // Without spans on AST nodes, Studio + the LSP can't anchor
    // diagnostics or breakpoints at the right line. Pin the parser to
    // fill `span` on every locatable node: spans must be non-empty and
    // the slice they cover must be the construct's source text.
    let src = r#"recipe "spans"
engine http
type Item { id: String }
input term: String
step list {
    method "GET"
    url    "https://api.example.com/items"
}
for $i in $list.items[*] {
    emit Item { id ← $i.id }
}
"#;
    let r = parse(src).expect("parse");

    assert_eq!(r.types.len(), 1);
    let ty_span = &r.types[0].span;
    assert!(ty_span.start < ty_span.end, "type span empty: {ty_span:?}");
    let ty_text = &src[ty_span.clone()];
    assert!(ty_text.starts_with("type Item"), "got {ty_text:?}");
    assert!(ty_text.ends_with('}'));

    assert_eq!(r.inputs.len(), 1);
    let in_span = &r.inputs[0].span;
    assert_eq!(&src[in_span.clone()], "input term: String");

    let Statement::Step(step) = &r.body[0] else {
        panic!("expected step")
    };
    let step_text = &src[step.span.clone()];
    assert!(step_text.starts_with("step list"), "got {step_text:?}");
    assert!(step_text.ends_with('}'));

    let Statement::ForLoop {
        span: for_span,
        body: for_body,
        ..
    } = &r.body[1]
    else {
        panic!("expected for-loop")
    };
    let for_text = &src[for_span.clone()];
    assert!(for_text.starts_with("for $i in"), "got {for_text:?}");
    assert!(for_text.ends_with('}'));

    let Statement::Emit(em) = &for_body[0] else {
        panic!("expected emit")
    };
    let em_text = &src[em.span.clone()];
    assert!(em_text.starts_with("emit Item"), "got {em_text:?}");
    assert!(em_text.ends_with('}'));
}

/// Regression: the recipe header is flat. A leftover `{` after the name
/// (the old block syntax) must be rejected — otherwise stale recipes would
/// parse "by accident" once the parser tolerates the brace.
#[test]
fn legacy_block_syntax_is_rejected() {
    let src = r#"
        recipe "old" {
            engine http
        }
    "#;
    let err = parse(src).expect_err("legacy block syntax must not parse");
    let msg = format!("{err}");
    assert!(
        msg.contains("engine") || msg.contains("'{'") || msg.contains("'}'") || msg.contains("{"),
        "unexpected error: {msg}"
    );
}

/// Regression: two `recipe` headers in one file is a hard error — the file
/// IS the recipe, so a second header is meaningless and almost certainly
/// indicates copy-paste rot.
#[test]
fn second_recipe_header_is_rejected() {
    let src = r#"
        recipe "first"
        engine http

        recipe "second"
        engine http
    "#;
    let err = parse(src).expect_err("second recipe header must not parse");
    let msg = format!("{err}");
    assert!(
        msg.contains("only declare one recipe") || msg.contains("recipe"),
        "unexpected error: {msg}"
    );
}

#[test]
fn parses_ref_field_type() {
    let src = r#"
        recipe "refs"
        engine http
        type Product { id: String }
        type Variant {
            product: Ref<Product>
            id:      String
        }
        step list { method "GET" url "https://x.test" }
        for $p in $list[*] {
            emit Product { id ← $p.id } as $prod
            emit Variant { product ← $prod, id ← $p.id }
        }
    "#;
    let r = parse(src).expect("parse");
    let variant = r.types.iter().find(|t| t.name == "Variant").unwrap();
    let product_field = variant.field("product").unwrap();
    match &product_field.ty {
        FieldType::Ref(target) => assert_eq!(target, "Product"),
        other => panic!("expected Ref<Product>, got {other:?}"),
    }
}

#[test]
fn parses_optional_ref_field_type() {
    let src = r#"
        recipe "refs"
        engine http
        type Product { id: String }
        type Variant {
            product: Ref<Product>?
            id:      String
        }
        step list { method "GET" url "https://x.test" }
        for $p in $list[*] {
            emit Variant { id ← $p.id }
        }
    "#;
    let r = parse(src).expect("parse");
    let variant = r.types.iter().find(|t| t.name == "Variant").unwrap();
    let product_field = variant.field("product").unwrap();
    assert!(product_field.optional);
    assert!(matches!(&product_field.ty, FieldType::Ref(t) if t == "Product"));
}

#[test]
fn parses_emit_with_as_binding() {
    let src = r#"
        recipe "binds"
        engine http
        type Item { id: String }
        step list { method "GET" url "https://x.test" }
        for $i in $list[*] {
            emit Item { id ← $i.id } as $it
        }
    "#;
    let r = parse(src).expect("parse");
    let Statement::ForLoop { body, .. } = &r.body[1] else {
        panic!("expected for-loop");
    };
    let Statement::Emit(em) = &body[0] else {
        panic!("expected emit");
    };
    assert_eq!(em.bind_name.as_deref(), Some("it"));
}

#[test]
fn parses_emit_without_as_binding() {
    let src = r#"
        recipe "no-binds"
        engine http
        type Item { id: String }
        step list { method "GET" url "https://x.test" }
        for $i in $list[*] {
            emit Item { id ← $i.id }
        }
    "#;
    let r = parse(src).expect("parse");
    let Statement::ForLoop { body, .. } = &r.body[1] else {
        panic!("expected for-loop");
    };
    let Statement::Emit(em) = &body[0] else {
        panic!("expected emit");
    };
    assert!(em.bind_name.is_none());
}

#[test]
fn ref_without_close_angle_fails_parse() {
    // The parser must reject `Ref<Foo` (no closing `>`) — otherwise the
    // mistake silently degrades to a record reference and the validator
    // emits a misleading downstream error.
    let src = r#"
        recipe "bad"
        engine http
        type T { f: Ref<Foo }
        step s { method "GET" url "https://x.test" }
    "#;
    assert!(parse(src).is_err());
}

#[test]
fn as_without_dollar_fails_parse() {
    let src = r#"
        recipe "bad"
        engine http
        type T { id: String }
        step s { method "GET" url "https://x.test" }
        for $i in $s[*] {
            emit T { id ← $i.id } as bareName
        }
    "#;
    assert!(parse(src).is_err());
}
