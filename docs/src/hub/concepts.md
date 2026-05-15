# Concepts

`hub.foragelang.com` is the public registry for Forage packages.
`api.foragelang.com` is the API the CLI / Studio / hub IDE talk to.

## Package identity

A package on the hub has:

- An **author** — your GitHub login. Matches `^[a-z0-9][a-z0-9-]{0,38}$`.
- A **slug** — the package name in your namespace, e.g. `zen-leaf`.
  Same shape as the author.
- A linear list of **versions** (1, 2, 3, …). Publishing a new version
  requires a matching `base_version` against the current
  `latest_version`; mismatches return `409 stale_base`.
- An **owner_login** — set on first publish (the caller's GitHub
  login). All future writes require the same login.
- A one-shot **forked_from** pointer on the v1 metadata of a fork.
  Points at the upstream `(author, slug, version)` the fork was cut
  from. Never updated thereafter; pulls from upstream are explicit
  re-publishes.

## Atomic version artifact

The unit of fetch is a **package version** — one indivisible JSON
artifact carrying:

- `recipe` — the main `.forage` source.
- `decls` — additional `.forage` files in the package (shared types,
  enums, helpers).
- `fixtures` — captured replay data (typically JSONL).
- `snapshot` — the runtime's output against the fixtures, in
  `{records, counts}` shape.
- `base_version`, `published_at`, `published_by`.

The hub stores each artifact under `ver:<author>:<slug>:<n>` in KV.
Artifacts that serialize past 20 MiB are written to R2 instead and the
KV slot holds a `{r2_key}` pointer; the wire shape is identical
either way.

## Social surfaces

- **Stars** — `POST /v1/packages/<author>/<slug>/stars` toggles a
  star and bumps the counter on the package metadata. Each star also
  goes into a reverse `stars_by:<user>` index so a profile can list
  what its owner has starred.
- **Downloads** — `POST /v1/packages/<author>/<slug>/downloads`
  increments the counter. Called by Studio's `sync_from_hub` and by
  the fork endpoint.
- **Forks** — `POST /v1/packages/<upstream>/<slug>/fork` creates
  `@me/<slug>` with v1 carrying the upstream's full content +
  `forked_from`. Bumps the upstream's `fork_count` + `downloads`.
- **Categories** — `GET /v1/categories` lists every category that has
  at least one package; `?category=` filters listings.
- **Profiles** — `GET /v1/users/<author>` returns the public profile;
  `…/packages` and `…/stars` list what they've shipped and what
  they've starred.

## Sharing and dependencies

Workspaces consume hub packages via `[deps]` in `forage.toml`:

```toml
# forage.toml
[deps]
"alice/cannabis"  = "*"
"alice/zen-leaf"  = "v2"
```

`forage update` resolves each entry, caches the fetched packages
locally, and unions every `share`d declaration they ship into the
consuming workspace's catalog. See
[Sharing and dependencies](../lang/imports.md).

## Trust model

- **Recipes are pure data.** They never run code in the host's
  process. The engine reads the recipe and drives HTTP / browser; the
  recipe can't shell out, hit the filesystem, or escape the engine's
  sandbox.
- **Owner-checked writes.** Only the package's `owner_login` (or an
  admin) can publish a new version. The Worker enforces this on every
  mutating endpoint.
- **Honest UA on the runtime.** The runtime hits target sites as
  `Forage/x.y.z (+https://foragelang.com)`; no IP rotation, no
  googlebot impersonation.

## Cost

The hub runs entirely on Cloudflare Workers + KV + R2. Hosting is
~free for read-heavy traffic; publishes are throttled by the rate
limiter (30/min per user).
