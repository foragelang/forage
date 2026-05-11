# Syntax reference

Every construct in the `.forage` DSL. Read top-to-bottom for a tour; jump to a section if you know what you're looking for.

## Recipe header

A file is a single `recipe` block. The name is a string literal; it appears in error messages and snapshot metadata.

```forage
recipe "my-platform" {
    engine http   // or: browser
    // ... body ...
}
```

Comments are `//` to end-of-line or `/* … */` block.

## Types

Declare the shape of records the recipe will emit. Fields are typed; `?` marks a field optional; `[T]` is a list. Nested record types are allowed.

```forage
type Product {
    externalId: String
    name:       String
    brand:      String?
    price:      Double?
    tags:       [String]
}
```

Built-in scalars: `String`, `Int`, `Double`, `Bool`.

## Enums

A closed set of named variants. Used as field types and in iteration.

```forage
enum MenuType { RECREATIONAL, MEDICAL }
```

## Inputs

Per-run parameters supplied by the consumer. The same recipe can serve every store on a platform; per-store config (store id, menu URL, category list) comes in as inputs.

```forage
input storeId: String
input menuTypes: [MenuType]
input categoryIds: [Int]
```

Reference an input anywhere a value is expected as `$input.fieldName`.

## Auth

Auth strategies are named, fixed primitives. Pick one (or none); the engine knows how to apply it.

### auth.staticHeader

A single header sent on every request.

```forage
auth.staticHeader {
    name:  "X-Store-Id"
    value: $input.storeId
}
```

### auth.htmlPrime

For sites that gate their AJAX endpoints behind a per-session nonce and a cookie set on first page load. A named `step` performs the prime; the engine extracts the nonce by regex on the response body and carries the cookie forward.

```forage
auth.htmlPrime {
    step:       prime
    nonceVar:   "ajaxNonce"
    ajaxUrlVar: "ajaxUrl"
}
```

## Steps

A `step` names an HTTP request whose response becomes addressable as `$<stepName>`. Steps appear at the top level of a recipe and can be nested inside `for` loops.

```forage
step products {
    method "POST"
    url    "https://api.example.com/products"
    body.json {
        page:     1
        pageSize: 50
        filters:  { category: [$catId] }
    }
}
```

Step keys:

| Key            | Form           | Notes                                                                |
| -------------- | -------------- | -------------------------------------------------------------------- |
| `method`       | String literal | `"GET"`, `"POST"`, …                                                 |
| `url`          | String literal | Templated: `{$input.x}` and `{$var.path}` interpolations.            |
| `headers`      | Object         | Per-request headers. Static-header auth is layered on top.           |
| `body.json`    | Object         | JSON body. Values can reference inputs, loop vars, prior step outputs.|
| `body.form`    | Object         | Form-encoded body.                                                   |
| `paginate`     | Strategy block | See [Pagination](/docs/engines#pagination).                          |

## Iteration

Two iteration sources: a list value (e.g. an input or a path into a response) or an enum's variants.

```forage
for $menu in $input.menuTypes {
    // $menu is a MenuType value, available in nested steps and emits
}

for $product in $products[*] {
    // $product is one element of the $products response list
}
```

Loops can nest. Inner scopes see all variables from enclosing scopes.

## Emit

An `emit` binds the fields of a declared type to extraction expressions. Each emit produces one record in the output snapshot.

```forage
emit Product {
    externalId ← $product.id | toString
    name       ← $product.name
    brand      ← $product.brand?.name
    price      ← $product.price
}
```

The validator checks every required (non-optional) field is bound and every bound field type matches.

## Path expressions

The right-hand side of an emit field is a path expression with optional pipes through transforms.

| Form         | Meaning                                                                       |
| ------------ | ----------------------------------------------------------------------------- |
| `$step`      | The full response value from a named step.                                    |
| `$input.x`   | A recipe input.                                                               |
| `$loopVar`   | The current iteration value.                                                  |
| `.field`     | Object field access.                                                          |
| `?.field`    | Optional chaining: short-circuits to null if any intermediate is null.        |
| `[*]`        | Iterate over a list (in for-loops) or map over a list (in expressions).       |
| `[N]`        | Index a list by integer.                                                      |

## String templates

A string literal in a URL, header, or body becomes a *template* — every `{...}` interpolation is a full extraction expression, evaluated against the current scope and stringified into the surrounding text.

```forage
url "https://api.example.com/stores/{$input.storeId}/products?page={$i}"

headers {
    "X-Trace": "page-{$i | toString}"
}

body.json {
    key: "price_{$weight | janeWeightKey}"   // dynamic key built from a transform
}
```

Inside `{...}`, you can use the same forms an extraction supports:

- bare paths — `{$input.x}`, `{$step.list[0].id}`
- pipe transforms — `{$weight | janeWeightKey}`, `{$count | toString}`
- function-call transforms — `{coalesce($a, $b, "fallback")}`
- `case … of { … }` branches

Transforms inside template interpolations are checked by the validator at load time, so a typo'd `{$x | snak_case}` fails before any HTTP request fires — not at runtime, three pages into a paginated scrape.

## Transforms

Transforms are named, engine-implemented functions chained with `|`. The vocabulary is fixed — new transforms are added in Swift as real platforms surface them, not invented per-recipe.

| Transform                                           | Effect                                                                |
| --------------------------------------------------- | --------------------------------------------------------------------- |
| `toString`                                          | Convert a number or bool to a string.                                 |
| `parseInt` / `parseFloat` / `parseBool`             | Parse a string to the named scalar; returns null if it doesn't parse. |
| `coalesce(a, b, …)`                                 | The piped value if non-null, otherwise the first non-null argument.   |
| `default(value)`                                    | Substitute `value` when the piped value is null.                      |
| `lower` / `upper` / `capitalize` / `titleCase` / `trim` | String case and whitespace.                                      |
| `length`                                            | Length of a list or string (0 for null).                              |
| `dedup`                                             | Remove duplicates from a list, preserving order.                      |
| `getField(name)`                                    | Look up a field on an object whose name is computed at runtime.       |

Forage also ships a small set of cannabis-domain transforms used by the bundled platform recipes — `parseSize`, `normalizeOzToGrams`, `normalizeUnitToGrams`, `prevalenceNormalize`, `parseJaneWeight`, `janeWeightUnit`, `janeWeightKey`. The last is a generic snake-caser (`"Half Ounce" → "half_ounce"`), useful for building dynamic-key getField lookups. These will move out of the engine and into consumer-supplied registrations once the type catalog is lifted.

## case expressions

Branch on an enum value. Useful when the same emit binds differently per dimension.

```forage
price ← case $menu of {
            RECREATIONAL → $variant.priceRec
            MEDICAL      → $variant.priceMed
        }
```

The validator requires every variant of the enum to be covered.

## Expectations

An `expect` block declares an invariant about the snapshot the recipe is supposed to produce. The engine evaluates each clause at the end of a run and adds any failures to `report.unmetExpectations` — a structured diagnostic instead of leaving the consumer to wonder why the output looks thin.

```forage
expect { records.where(typeName == "Product").count >= 50 }
expect { records.where(typeName == "Variant").count > 0 }
```

See the [expectations page](/docs/expectations) for the full grammar and failure rendering.

::: tip The validator is your first reader
Most mistakes — unknown types, unbound paths, missing required fields, unknown transforms (including ones inside `{...}` template interpolations), non-exhaustive case branches — are caught statically before any HTTP request fires. Errors point at the line and column with terms from the DSL.
:::
