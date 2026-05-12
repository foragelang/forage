# Concepts

`hub.foragelang.com` is the public registry for Forage recipes.
`api.foragelang.com` is the API the CLI / Studio / web IDE talk to.

## Recipe identity

A recipe on the hub has:

- A **slug** in `<namespace>/<name>` form, e.g. `alice/zen-leaf` or
  `forage/hacker-news`. Both segments match `^[a-z0-9][a-z0-9-]{1,63}$`.
- A **version** — a monotonic integer the hub assigns on each publish.
  Publishing the same slug bumps the version; old versions stay
  queryable via `?version=N`.
- An **ownerLogin** — the GitHub login of the publisher. Set lazily on
  the first OAuth-authenticated publish. Legacy recipes published via
  the shared admin token carry `ownerLogin: "admin"` and are
  admin-only writable.

## Storage

Each recipe ships with:

- **`recipe.forage`** — the source text. Cached in R2 under
  `blobs/<slug>/<version>/recipe.forage`.
- **`captures.jsonl`** (optional) — recorded fixtures.
- **`expected.snapshot.json`** (optional) — the golden snapshot the
  recipe produces against the bundled fixtures.
- **Metadata** — display name, summary, tags, license, SHA-256 of the
  body. Stored in KV under `recipe:<slug>` for fast list/index.

`hub.foragelang.com` browses these; the IDE renders source with Forage
syntax highlighting; the CLI / Studio fetch them through `forage-hub`.

## Imports

Recipes reference each other via `import <slug>` directives at the top
of the file:

```forage
import forage/cannabis     // shared schema
import alice/zen-leaf v2   // a specific dispensary recipe
```

The resolver pulls them from the hub, caches them locally, and unions
their types/enums/inputs into the importing recipe's catalog. See
[Imports](../lang/imports.md).

## Trust model

- **Recipes are pure data.** They never run code in the host's
  process. The engine reads the recipe and drives HTTP / browser; the
  recipe can't shell out, hit the filesystem, or escape the engine's
  sandbox.
- **Owner-checked writes.** Only the recipe's owner (or an admin) can
  publish a new version or delete a recipe. The Worker enforces this
  via `callerCanWrite` on every mutating endpoint.
- **Honest UA on the runtime.** The runtime hits target sites as
  `Forage/x.y.z (+https://foragelang.com)`; no IP rotation, no
  googlebot impersonation.

## Versions

`/v1/recipes/<slug>/versions` lists every published version with
timestamps. `?version=N` on a `GET` pins to a specific one. The hub
doesn't garbage-collect old versions — recipes that import an older
version of another recipe keep working.

## Cost

The hub runs entirely on Cloudflare Workers + KV + R2. Hosting is
~free for read-heavy traffic; publishes are throttled by the rate
limiter (30/min per user). The whole thing should fit inside
Cloudflare's free tier for the foreseeable future.
