//! Forage expression / template / transform evaluator.
//!
//! Public surface: `Evaluator::new(registry)` then
//! `evaluator.eval_extraction(expr, scope) -> Result<EvalValue>`.

pub mod error;
pub mod html;
pub mod scope;
pub mod transforms;
pub mod value;

pub use error::EvalError;
pub use scope::Scope;
pub use transforms::{TransformFn, TransformRegistry, default_registry};
pub use value::EvalValue;

use crate::ast::*;

pub struct Evaluator<'r> {
    pub registry: &'r TransformRegistry,
}

impl<'r> Evaluator<'r> {
    pub fn new(registry: &'r TransformRegistry) -> Self {
        Self { registry }
    }

    pub fn eval_extraction(
        &self,
        expr: &ExtractionExpr,
        scope: &Scope,
    ) -> Result<EvalValue, EvalError> {
        match expr {
            ExtractionExpr::Path(p) => self.eval_path(p, scope),
            ExtractionExpr::Literal(j) => Ok(EvalValue::from(j.clone())),
            ExtractionExpr::Template(t) => self.render_template(t, scope).map(EvalValue::String),
            ExtractionExpr::Pipe(head, calls) => {
                let mut v = self.eval_extraction(head, scope)?;
                for call in calls {
                    v = self.apply_transform(&call.name, v, &call.args, scope)?;
                }
                Ok(v)
            }
            ExtractionExpr::Call { name, args } => {
                // Function-call form: feed the *current* binding as the head value
                // and the explicit args as args. If there's no current (e.g. top-level),
                // pass Null.
                let head = scope
                    .current
                    .clone()
                    .or_else(|| Some(EvalValue::Null))
                    .unwrap();
                self.apply_transform(name, head, args, scope)
            }
            ExtractionExpr::CaseOf {
                scrutinee,
                branches,
            } => {
                let v = self.eval_path(scrutinee, scope)?;
                let label = match &v {
                    EvalValue::Bool(b) => b.to_string(),
                    EvalValue::String(s) => s.clone(),
                    EvalValue::Int(n) => n.to_string(),
                    EvalValue::Null => "null".into(),
                    other => format!("{other:?}"),
                };
                for (l, arm) in branches {
                    if l == &label {
                        return self.eval_extraction(arm, scope);
                    }
                }
                Err(EvalError::CaseNoMatch { label })
            }
            ExtractionExpr::MapTo { path, .. } => {
                // Emission inside MapTo is handled by the snapshot/engine layer;
                // here we just resolve the path and return.
                self.eval_path(path, scope)
            }
        }
    }

    pub fn eval_path(&self, p: &PathExpr, scope: &Scope) -> Result<EvalValue, EvalError> {
        match p {
            PathExpr::Current => Ok(scope.current.clone().unwrap_or(EvalValue::Null)),
            PathExpr::Input => Ok(EvalValue::Object(
                scope
                    .inputs()
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
            )),
            PathExpr::Secret(name) => scope
                .secret(name)
                .map(|s| EvalValue::String(s.into()))
                .ok_or(EvalError::UndefinedSecret(name.clone())),
            PathExpr::Variable(name) => scope
                .lookup(name)
                .cloned()
                .ok_or(EvalError::UndefinedVariable(name.clone())),
            PathExpr::Field(base, field) => {
                // Special case: `$input.<x>` — look up the input directly.
                if let PathExpr::Input = base.as_ref() {
                    return scope
                        .input(field)
                        .cloned()
                        .ok_or(EvalError::UndefinedInput(field.clone()));
                }
                let v = self.eval_path(base, scope)?;
                Ok(field_of(&v, field))
            }
            PathExpr::OptField(base, field) => {
                if let PathExpr::Input = base.as_ref() {
                    return Ok(scope.input(field).cloned().unwrap_or(EvalValue::Null));
                }
                let v = self.eval_path(base, scope)?;
                if matches!(v, EvalValue::Null) {
                    return Ok(EvalValue::Null);
                }
                Ok(field_of(&v, field))
            }
            PathExpr::Index(base, idx) => {
                let v = self.eval_path(base, scope)?;
                match v {
                    EvalValue::Array(mut xs) => {
                        let i = if *idx < 0 {
                            (xs.len() as i64 + *idx) as usize
                        } else {
                            *idx as usize
                        };
                        if i >= xs.len() {
                            return Err(EvalError::IndexOutOfBounds {
                                idx: *idx,
                                len: xs.len(),
                            });
                        }
                        Ok(xs.swap_remove(i))
                    }
                    EvalValue::NodeList(mut xs) => {
                        let i = if *idx < 0 {
                            (xs.len() as i64 + *idx) as usize
                        } else {
                            *idx as usize
                        };
                        if i >= xs.len() {
                            return Err(EvalError::IndexOutOfBounds {
                                idx: *idx,
                                len: xs.len(),
                            });
                        }
                        Ok(EvalValue::Node(xs.swap_remove(i)))
                    }
                    EvalValue::Null => Ok(EvalValue::Null),
                    other => Err(EvalError::TypeMismatch {
                        expected: "array",
                        actual: kind_of(&other),
                    }),
                }
            }
            PathExpr::Wildcard(base) => {
                let v = self.eval_path(base, scope)?;
                match v {
                    EvalValue::Array(xs) => Ok(EvalValue::Array(xs)),
                    EvalValue::NodeList(xs) => Ok(EvalValue::NodeList(xs)),
                    EvalValue::Null => Ok(EvalValue::Array(vec![])),
                    other => Err(EvalError::WildcardOnNonArray {
                        kind: kind_of(&other),
                    }),
                }
            }
        }
    }

