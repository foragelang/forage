# Adding a transform

Transforms are pure functions over `EvalValue`. Three steps:

## 1. Implement

In `crates/forage-core/src/eval/transforms.rs`, write the function:

```rust
fn shouting_case(v: EvalValue, _: &[EvalValue]) -> Result<EvalValue, EvalError> {
    let s = require_string("shoutingCase", &v)?;
    Ok(EvalValue::String(s.to_uppercase() + "!"))
}
```

The signature is `fn(EvalValue, &[EvalValue]) -> Result<EvalValue,
EvalError>`. The first argument is the value flowing in from the
pipeline; the slice is positional args. Use the existing helpers:

- `require_string("name", &v)` — coerce to a string or return a
  helpful error.
- `EvalError::TransformError { name, msg }` for domain-specific
  failures.

## 2. Register

Add it to `build_default` near the other transforms:

```rust
r.register("shoutingCase", shouting_case);
```

## 3. Surface

Add the name to two places so the validator + LSP know about it:

- `crates/forage-core/src/validate/mod.rs::BUILTIN_TRANSFORMS` — keeps
  unknown-transform validation honest.
- `apps/studio/ui/src/lib/monaco-forage.ts::BUILTIN_TRANSFORMS` — same
  list for the static Monaco completion items.

## 4. Test + document

A `#[test]` in `transforms.rs` is enough:

```rust
#[test]
fn shouting_case_yells() {
    let f = default_registry().get("shoutingCase").unwrap();
    let v = f(EvalValue::String("hi".into()), &[]).unwrap();
    assert_eq!(v, EvalValue::String("HI!".into()));
}
```

Then add a row to `docs/src/lang/transforms.md` so the language
reference picks it up.

## Naming

- `camelCase`. Consistent with the rest of the catalog and with how
  recipe authors will reference it: `$x | shoutingCase`.
- Avoid overloading existing names. `parse*` is for type coercion;
  `normalize*` is for cleanup; domain-specific helpers are namespaced
  (`janeWeightUnit`, not `weightUnit`).
- Don't take optional args unless they're genuinely optional. Prefer
  separate transforms or an explicit `default(v)` upstream.

## Where transforms run

- **CLI** (`cargo run --bin forage -- run …`) — uses the in-process
  registry built by `forage-core::eval::default_registry`.
- **Forage Studio** — same registry, embedded in the Tauri backend.
- **Web IDE** — same registry, compiled to WebAssembly via
  `forage-wasm`. Run `wasm-pack build --target web --out-dir
  ../../hub-site/forage-wasm/pkg` from `crates/forage-wasm` after
  adding a transform.
- **LSP** — pulls names from the static `BUILTIN_TRANSFORMS` list.

## Beyond builtins

The runtime currently uses a static registry — no recipe-defined
transforms, no plugins. The validator's `UnknownTransform` error
catches typos at parse time, which would be impossible against an
open registry. If the use case for recipe-author-defined transforms
emerges, the registry can grow a `register` hook the host calls;
until then it stays closed by design.
