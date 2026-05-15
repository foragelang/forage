# Forage expression language — reference shape

Reference for the post-cannabis-migration expression grammar. Describes
the language as it is today; the cannabis transforms that lived as
engine built-ins moved out into user-defined functions in the recipes
that use them.

## Expression grammar

```text
extraction      = pipe
pipe            = additive ('|' transform_call)*
additive        = multiplicative (('+'|'-') multiplicative)*
multiplicative  = unary (('*'|'/'|'%') unary)*
unary           = '-' unary | postfix
postfix         = primary ('[' extraction ']')*   # see "indexing rules"
primary         = path
                | literal
                | call '(' args ')'
                | '(' extraction ')'
                | 'case' path 'of' '{' arm* '}'
                | struct_literal
                | regex_literal
                | template
struct_literal  = '{' (field_name ':' extraction (','|';')?)* '}'
regex_literal   = '/' pattern '/' flags          # /pat/imsu
arm             = case_label '→' extraction (','|';')?
case_label      = ident | type_name | keyword | bool | null | int | str | '_'
```

Pipes sit at the lowest precedence so `$x * 28 | toString` reads as
`($x * 28) | toString`. Arithmetic + unary `-` use Rust-like ordering.

## Numeric coercion

`Int op Int` for `+`, `-`, `*` stays `Int` unless the checked op
overflows, in which case the result promotes to `Double`. `/` always
returns `Double` (no integer division — `1/2` is `0.5`). `%` with two
`Int` stays `Int`; either side `Double` promotes the result. Mixed
`Double op _` always returns `Double`. `null` or non-numeric types are
`EvalError::TypeMismatch`. Division by zero (incl. modulo by zero) is
`EvalError::ArithmeticDomain` — no `Infinity` or silent `NaN`.

`String + String` concatenates. No other implicit string coercion.

## Regex literals

`/pattern/flags`. Supported flags:

- `i` case-insensitive
- `m` multi-line (`^`/`$` match line boundaries)
- `s` dot matches newline
- `u` Unicode-aware

The pattern is the [`regex`
crate](https://docs.rs/regex) dialect — no lookahead, no
backreferences. Patterns compile at parse time;
`ParseError::InvalidRegex` / `InvalidRegexFlag` fire with a span if
something's wrong, never at runtime.

Regex values are intermediate — they consume into `match`, `matches`,
`replaceAll` and must not land on emit fields. `EvalValue::Regex`
deliberately does not implement `Serialize`; reaching `into_json`
with a regex value panics.

## String built-ins

`lower`/`lowercase`, `upper`/`uppercase`, `trim`, `capitalize`,
`titleCase`, `replace(from, to)` (literal substring), `split(sep)`.

## Regex transforms

- `match(/p/)` → `{matched: Bool, captures: [String?]}`. `captures[0]`
  is the full match; `[1..]` are groups; `null` for unmatched groups.
- `matches(/p/)` → `Bool`.
- `replaceAll(/p/, "rep")` → `String`. `$1`, `$2`, `$&` work inside
  the replacement.

## Struct literals

Inline object value, same field syntax as `emit`. `{ x: $a, y: $b |
upper }`. Duplicate fields are `DuplicateStructField` (validator),
`EvalError::DuplicateStructField` (runtime, if the validator was
skipped).

## Array indexing

The path-level form (`$x.range[0]`) stays null-tolerant — that's the
contract for scraping records with optional fields. The
expression-level form (postfix `[expr]` on the result of a call or
struct literal) is strict: `EvalError::IndexOutOfBounds` for an
out-of-range index, `EvalError::InvalidIndexBase` when the base isn't
an array.

## `case … of`

```forage
case $scrutinee of {
    "indica" → "INDICA"
    "hybrid" → "HYBRID"
    _ → $scrutinee | lowercase
}
```

The scrutinee is a path expression. Arm labels are scalar literals
(`true`, `false`, `null`, ints, strings) or enum variant names. The
`_` label is a catch-all default. Enum-typed scrutinees still get the
exhaustive-coverage warning unless `_` is present.

## Function bodies — let-bindings

A `fn` body is `(let $name = extraction)* trailing_expression`. Each
binding adds to the function-local scope; later bindings see earlier
ones; the trailing expression sees them all. Single-assignment —
`DuplicateLetBinding` for redeclaration, `LetShadowsParam` for
parameter shadowing.

```forage
fn parseSize($s) {
    let $m = $s | match(/([0-9.]+)\s*([a-zA-Z]*)/)
    case $m.matched of {
        true → { value: $m.captures[1] | parseFloat, unit: $m.captures[2] | lowercase }
        false → null
    }
}
```

## Cross-implementation parity

`crates/forage-test/fixtures/` is the contract any future
implementation (reborn TS port, Python port, …) holds to. Today the
only consumer is the Rust core compiled to WebAssembly that powers
Studio + the hub IDE; the previous TS port was retired in the
cannabis-migration push.
