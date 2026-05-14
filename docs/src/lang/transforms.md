# Transforms

Transforms are the building blocks of every expression pipeline. Each
takes a current value plus zero or more arguments and returns a new
value. They live in `forage-core::eval::transforms` and ship with both
the CLI and the wasm core, so the same names work in every host.

## String

| Transform | Behavior |
|---|---|
| `toString` | coerce anything to its string form |
| `lower` | lowercase |
| `upper` | uppercase |
| `trim` | strip leading + trailing whitespace |
| `capitalize` | uppercase the first character |
| `titleCase` | uppercase the first character of each whitespace-separated word |

## Parsing scalars

| Transform | Behavior |
|---|---|
| `parseInt` | parse as i64; passes through ints, truncates doubles |
| `parseFloat` | parse as f64; passes through numbers |
| `parseBool` | "true"/"yes"/"1" → true, "false"/"no"/"0" → false |
| `parseJson(s)` | parse a JSON string into a value |

## Lists / objects

| Transform | Behavior |
|---|---|
| `length` | string char count, array length, object key count, node-list size |
| `dedup` | drop duplicate items (order preserved) |
| `first` | first item of an array or node-list; null on empty |
| `coalesce(a, b, …)` | the first non-null value among the args (input itself counts) |
| `default(v)` | replace null with `v` |
| `getField(name)` | dynamic field access — `getField($obj, "x")` |

## Size / weight normalization (cannabis-domain helpers)

These are domain-specific but pulled out as transforms because every
dispensary site uses them.

| Transform | Behavior |
|---|---|
| `parseSize` | `"2.5g"` → `{value: 2.5, unit: "g"}` |
| `normalizeOzToGrams` | convert oz to grams; pass through if already grams |
| `sizeValue` | unwrap `value` from a `parseSize` output |
| `sizeUnit` | unwrap `unit` from a `parseSize` output |
| `normalizeUnitToGrams` | normalize the unit label (`oz` → `g`) |
| `prevalenceNormalize` | normalize "indica-dominant"/"sativa" → INDICA/SATIVA/HYBRID/CBD |
| `parseJaneWeight` | Jane's textual weight (`"eighth ounce"`) → numeric grams |
| `janeWeightUnit` | Jane's weight label → unit (`"g"` or `"EA"`) |
| `janeWeightKey` | Jane's weight label → snake_case key for `getField` |

## HTML

Used by browser recipes and HTTP recipes that fetch HTML pages directly
(SCOTUS, HN-HTML).

| Transform | Behavior |
|---|---|
| `parseHtml` | wrap a raw HTML string as a parseable node |
| `select(sel)` | CSS-select children of a node, returns a node-list |
| `text` | flatten text content of a node or node-list (trimmed) |
| `attr(name)` | get an attribute value off the first matched element |
| `html` | serialize a node as outer HTML (string) |
| `innerHtml` | serialize a node's children as HTML (string) |

Selectors are standard CSS3 — anything `scraper`/`cssselect` recognizes.
Selection on a node-list flat-maps: `nodes | select(".inner")` returns
all `.inner` descendants of any item in the list.

## User-defined transforms

Recipes can declare their own transforms via the top-level `fn` form.
A user-defined transform is a name, a parameter list, and a single
expression body. Call sites are identical to built-ins — either piped
(`$x |> myFn`) or directly invoked (`myFn($x, $y)`).

```forage
fn shouty($x) { $x | upper | trim }

for $i in $list.items[*] {
    emit Item { id ← $i.name | shouty }
}
```

See [User-defined functions](./functions.md) for the full reference,
including scope rules, namespace resolution, and recursion behavior.

## Adding a built-in transform

See [Contributing → Adding a transform](../contributing/transforms.md).
Each built-in is a `fn(EvalValue, &[EvalValue]) -> Result<EvalValue>`
registered by name; new built-ins ship as a small PR.