    pub fn render_template(&self, t: &Template, scope: &Scope) -> Result<String, EvalError> {
        let mut out = String::new();
        for part in &t.parts {
            match part {
                TemplatePart::Literal(s) => out.push_str(s),
                TemplatePart::Interp(e) => {
                    let v = self.eval_extraction(e, scope)?;
                    out.push_str(&stringify(&v));
                }
            }
        }
        Ok(out)
    }

    fn apply_transform(
        &self,
        name: &str,
        head: EvalValue,
        args: &[ExtractionExpr],
        scope: &Scope,
    ) -> Result<EvalValue, EvalError> {
        let f = self
            .registry
            .get(name)
            .ok_or_else(|| EvalError::UnknownTransform { name: name.into() })?;
        let mut resolved = Vec::with_capacity(args.len());
        for a in args {
            resolved.push(self.eval_extraction(a, scope)?);
        }
        f(head, &resolved)
    }
}

fn field_of(v: &EvalValue, field: &str) -> EvalValue {
    match v {
        EvalValue::Object(o) => o.get(field).cloned().unwrap_or(EvalValue::Null),
        // `[*].field` projects: distribute field access across array elements.
        // Without this, `terpenes[*].name | dedup` blows up because `.name`
        // returns null on an Array, then dedup chokes on null.
        EvalValue::Array(xs) => EvalValue::Array(xs.iter().map(|x| field_of(x, field)).collect()),
        _ => EvalValue::Null,
    }
}

fn kind_of(v: &EvalValue) -> &'static str {
    match v {
        EvalValue::Null => "null",
        EvalValue::Bool(_) => "bool",
        EvalValue::Int(_) => "int",
        EvalValue::Double(_) => "double",
        EvalValue::String(_) => "string",
        EvalValue::Array(_) => "array",
        EvalValue::Object(_) => "object",
        EvalValue::Node(_) => "node",
        EvalValue::NodeList(_) => "nodelist",
    }
}

