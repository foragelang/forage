# User-defined functions

Forage recipes can declare named transforms inline with the `fn` form.
A user-defined function is a name, a parameter list, and a single
expression body. The engine treats it identically to a built-in
transform at every call site — pipe (`$x |> myFn`) or direct call
(`myFn($x, $y)`).

```forage
fn shout($x) { $x | upper | trim }

fn variantKey($name) {
    case $name of {
        "Half Ounce" → "half_ounce"
        "Ounce"      → "ounce"
    }
}
```

## Syntax

```text
fn <name>( <$param>, ... ) { <expression> }
```

- `<name>` follows the same lexical rules as a built-in transform —
  lowercase identifier, camelCase by convention.
- Each parameter is a `$<ident>` token. The leading `$` is required.
  Zero parameters is allowed: `fn now() { … }`.
- The body is a single `ExtractionExpr` — the same grammar used at any
  call site. Branching uses `case … of`; composition uses pipes;
  templates and literals follow the standard rules.

## Calling

```forage
// Pipe call — the pipe head becomes param 0.
$name | shout

// Pipe call with extra args — head + arg1 + arg2.
$variant.size | normalizeOzToGrams($variant.unit)

// Direct call — every arg is explicit.
shout($variant.label)
```

Pipe calls always pass the pipe head as the function's first parameter.
Direct calls pass every argument explicitly: `myFn(a, b, c)` binds
`a → param0`, `b → param1`, `c → param2`. This differs from the
historical built-in convention (where the current scope value
implicitly fills the head) — user functions are explicit at the
boundary.

## Scope

Function bodies see only their parameters plus the recipe-level
constants `$input.*` and `$secret.*`. They do **not** see:

- For-loop variables (`for $x in …`).
- `emit … as $v` bindings.
- Step result bindings (`$<stepName>`).

A function is a closed unit. If you need data from the caller's scope,
pass it explicitly as a parameter.

```forage
fn tag($x) {
    "{$secret.token}:{$input.mode}:{$x}"
}
```

## Namespace and resolution order

Call-site resolution checks the user-fn declarations first, then the
built-in transform registry. Declaring a function with the same name
as a built-in shadows the built-in — useful for stubbing in tests,
dangerous in production. The validator surfaces the shadow as a
warning, not an error.

```forage
fn lower($x) { $x }   // warning: shadows the built-in `lower`
```

## Forward references and recursion

The validator collects all function names before checking any body, so
functions may call functions declared later in the file.

Direct self-reference (a body calls the enclosing function by name)
emits a `RecursiveFunction` warning. The recipe still builds — the
runtime has no recursion guard, so a self-call without a base case
will not terminate. Mutual recursion across two functions compiles
silently for now.

## Limits

- Bodies are single expressions. No statement sequencing, no
  `let`-bindings, no early return. Use `case … of`, pipes, and
  templates for control flow.
- Parameters are dynamically typed (no annotations). The validator
  catches arity mismatches but not type mismatches.
- Functions cannot capture for-loop or `as` bindings.
- Workspace-shared functions (functions in a declarations file) are
  not yet supported — declarations files carry only types and enums.
