//! Integration tests for the evaluator's user-fn machinery. Exercises
//! `TransformRegistry::with_user_fns` + `Evaluator::apply_transform`
//! against parsed recipes — same path the live engine takes, minus
//! HTTP / browser I/O.

use forage_core::ast::*;
use forage_core::eval::{TransformRegistry, default_registry};
use forage_core::parse;
use forage_core::{EvalValue, Evaluator, Scope};

/// Evaluate the first `emit`'s first binding against a recipe whose body
/// is shaped `emit T { f ← <expr> }`. The recipe's user-fn declarations
/// are layered into the registry so they're callable inside `<expr>`.
fn eval_first_binding(src: &str, scope: Scope) -> EvalValue {
    let r = parse(src).expect("parse");
    let registry = TransformRegistry::with_user_fns(default_registry(), r.functions.clone());
    let ev = Evaluator::new(&registry);
    let Statement::Emit(em) = &r.body[0] else {
        panic!("expected top-level emit");
    };
    ev.eval_extraction(&em.bindings[0].expr, &scope)
        .expect("evaluate")
}

#[test]
fn user_fn_called_via_pipe_passes_head_as_first_param() {
    let v = eval_first_binding(
        r#"
            recipe "x"
            engine http
            fn shout($x) { $x | upper }
            type T { f: String }
            emit T { f ← "hi" | shout }
        "#,
        Scope::new(),
    );
    assert_eq!(v, EvalValue::String("HI".into()));
}

#[test]
fn user_fn_called_via_direct_call_passes_all_args() {
    // Direct call: every arg is explicit. `mark($x, $y)` returns a
    // template "{x}:{y}" — calling `mark("a", "b")` should yield "a:b".
    let v = eval_first_binding(
        r#"
            recipe "x"
            engine http
            fn mark($x, $y) { "{$x}:{$y}" }
            type T { f: String }
            emit T { f ← mark("a", "b") }
        "#,
        Scope::new(),
    );
    assert_eq!(v, EvalValue::String("a:b".into()));
}

#[test]
fn user_fn_returns_object() {
    // A user fn whose body is a path that resolves to an object.
    let v = eval_first_binding(
        r#"
            recipe "x"
            engine http
            fn id($x) { $x }
            type T { f: String }
            emit T { f ← id("hello") }
        "#,
        Scope::new(),
    );
    assert_eq!(v, EvalValue::String("hello".into()));
}

#[test]
fn user_fn_can_compose_with_built_in_transform() {
    let v = eval_first_binding(
        r#"
            recipe "x"
            engine http
            fn shouty($x) { $x | upper }
            type T { f: String }
            emit T { f ← "hi there" | shouty | trim }
        "#,
        Scope::new(),
    );
    assert_eq!(v, EvalValue::String("HI THERE".into()));
}

#[test]
fn user_fn_composes_with_another_user_fn() {
    let v = eval_first_binding(
        r#"
            recipe "x"
            engine http
            fn shout($x) { $x | upper }
            fn excite($x) { "{$x}!" }
            type T { f: String }
            emit T { f ← "hi" | shout | excite }
        "#,
        Scope::new(),
    );
    assert_eq!(v, EvalValue::String("HI!".into()));
}

#[test]
fn zero_param_fn_called_via_direct_call_returns_body() {
    // A direct call `answer()` supplies no head and no args. The fn has
    // zero params; arity matches and the body evaluates as a literal.
    let v = eval_first_binding(
        r#"
            recipe "x"
            engine http
            fn answer() { 42 }
            type T { id: Int }
            emit T { id ← answer() }
        "#,
        Scope::new(),
    );
    assert_eq!(v, EvalValue::Int(42));
}

#[test]
fn zero_param_fn_called_via_pipe_is_arity_mismatch() {
    // A pipe call always carries the head as param 0; a zero-param fn
    // has no slot to bind it to, so the runtime rejects with
    // `FnArityMismatch { expected: 0, got: 1 }`. The validator should
    // catch the same case at compile time (see `validate` tests);
    // build the recipe by hand to exercise the runtime check directly.
    use forage_core::ast::FnDecl;
    use forage_core::eval::EvalError;
    let decl = FnDecl {
        name: "answer".into(),
        params: vec![],
        body: forage_core::ast::FnBody {
            bindings: vec![],
            result: ExtractionExpr::Literal(JSONValue::Int(42)),
        },
        shared: false,
        span: 0..0,
    };
    let registry = TransformRegistry::with_user_fns(default_registry(), vec![decl]);
    let ev = Evaluator::new(&registry);
    let call = ExtractionExpr::Pipe(
        Box::new(ExtractionExpr::Literal(JSONValue::String("hi".into()))),
        vec![TransformCall {
            name: "answer".into(),
            args: vec![],
        }],
    );
    let err = ev
        .eval_extraction(&call, &Scope::new())
        .expect_err("zero-param fn must reject a pipe call");
    assert_eq!(
        err,
        EvalError::FnArityMismatch {
            name: "answer".into(),
            expected: 0,
            got: 1,
        },
    );
}

#[test]
fn wrong_arity_direct_call_reports_typed_error() {
    // `mark` expects 2 args; passing 1 must surface `FnArityMismatch`
    // (not `EvalError::Generic`) so downstream tooling can pattern-match.
    use forage_core::ast::FnDecl;
    use forage_core::eval::EvalError;
    let decl = FnDecl {
        name: "mark".into(),
        params: vec!["x".into(), "y".into()],
        body: forage_core::ast::FnBody {
            bindings: vec![],
            result: ExtractionExpr::Path(PathExpr::Variable("x".into())),
        },
        shared: false,
        span: 0..0,
    };
    let registry = TransformRegistry::with_user_fns(default_registry(), vec![decl]);
    let ev = Evaluator::new(&registry);
    let call = ExtractionExpr::Call {
        name: "mark".into(),
        args: vec![ExtractionExpr::Literal(JSONValue::String("only".into()))],
    };
    let err = ev
        .eval_extraction(&call, &Scope::new())
        .expect_err("missing arg must be FnArityMismatch");
    assert_eq!(
        err,
        EvalError::FnArityMismatch {
            name: "mark".into(),
            expected: 2,
            got: 1,
        },
    );
}

#[test]
fn for_loop_var_is_not_visible_in_fn_body_at_runtime() {
    // The validator catches this at compile time, but the runtime
    // should also fail cleanly if a fn body references a parent-scope
    // variable. Build the recipe by hand to bypass validation.
    use forage_core::ast::FnDecl;
    let body = forage_core::ast::FnBody {
        bindings: vec![],
        result: ExtractionExpr::Path(PathExpr::Variable("leak".into())),
    };
    let decl = FnDecl {
        name: "leaky".into(),
        params: vec!["x".into()],
        body,
        shared: false,
        span: 0..0,
    };
    let registry = TransformRegistry::with_user_fns(default_registry(), vec![decl]);
    let ev = Evaluator::new(&registry);
    let mut scope = Scope::new();
    scope.bind("leak", EvalValue::String("visible-from-caller".into()));
    let call = ExtractionExpr::Pipe(
        Box::new(ExtractionExpr::Literal(JSONValue::String("hi".into()))),
        vec![TransformCall {
            name: "leaky".into(),
            args: vec![],
        }],
    );
    let res = ev.eval_extraction(&call, &scope);
    assert!(
        res.is_err(),
        "fn body must not see caller's bindings; got {res:?}",
    );
}
