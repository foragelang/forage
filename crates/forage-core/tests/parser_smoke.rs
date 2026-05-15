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
    assert_eq!(r.recipe_name(), Some("tiny"));
    assert_eq!(r.engine_kind(), Some(EngineKind::Http));
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
    assert_eq!(r.engine_kind(), Some(EngineKind::Browser));
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

/// The parser accepts any number of recipe headers; the validator
/// emits `DuplicateRecipeHeader` for everything past the first.
#[test]
fn second_recipe_header_is_kept_by_parser() {
    let src = r#"
        recipe "first"
        engine http

        recipe "second"
        engine http
    "#;
    let f = parse(src).expect("parser tolerates duplicate header");
    assert_eq!(f.recipe_headers.len(), 2);
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

#[test]
fn fn_decl_parses_with_one_param() {
    let src = r#"
        recipe "ok"
        engine http
        fn double($x) { $x }
        type T { id: String }
        step s { method "GET" url "https://x.test" }
        emit T { id ← "a" }
    "#;
    let r = parse(src).expect("parse");
    assert_eq!(r.functions.len(), 1);
    assert_eq!(r.functions[0].name, "double");
    assert_eq!(r.functions[0].params, vec!["x".to_string()]);
    assert!(r.functions[0].body.bindings.is_empty());
    assert!(matches!(r.functions[0].body.result, ExtractionExpr::Path(_)));
}

#[test]
fn fn_decl_parses_with_multiple_params() {
    let src = r#"
        recipe "ok"
        engine http
        fn pair($a, $b) { $a }
        type T { id: String }
        step s { method "GET" url "https://x.test" }
        emit T { id ← "a" }
    "#;
    let r = parse(src).expect("parse");
    assert_eq!(r.functions.len(), 1);
    assert_eq!(
        r.functions[0].params,
        vec!["a".to_string(), "b".to_string()]
    );
}

#[test]
fn fn_decl_zero_params_parses() {
    let src = r#"
        recipe "ok"
        engine http
        fn answer() { 42 }
        type T { id: Int }
        step s { method "GET" url "https://x.test" }
        emit T { id ← 1 }
    "#;
    let r = parse(src).expect("parse");
    assert_eq!(r.functions.len(), 1);
    assert!(r.functions[0].params.is_empty());
}

#[test]
fn fn_decl_rejects_missing_brace() {
    let src = r#"
        recipe "bad"
        engine http
        fn broken($x) $x
        type T { id: String }
        step s { method "GET" url "https://x.test" }
        emit T { id ← "a" }
    "#;
    assert!(parse(src).is_err());
}

#[test]
fn fn_decl_rejects_non_dollar_param() {
    let src = r#"
        recipe "bad"
        engine http
        fn nope(x) { $x }
        type T { id: String }
        step s { method "GET" url "https://x.test" }
        emit T { id ← "a" }
    "#;
    assert!(parse(src).is_err());
}

#[test]
fn fn_decl_rejects_dollar_input_param() {
    // The lexer emits `$input` as `DollarInput`, not `DollarVar`, so
    // the parser is the layer that rejects it as a parameter. If a
    // future refactor folds `$input` back into `DollarVar`, the
    // ReservedParam validator branch goes dead and nothing catches it
    // — this test pins the parser-side rejection.
    let src = r#"
        recipe "bad"
        engine http
        fn nope($input) { 1 }
        type T { id: String }
        step s { method "GET" url "https://x.test" }
        emit T { id ← "a" }
    "#;
    let err = parse(src).expect_err("parser must reject $input parameter");
    let msg = err.to_string();
    assert!(
        msg.contains("$input") && msg.contains("reserved"),
        "expected message to mention '$input' and 'reserved'; got: {msg}",
    );
}

#[test]
fn fn_decl_rejects_dollar_secret_param() {
    let src = r#"
        recipe "bad"
        engine http
        fn nope($secret) { 1 }
        type T { id: String }
        step s { method "GET" url "https://x.test" }
        emit T { id ← "a" }
    "#;
    let err = parse(src).expect_err("parser must reject $secret parameter");
    let msg = err.to_string();
    assert!(
        msg.contains("$secret") && msg.contains("reserved"),
        "expected message to mention '$secret' and 'reserved'; got: {msg}",
    );
}

#[test]
fn type_level_alignment_parses_with_ontology_and_term() {
    let src = r#"
        recipe "aligned"
        engine http
        type Product aligns schema.org/Product {
            name: String
        }
        step s { method "GET" url "https://x.test" }
        emit Product { name ← "a" }
    "#;
    let r = parse(src).expect("parse");
    let ty = r.types.iter().find(|t| t.name == "Product").unwrap();
    assert_eq!(ty.alignments.len(), 1);
    assert_eq!(ty.alignments[0].ontology, "schema.org");
    assert_eq!(ty.alignments[0].term, "Product");
}

#[test]
fn type_level_alignments_accumulate_across_ontologies() {
    let src = r#"
        recipe "aligned"
        engine http
        type Product
            aligns schema.org/Product
            aligns wikidata/Q2424752
        {
            name: String
        }
        step s { method "GET" url "https://x.test" }
        emit Product { name ← "a" }
    "#;
    let r = parse(src).expect("parse");
    let ty = r.types.iter().find(|t| t.name == "Product").unwrap();
    assert_eq!(ty.alignments.len(), 2);
    assert_eq!(ty.alignments[0].ontology, "schema.org");
    assert_eq!(ty.alignments[0].term, "Product");
    assert_eq!(ty.alignments[1].ontology, "wikidata");
    assert_eq!(ty.alignments[1].term, "Q2424752");
}

#[test]
fn field_level_alignment_parses_after_optional_marker() {
    let src = r#"
        recipe "aligned"
        engine http
        type Product {
            name:        String   aligns schema.org/name
            description: String?  aligns schema.org/description
            price:       Double   aligns schema.org/offers.price
        }
        step s { method "GET" url "https://x.test" }
        emit Product { name ← "a", price ← 1.0 }
    "#;
    let r = parse(src).expect("parse");
    let ty = r.types.iter().find(|t| t.name == "Product").unwrap();
    let name = ty.field("name").unwrap();
    let description = ty.field("description").unwrap();
    let price = ty.field("price").unwrap();
    assert_eq!(name.alignment.as_ref().unwrap().ontology, "schema.org");
    assert_eq!(name.alignment.as_ref().unwrap().term, "name");
    assert!(description.optional);
    assert_eq!(
        description.alignment.as_ref().unwrap().term,
        "description"
    );
    assert_eq!(price.alignment.as_ref().unwrap().term, "offers.price");
}

#[test]
fn shared_type_carries_alignments() {
    // Alignments are independent of `share`: a workspace-shared type
    // can carry the same alignment vector as a file-local one.
    let src = r#"
        share type Product aligns schema.org/Product {
            name: String aligns schema.org/name
        }
    "#;
    let r = parse(src).expect("parse");
    let ty = r.types.iter().find(|t| t.name == "Product").unwrap();
    assert!(ty.shared);
    assert_eq!(ty.alignments.len(), 1);
    assert_eq!(ty.fields[0].alignment.as_ref().unwrap().term, "name");
}

#[test]
fn type_without_alignments_yields_empty_vectors() {
    let src = r#"
        recipe "plain"
        engine http
        type Product { name: String }
        step s { method "GET" url "https://x.test" }
        emit Product { name ← "a" }
    "#;
    let r = parse(src).expect("parse");
    let ty = r.types.iter().find(|t| t.name == "Product").unwrap();
    assert!(ty.alignments.is_empty());
    assert!(ty.fields[0].alignment.is_none());
}

#[test]
fn fn_with_pipe_body_round_trips_through_ast() {
    let src = r#"
        recipe "ok"
        engine http
        fn shouty($x) { $x | upper | trim }
        type T { id: String }
        step s { method "GET" url "https://x.test" }
        emit T { id ← "a" }
    "#;
    let r = parse(src).expect("parse");
    let body = &r.functions[0].body;
    assert!(body.bindings.is_empty(), "expected no let-bindings");
    let ExtractionExpr::Pipe(_, calls) = &body.result else {
        panic!("expected pipe, got {:?}", body.result);
    };
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].name, "upper");
    assert_eq!(calls[1].name, "trim");
}

