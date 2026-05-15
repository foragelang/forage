# Sharing and dependencies

Forage has two scopes of cross-file visibility:

1. **Within a workspace** — `share` on a `type` / `enum` / `fn`
   declaration publishes it to the workspace-wide catalog visible to
   every other `.forage` file.
2. **Across workspaces** — `[deps]` in `forage.toml` pulls a published
   package from the hub; its `share`d declarations join the consuming
   workspace's catalog.

## `share` within a workspace

Default visibility is **file-scoped**. Prefix a `type` / `enum` / `fn`
with `share` to make it workspace-visible:

```forage
// cannabis.forage  — a pure declarations file (no recipe header).
share type Product   { name: String; sku: String; price: Money }
share enum  MenuType { RECREATIONAL, MEDICAL }
share fn    prevalenceNormalize($x) { … }
```

```forage
// remedy-baltimore.forage
recipe "remedy-baltimore"
engine http

// `Product` and `MenuType` resolve from cannabis.forage above.
emit Product { … }
```

Workspace-wide name collisions among `share`d declarations are a
validator error. A file-scoped declaration overrides a same-named
`share`d declaration when both reach the same recipe's catalog.

`input` and `secret` are recipe-local — they don't take `share`.

## `[deps]` across workspaces

A workspace's `forage.toml` carries a `[deps]` table mapping
`<author>/<slug>` to a version constraint:

```toml
# forage.toml
name = "alice/anything"

[deps]
"alice/cannabis" = "*"
"alice/zen-leaf" = "v2"
```

`forage update` resolves each entry against `api.foragelang.com`,
fetches the package into the local cache, and writes `forage.lock`. The
resolver:

1. Looks up `hub://<author>/<slug>?version=<N>` against
   `api.foragelang.com`.
2. Caches the fetched package files under
   `~/Library/Forage/Cache/hub/<author>/<slug>/<version>/`.
3. Unions every `share`d declaration the package ships into the
   consuming workspace's catalog. Conflicts (same type name with
   different shape) surface as validation errors.
4. Recurses — deps can have deps. Cycles fail with a clear error.

Cached entries are reused across runs and across CLI / Studio / Web IDE
hosts, so a dep resolves cheaply after the first `update`.

## Why this matters

Shared schemas are the main use case. A workspace dep on
`forage/cannabis` declares `Product`, `Variant`, `PriceObservation`,
`MenuType`, `StrainPrevalence` once; every dispensary recipe in the
workspace consumes the same shapes downstream code can union into a
single table.

The hub doesn't (yet) ship a curated namespace — deps today are
publish-your-own. The roadmap has `forage/cannabis`,
`forage/news`, and a few other reference packs as future work.
