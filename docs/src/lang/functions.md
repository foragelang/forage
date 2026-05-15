# User-defined functions

Forage files can declare named transforms with the `fn` form. A
user-defined function is a name, a parameter list, and a single
expression body. The engine treats it identically to a built-in
transform at every call site — pipe (`$x |> myFn`) or direct call
(`myFn($x, $y)`).

```forage
fn shout($x) { $x | upper | trim }

share fn variantKey($name) {
    case $name of {
        "Half Ounce" → "half_ounce"
        "Ounce"      → "ounce"
    }
}
```

Like `type` and `enum`, a `fn` is **file-scoped** by default — visible
only to other declarations in the same file. Prefix with `share` to
publish it to the workspace catalog visible to every other `.forage`
file.

## Syntax

```text
'share'? 'fn' <name>( <$param>, ... ) {
    (let <$name> = <expression>)*
    <trailing_expression>
}
```

- `<name>` follows the same lexical rules as a built-in transform —
  lowercase identifier, camelCase by convention.
- Each parameter is a `$<ident>` token. The leading `$` is required.
  Zero parameters is allowed: `fn now() { … }`.
- The body is a sequence of `let <$name> = <expression>` bindings
  followed by exactly one trailing expression. The trailing expression
  is the function's return value. Let-bindings are fn-body-only — not
  legal in step bodies, emit fields, or top-level expressions.

```forage
fn parseSize($s) {
    let $m = $s | match(/([0-9.]+)\s*([a-zA-Z]*)/)
    case $m.matched of {
        true → { value: $m.captures[1] | parseFloat, unit: $m.captures[2] | lowercase }
        false → null
    }
}
```

## Calling

```forage
// Pipe call — the pipe head becomes param 0.
$name | shout

// Pipe call with extra args — head + arg1.
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

## Let-bindings

Each `let $name = <expression>` adds `$name` to the function-local
scope. Later bindings see earlier ones; the trailing expression sees
them all. Single-assignment — declaring the same name twice is
`DuplicateLetBinding`; shadowing a parameter is `LetShadowsParam`.

```forage
fn normalize($variant) {
    let $size = $variant.label | parseSize
    let $grams = $size.value | normalizeOzToGrams($size.unit)
    { value: $grams, unit: "g" }
}
```

## Limits

- Parameters are dynamically typed (no annotations). The validator
  catches arity mismatches but not type mismatches.
- Functions cannot capture for-loop or `as` bindings.