#[test]
fn parses_single_type_output_decl() {
    let src = r#"
        recipe "single"
        engine http
        output Product
        type Product { id: String }
        step list { method "GET" url "https://x.test" }
        for $p in $list[*] { emit Product { id ← $p.id } }
    "#;
    let r = parse(src).expect("parse");
    let out = r.output.expect("output decl");
    assert_eq!(out.types, vec!["Product".to_string()]);
    assert!(out.span.start < out.span.end);
}

#[test]
fn parses_multi_type_output_decl() {
    let src = r#"
        recipe "multi"
        engine http
        output Product | Variant | PriceObservation
        type Product { id: String }
        type Variant { id: String }
        type PriceObservation { id: String }
        step list { method "GET" url "https://x.test" }
        for $p in $list[*] {
            emit Product { id ← $p.id }
            emit Variant { id ← $p.id }
            emit PriceObservation { id ← $p.id }
        }
    "#;
    let r = parse(src).expect("parse");
    let out = r.output.expect("output decl");
    assert_eq!(
        out.types,
        vec![
            "Product".to_string(),
            "Variant".to_string(),
            "PriceObservation".to_string(),
        ],
    );
}

#[test]
fn output_decl_carries_span_to_its_clause() {
    let src = "recipe \"spans\"\nengine http\noutput Product | Variant\ntype Product { id: String }\ntype Variant { id: String }\n";
    let r = parse(src).expect("parse");
    let out = r.output.expect("output decl");
    let text = &src[out.span.clone()];
    assert!(text.starts_with("output Product"), "got {text:?}");
    assert!(text.ends_with("Variant"), "got {text:?}");
}

#[test]
fn output_decl_without_types_yields_empty_list() {
    // `output` alone is accepted by the parser; the validator surfaces
    // `EmptyOutput`. The next top-level form still parses normally.
    let src = r#"
        recipe "empty"
        engine http
        output
        type Item { id: String }
        step list { method "GET" url "https://x.test" }
        for $i in $list[*] { emit Item { id ← $i.id } }
    "#;
    let r = parse(src).expect("parse");
    let out = r.output.expect("output decl present even when empty");
    assert!(out.types.is_empty());
}

#[test]
fn duplicate_output_decl_is_a_parse_error() {
    let src = r#"
        recipe "dup"
        engine http
        output Item
        output Item
        type Item { id: String }
    "#;
    assert!(parse(src).is_err());
}
