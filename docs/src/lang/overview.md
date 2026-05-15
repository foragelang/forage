# Language overview

A `.forage` file is a sequence of top-level forms. A file with a `recipe
"<name>" engine <kind>` header declares a recipe; one without is a pure
declarations file contributing `share`d types / enums / fns to the
workspace catalog. File location and basename carry no semantics — the
recipe's identity is the string in its header.

```forage
recipe "<name>"
engine <http | browser>
emits Product

// File-scoped helper type.
type LocalPanel { id: String }

// Workspace-visible declarations (visible to every other .forage file).
share type Product { name: String, price: Double }
share enum MenuType { RECREATIONAL, MEDICAL }

input storeId: String
secret apiToken

auth.staticHeader { name: "Authorization", value: "Bearer {$secret.apiToken}" }

step list {
    method "GET"
    url    "https://api.example.com/items?store={$input.storeId}"
    paginate pageWithTotal {
        items: $.results, total: $.total,
        pageParam: "page", pageSize: 50
    }
}
for $item in $list.results[*] {
    emit Product {
        name  ← $item.name
        price ← $item.price | parseFloat
    }
}

expect { records.where(typeName == "Product").count >= 20 }
```

Two engines, one DSL. Pick `engine http` for sites that expose a JSON
API the runtime can drive directly; pick `engine browser` for SPAs that
render in JS or sit behind JS-challenge bot management. Both engines
target the same output type catalog, so downstream code never has to
know which one ran.

Workspaces pull in cross-workspace shared types through hub `[deps]` in
`forage.toml`; see [Imports](./imports.md).

## Pipeline

1. **Parse**: source → AST. Diagnostics carry byte spans the LSP turns
   into squiggles.
2. **Validate**: semantic checks — every type/input/secret/transform
   reference resolves, every emit has required fields bound, engine
   consistency (HTTP recipes don't declare `browser { … }` etc.).
3. **Evaluate**: the engine walks the body, drives HTTP/browser, binds
   step responses to `$<stepName>`, resolves path expressions, applies
   transforms, accumulates emit records into a `Snapshot`.
4. **Verify**: expectations run against the final snapshot; unmet ones
   land in `diagnostic.unmet_expectations`.

The pipeline is deterministic given inputs + fixtures, so recipes
round-trip cleanly through `forage test`.

## Where to go next

- [Types and enums](./types.md) — the catalog every emit refers to.
- [Inputs and secrets](./inputs-secrets.md) — what consumers supply.
- [HTTP engine](./http.md) and [Browser engine](./browser.md).
- [Expressions and templates](./expressions.md) — path syntax,
  pipelines, case-of, interpolation.
- [Transforms](./transforms.md) — the built-ins (`lower`, `dedup`,
  `parseHtml`, `select`, `match`, `replaceAll`, …).
