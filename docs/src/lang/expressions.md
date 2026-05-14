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
normalizeOzToGrams($variant.unitSize?.value, $variant.unitSize?.unitAbbr)
getField($product.attrs, "size")
```

These are the same transforms — just bound by call instead of
pipe-feed.

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
text       ← "price_{$weight | janeWeightKey}"
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