/// String coercion used by templates and `toString`.
fn stringify(v: &EvalValue) -> String {
    match v {
        EvalValue::Null => String::new(),
        EvalValue::Bool(b) => b.to_string(),
        EvalValue::Int(n) => n.to_string(),
        EvalValue::Double(n) => n.to_string(),
        EvalValue::String(s) => s.clone(),
        EvalValue::Node(s) => s.clone(),
        EvalValue::Array(xs) => xs.iter().map(stringify).collect::<Vec<_>>().join(","),
        EvalValue::Object(o) => serde_json::to_string(
            &o.iter()
                .map(|(k, v)| (k.clone(), v.clone().into_json()))
                .collect::<indexmap::IndexMap<_, _>>(),
        )
        .unwrap_or_default(),
        EvalValue::NodeList(xs) => xs.join(","),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    fn eval_recipe_expr(src: &str) -> EvalValue {
        let r = parse(src).expect("parse");
        let registry = default_registry();
        let ev = Evaluator::new(registry);
        // The fixture wraps an extraction in `emit` so we can grab the expression.
        let Statement::Emit(em) = &r.body[0] else {
            panic!("expected emit")
        };
        let scope = Scope::new();
        ev.eval_extraction(&em.bindings[0].expr, &scope).unwrap()
    }

    #[test]
    fn literal_eval() {
        let v = eval_recipe_expr(
            r#"
            recipe "x" { engine http
                type T { f: String }
                emit T { f ← "hello" }
            }
        "#,
        );
        assert_eq!(v, EvalValue::String("hello".into()));
    }

    #[test]
    fn template_with_input() {
        let r = parse(
            r#"
            recipe "x" { engine http
                input name: String
                type T { f: String }
                emit T { f ← "hi {$input.name}" }
            }
        "#,
        )
        .unwrap();
        let registry = default_registry();
        let ev = Evaluator::new(registry);
        let inputs: indexmap::IndexMap<String, EvalValue> =
            [("name".to_string(), EvalValue::String("world".into()))]
                .into_iter()
                .collect();
        let scope = Scope::new().with_inputs(inputs);
        let Statement::Emit(em) = &r.body[0] else {
            panic!()
        };
        let v = ev.eval_extraction(&em.bindings[0].expr, &scope).unwrap();
        assert_eq!(v, EvalValue::String("hi world".into()));
    }

    #[test]
    fn pipe_dedup_lowercases() {
        let r = parse(
            r#"
            recipe "x" { engine http
                type T { f: String }
                emit T { f ← "HELLO" | lower }
            }
        "#,
        )
        .unwrap();
        let registry = default_registry();
        let ev = Evaluator::new(registry);
        let scope = Scope::new();
        let Statement::Emit(em) = &r.body[0] else {
            panic!()
        };
        let v = ev.eval_extraction(&em.bindings[0].expr, &scope).unwrap();
        assert_eq!(v, EvalValue::String("hello".into()));
    }

    #[test]
    fn wildcard_field_projects_over_array() {
        // `$xs[*].name` on an array of objects should yield an array of names,
        // not Null. Without field-on-array distribution, downstream collection
        // transforms (dedup, join, count) blow up because they see Null.
        let r = parse(
            r#"
            recipe "x" { engine http
                type T { fs: [String] }
                emit T { fs ← $xs[*].name | dedup }
            }
        "#,
        )
        .unwrap();
        let registry = default_registry();
        let ev = Evaluator::new(registry);
        let mut scope = Scope::new();
        let mk = |n: &str| -> EvalValue {
            EvalValue::Object(
                [("name".to_string(), EvalValue::String(n.into()))]
                    .into_iter()
                    .collect(),
            )
        };
        scope.bind(
            "xs",
            EvalValue::Array(vec![mk("alpha"), mk("beta"), mk("alpha")]),
        );
        let Statement::Emit(em) = &r.body[0] else {
            panic!()
        };
        let v = ev.eval_extraction(&em.bindings[0].expr, &scope).unwrap();
        assert_eq!(
            v,
            EvalValue::Array(vec![
                EvalValue::String("alpha".into()),
                EvalValue::String("beta".into()),
            ])
        );
    }

    #[test]
    fn opt_field_then_wildcard_collapses_to_empty_array() {
        // Reproduces the Sweed runtime error:
        //   "transform 'dedup': can only dedup arrays, got null"
        // when a recipe writes `$product.strain?.terpenes[*].name | dedup`
        // and the strain object is null. `?.terpenes` is null,
        // `[*]` widens null → [], `.name` projects → [] (now, with the
        // distribute-over-array fix), `dedup` succeeds with [].
        let r = parse(
            r#"
            recipe "x" { engine http
                type T { fs: [String] }
                emit T { fs ← $product.strain?.terpenes[*].name | dedup }
            }
        "#,
        )
        .unwrap();
        let registry = default_registry();
        let ev = Evaluator::new(registry);
        let mut scope = Scope::new();
        // Strain present but null.
        scope.bind(
            "product",
            EvalValue::Object(
                [("strain".to_string(), EvalValue::Null)]
                    .into_iter()
                    .collect(),
            ),
        );
        let Statement::Emit(em) = &r.body[0] else {
            panic!()
        };
        let v = ev.eval_extraction(&em.bindings[0].expr, &scope).unwrap();
        assert_eq!(v, EvalValue::Array(vec![]));
    }

    #[test]
    fn case_of_evaluates() {
        let r = parse(
            r#"
            recipe "x" { engine http
                input flag: Bool
                type T { f: Int }
                emit T { f ← case $input.flag of { true → 1, false → 0 } }
            }
        "#,
        )
        .unwrap();
        let registry = default_registry();
        let ev = Evaluator::new(registry);
        let inputs: indexmap::IndexMap<String, EvalValue> =
            [("flag".to_string(), EvalValue::Bool(true))]
                .into_iter()
                .collect();
        let scope = Scope::new().with_inputs(inputs);
        let Statement::Emit(em) = &r.body[0] else {
            panic!()
        };
        let v = ev.eval_extraction(&em.bindings[0].expr, &scope).unwrap();
        assert_eq!(v, EvalValue::Int(1));
    }
}
