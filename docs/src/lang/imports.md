# Imports

Recipes can pull in shared types, enums, and inputs from the hub.

```forage
import alice/zen-leaf v2
import demo

recipe "uses-imports"
engine http
// Types + enums declared by zen-leaf v2 + demo are now in scope.
// Their inputs union into this recipe's input set.
```

## Syntax

```text
import <name>            // unversioned — latest
import <author>/<name>   // namespaced
import <author>/<name> v<N>   // pinned to a specific version
import <author>/<name>@v<N>   // equivalent
```

A bare `<name>` resolves under the `forage` namespace by default.

## Resolution

`forage-hub::importer` resolves imports recursively before the recipe
runs. The resolver:

1. Looks up `hub://<author>/<name>?version=<N>` against
   `api.foragelang.com`.
2. Caches the fetched recipe text at
   `~/Library/Forage/Cache/hub/<author>/<name>/<version>/recipe.forage`.
3. Unions the imported recipe's types, enums, inputs, and secrets into
   the importing recipe's catalog. Conflicts (same type name with
   different shape) surface as validation errors.
4. Recurses — imports can import imports. Cycles fail with a clear error.

Cached entries are reused across runs and across CLI/Studio/Web IDE
hosts, so a recipe imports cheaply after the first run.

## Why imports

Shared schemas are the main use case. A `forage/cannabis` import
declares `Product`, `Variant`, `PriceObservation`, `MenuType`,
`StrainPrevalence` once; every dispensary recipe imports it and emits
the same shapes downstream code can union into a single table.

The hub doesn't (yet) ship a curated namespace — imports today are
publish-your-own. The roadmap has `forage/cannabis`,
`forage/news`, and a few other reference packs as future work.
