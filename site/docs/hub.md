# Hub: publish & import

The Forage hub is a registry at `hub.foragelang.com` (UI) and
`api.foragelang.com` (API). It hosts community packages — recipe
sources plus their shared declarations, replay fixtures, and the
snapshot the recipe produced against them — and serves them to:

- the `forage` CLI's `publish` / `sync` / `fork` commands;
- Studio's Publish tab and "Clone from hub" workspace sidebar;
- recipes that declare `import <author>/<slug>` at the top.

## Authoring a recipe

A recipe directory is the unit of work — one `.forage` file plus
optional sibling assets.

```
my-recipe/
├── recipe.forage         # required
├── shared.forage         # optional: shared types / helpers
├── fixtures/
│   └── captures.jsonl    # optional: replayable fixtures
└── expected.snapshot.json # optional: golden snapshot
```

`forage.toml` at the workspace root declares the package's
description, category, and tags.

## Publishing

Sign in once via GitHub:

```bash
forage auth login
```

Dry-run is the default — it prints the JSON envelope the CLI *would*
POST without hitting the network:

```bash
forage publish path/to/my-recipe
```

Pass `--publish` to actually POST. The hub stamps the next available
version:

```bash
forage publish path/to/my-recipe --publish
# published alice/my-recipe v1
# curl -fsSL https://api.foragelang.com/v1/packages/alice/my-recipe
```

By default the CLI talks to `https://api.foragelang.com`. Override via
`FORAGE_HUB_URL` (useful for staging or `wrangler dev`).

## Stale-base detection

The CLI tags every publish with the `base_version` you rebased from.
If a teammate landed a later version while you were drafting, the hub
returns `409 stale_base` with the current `latest_version`; the CLI
re-pulls, replays your delta, and retries.

## Sharing declarations across recipes

Recipes reference each other via `import <author>/<slug>` directives
at the top:

```forage
import alice/cannabis           // shared schema
import alice/zen-leaf v2        // a specific dispensary recipe
```

The resolver pulls them from the hub, caches them locally, and unions
their types / enums / inputs into the importing recipe's catalog.

## Forks

Fork any public package into your own namespace:

```bash
forage fork alice/zen-leaf            # → @me/zen-leaf
forage fork alice/zen-leaf my-leaf    # → @me/my-leaf
```

The fork carries `forked_from: {author: "alice", slug: "zen-leaf",
version: N}` on its v1 metadata, where N is the upstream version you
forked. After that the fork is independent — there is no auto-tracking.
Pulls from upstream are explicit re-publishes through the regular
publish path.

## API endpoints (reference)

| Method | Path                                              | Auth     | Returns                                          |
|--------|---------------------------------------------------|----------|--------------------------------------------------|
| GET    | `/v1/health`                                      | —        | `{"status":"ok"}`                                |
| GET    | `/v1/packages`                                    | —        | `{items, next_cursor}` — paginated listing       |
| GET    | `/v1/packages?sort=&category=&q=&cursor=&limit=`  | —        | filtered + sorted listing                        |
| GET    | `/v1/packages/:author/:slug`                      | —        | package metadata                                 |
| GET    | `/v1/packages/:author/:slug/versions`             | —        | version history                                  |
| GET    | `/v1/packages/:author/:slug/versions/:n`          | —        | atomic version artifact (`n` or `latest`)        |
| POST   | `/v1/packages/:author/:slug/versions`             | Bearer   | publish next version (`base_version` required)   |
| POST   | `/v1/packages/:author/:slug/stars`                | Bearer   | star the package                                 |
| DELETE | `/v1/packages/:author/:slug/stars`                | Bearer   | unstar                                           |
| GET    | `/v1/packages/:author/:slug/stars`                | —        | who starred it                                   |
| POST   | `/v1/packages/:author/:slug/downloads`            | —        | bump the download counter                        |
| POST   | `/v1/packages/:author/:slug/fork`                 | Bearer   | fork into the caller's namespace                 |
| GET    | `/v1/users/:author`                               | —        | public profile                                   |
| GET    | `/v1/users/:author/packages`                      | —        | packages owned by the user                       |
| GET    | `/v1/users/:author/stars`                         | —        | packages the user has starred                    |
| GET    | `/v1/categories`                                  | —        | list of categories with at least one package     |

Hand-roll a publish:

```bash
curl -fsSL -X POST https://api.foragelang.com/v1/packages/alice/my-recipe/versions \
    -H "Authorization: Bearer $FORAGE_HUB_TOKEN" \
    -H "Content-Type: application/json" \
    -d @payload.json
```

Where `payload.json` matches:

```json
{
    "description": "What this recipe does",
    "category": "dispensary",
    "tags": ["sweed", "cannabis"],
    "recipe": "recipe \"my-recipe\"\nengine http\n…",
    "decls": [
        {"name": "shared.forage", "source": "type Item { id: String }\n"}
    ],
    "fixtures": [
        {"name": "captures.jsonl", "content": "…jsonl content…"}
    ],
    "snapshot": {"records": {/* … */}, "counts": {/* … */}},
    "base_version": null,
    "forked_from": null
}
```

`base_version` is `null` on the first publish, the current
`latest_version` on subsequent ones. `forked_from` is `null` on
regular publishes (the fork endpoint sets it on the v1 of a fork).
