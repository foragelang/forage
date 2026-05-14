//! Parser tests covering the workspace-file dispatch:
//!
//! - Header-less files parse as `DeclarationsFile`.
//! - Top-level non-decl forms in a declarations file are parse errors.
//! - The dropped `import` keyword is now an identifier and falls
//!   through to a top-level error.

use forage_core::ast::WorkspaceFile;
use forage_core::parse::{parse, parse_workspace_file};

#[test]
fn header_less_file_parses_as_declarations() {
    let src = r#"
        type Dispensary { id: String, name: String }
        enum MenuType { Recreational, Medical }
    "#;
    match parse_workspace_file(src).expect("parse") {
        WorkspaceFile::Declarations(d) => {
            assert_eq!(d.types.len(), 1);
            assert_eq!(d.types[0].name, "Dispensary");
            assert_eq!(d.enums.len(), 1);
            assert_eq!(d.enums[0].name, "MenuType");
        }
        WorkspaceFile::Recipe(_) => panic!("expected declarations file"),
    }
}

#[test]
fn empty_file_parses_as_empty_declarations() {
    match parse_workspace_file("").expect("parse") {
        WorkspaceFile::Declarations(d) => {
            assert!(d.types.is_empty());
            assert!(d.enums.is_empty());
        }
        WorkspaceFile::Recipe(_) => panic!("empty file should yield declarations"),
    }
}

#[test]
fn step_at_top_level_of_declarations_file_is_error() {
    let src = r#"
        type Item { id: String }
        step orphan {
            method "GET"
            url "https://example.com"
        }
    "#;
    let err = parse_workspace_file(src).expect_err("step is illegal here");
    let msg = format!("{err}");
    assert!(
        msg.contains("declarations file"),
        "expected declarations-file diagnostic, got: {msg}"
    );
}

#[test]
fn emit_at_top_level_of_declarations_file_is_error() {
    let src = "emit Item { id ← $x.id }\n";
    let err = parse_workspace_file(src).expect_err("emit is illegal here");
    let msg = format!("{err}");
    assert!(msg.contains("declarations"));
}

#[test]
fn for_loop_at_top_level_of_declarations_file_is_error() {
    let src = "for $x in $y { }\n";
    let err = parse_workspace_file(src).expect_err("for-loop is illegal here");
    let msg = format!("{err}");
    assert!(msg.contains("declarations"));
}

#[test]
fn non_decl_constructs_rejected_in_declarations_file() {
    for snippet in [
        "auth { }",
        "browser { }",
        "secret token",
        "engine http",
        "expect { records.where(typeName == \"X\").count > 0 }",
    ] {
        let err = parse_workspace_file(snippet)
            .unwrap_err_or_else_describe(format!("top-level {snippet}"));
        let msg = format!("{err}");
        assert!(msg.contains("declarations"), "{snippet}: {msg}");
    }
}

#[test]
fn import_keyword_is_no_longer_recognized() {
    // `import` is now an ordinary identifier. At the top level of a
    // declarations file, identifiers are illegal — and the rest of the
    // old `import` syntax (`author/slug`) wasn't lexable either since
    // the dedicated ref-scan path is gone. Either way, the parser
    // rejects the input.
    let src = "import xyz\n";
    let err = parse_workspace_file(src).expect_err("must not parse");
    let msg = format!("{err}");
    assert!(
        msg.contains("declarations") || msg.contains("unexpected"),
        "unexpected error: {msg}"
    );
}

#[test]
fn convenience_parse_rejects_declarations_file() {
    // The `parse` convenience demands a full recipe; a declarations
    // file passed to it surfaces a clear parse error rather than
    // returning a half-empty recipe.
    let src = "type Dispensary { id: String }\n";
    let err = parse(src).expect_err("parse must reject declarations file");
    let msg = format!("{err}");
    assert!(
        msg.contains("recipe header") || msg.contains("declarations"),
        "unexpected error: {msg}"
    );
}

trait UnwrapErrExt<E> {
    fn unwrap_err_or_else_describe(self, ctx: impl Into<String>) -> E;
}

impl<T, E> UnwrapErrExt<E> for Result<T, E> {
    fn unwrap_err_or_else_describe(self, ctx: impl Into<String>) -> E {
        match self {
            Ok(_) => panic!("expected error in {}", ctx.into()),
            Err(e) => e,
        }
    }
}
