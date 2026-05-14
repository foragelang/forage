# Expressions and templates

Expressions live on the right-hand side of every `←` binding, every
`emit` field, every step URL/header/body, and every template
interpolation.

## Path expressions

Path expressions navigate the current scope:

```forage
$                       // the current binding ($.); inside a for-loop, the loop value
$.field                 // field access from current
$.x.y                   // nested
$.x?.y                  // optional access — yields null if any segment is null
$.x[0]                  // array index (signed; -1 = last)
$.x[*]                  // wildcard — broadens to an array of all items
$input.storeId          // recipe input
$secret.apiToken        // host-resolved secret
$<stepName>             // a previous step's response body
$<stepName>.items[*]    // typical
```

Inside a `for $x in <expr>` body, `$x` and `$.` both bind to the loop
item. After the loop ends, `$x` is out of scope.

`emit T { … } as $v` also introduces a name into scope — a typed
reference (`Ref<T>`) to the record that was just emitted. The binding
lives until the enclosing lexical block ends; subsequent emits inside
the same for-loop body (or sibling top-level statements) can refer to
`$v` when binding their own `Ref<T>` fields. See
[Types — Typed references](./types.md#typed-references--reft) for the
full type-check rules.

## Pipelines

The `|` operator threads a value through a sequence of transforms:

```forage
$variant.priceLabel | trim | parseFloat
$product.images[*].url | dedup
$product.tags[*] | titleCase
```

Each transform receives the previous value as input. Some transforms
take arguments:

```forage
$.price | default(0.0)
$.tags  | length
$variant.attrs | getField("size")
```

See [Transforms](./transforms.md) for the catalog.

## Function-call form

Where a transform takes multiple arguments or you want
call-shape syntax, use the call form:

```forage
coalesce($variant.priceSale, $variant.priceList)
getField($product.attrs, "size")
```

These are the same transforms — just bound by call instead of
pipe-feed.

## Arithmetic

Binary `+`, `-`, `*`, `/`, `%` and unary `-`. Precedence (low → high):
pipe `|` → additive `+`/`-` → multiplicative `*`/`/`/`%` → unary `-` →
postfix `[…]` → primary. `Int op Int` stays `Int` for `+`/`-`/`*`;
`/` and `%` promote to `Double` (so `1/2` is `0.5`). Any `Double` on
either side promotes the result. `String + String` concatenates.
`null` and mixed-type operations are `TypeMismatch`. Division by zero
is `ArithmeticDomain`.

```forage
$variant.unitSize.value * 28.0         // ounces → grams
$product.priceCents / 100              // cents → dollars (always Double)
"price=" + ($variant.price | toString) // string concat
```

## Regex literals

`/pattern/flags` parses to a compiled regex. Flags: `i`, `m`, `s`, `u`
(see [Transforms — Regex](./transforms.md#regex)). The literal is
intermediate — it only flows into `match` / `matches` / `replaceAll`
and never lands on a record field.

```forage
$variant.label | matches(/^(oz|ounce)$/i)
$variant.label | replaceAll(/[^a-z0-9]+/i, "-")
```

## Struct literals

An inline object value. Same field-binding shape as `emit`, used as
an expression:

```forage
{ value: $weight | parseFloat, unit: $abbr | lowercase }
```

Duplicate field names are a validator error
(`DuplicateStructField`).

## Array indexing in expressions

Bracket notation on any expression result. The path-level form
(`$x.range[0]`) stays null-tolerant for missing-record-field
ergonomics; expression-level indexing (e.g. on the output of a
function call or struct literal) raises `IndexOutOfBounds` instead.

## Case-of

`case` dispatches on an enum (or bool/string/null) value:

```forage
case $menu of {
    RECREATIONAL → $variant.priceRec
    MEDICAL      → $variant.priceMed
}
```

```forage
case $product.search_attributes.at_visible_store of {
    true  → 1.0
    false → 0.0
}
```

The validator warns when an enum's `case-of` doesn't cover every
variant. Labels that aren't part of the enum are errors.

## Templates

A string literal containing `{...}` is a template. The interpolation
body is a full expression — pipes, function calls, case-of, all in
scope:

```forage
url "https://example.com/items?store={$input.storeId}&page={$page}"
externalId ← "{$product.id}:{$weight}"
text       ← "price_{$weight | lowercase | replace(" ", "_")}"
```

Stringification rules: strings unchanged; numbers / bools to their
literal text; null → empty string; arrays → JSON; objects → JSON.

## Literals

```forage
"hello"                       // string
42                            // int
3.14                          // double
true / false                  // bool
null                          // null
1990-01-01                    // date (year-month-day; used by ageGate.dob)
```
