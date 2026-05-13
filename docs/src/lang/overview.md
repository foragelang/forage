# Language overview

A `.forage` file declares one recipe. The header (`recipe "<name>"` +
`engine <kind>`) lives flat at the top of the file; every subsequent
declaration belongs to that recipe. There is no surrounding `{ }` block —
the file IS the recipe.

```forage
recipe "<name>"
engine <http | browser>

// Optional hub imports.
// import alice/zen-leaf v2

type Product { name: String, price: Double }
enum MenuType { RECREATIONAL, MEDICAL }

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
- [Transforms](./transforms.md) — the 30+ built-ins (`lower`, `dedup`,
  `parseHtml`, `select`, `prevalenceNormalize`, …).
