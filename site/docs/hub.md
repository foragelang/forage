# Hub: publish & import

The Forage hub is a registry at `hub.foragelang.com` (UI) and
`api.foragelang.com` (API). It hosts community recipes — `recipe.forage`
source plus optional fixtures and a snapshot — and serves them to:

- the `forage` CLI's `forage publish` command;
- Studio's Publish tab;
- recipes that declare `import <ref>` at the top of their source.

## Authoring a recipe

A recipe directory is the unit of work — one `.forage` file plus optional
sibling assets.

```
my-recipe/
├── recipe.forage         # required
├── recipe.json           # optional: namespace / name / displayName / summary / tags / author
├── fixtures/
│   └── captures.jsonl    # optional: replayable fixtures
└── expected.snapshot.json # optional: golden snapshot
```

`recipe.json` lets you keep publish metadata out of the CLI invocation:

```json
{
    "namespace": "alice",
    "name": "my-recipe",
    "displayName": "My recipe",
    "summary": "What this recipe does",
    "tags": ["dispensary", "sweed"],
    "author": "alice"
}
```

Shape rules:

- **`name`**: `^[a-z0-9][a-z0-9-]{1,63}$`. Lowercase letters, digits, and
  hyphens; 2–64 chars; must start with a letter or digit.
- **`namespace`**: same regex as `name`. Defaults to `forage` (the official
  namespace) if you omit it. Pick your own handle for personal recipes.

The published slug is always `<namespace>/<name>`.

## Publishing

Set an API key in your environment:

```bash
export FORAGE_HUB_TOKEN="your-api-key"
```

Dry-run is the default — it prints the JSON the CLI *would* POST without
hitting the network:

```bash
forage publish path/to/my-recipe
# {
#   "slug": "forage/my-recipe",
#   "displayName": "My recipe",
#   …
# }
# dry-run — pass --publish to POST
```

Pass `--publish` to actually POST. Use it once you're happy with the
payload:

```bash
forage publish path/to/my-recipe --publish
# published forage/my-recipe v1
# sha256: deadbeef…
# curl -fsSL https://api.foragelang.com/v1/packages/forage/my-recipe
```

Override metadata from the command line when there's no `recipe.json` (or
when you want to tweak just one field for one run):

```bash
forage publish path/to/my-recipe \
    --namespace alice \
    --name my-recipe \
    --display-name "My recipe" \
    --summary "What this recipe does" \
    --tags cannabis,sweed \
    --author alice \
    --publish
```

By default the CLI talks to `https://api.foragelang.com`. Override via
`FORAGE_HUB_URL` (useful for staging or `wrangler dev`):

```bash
export FORAGE_HUB_URL="http://127.0.0.1:8787"
```

## Sharing declarations across recipes

The unit of distribution is a **package**: every `.forage` file in a
workspace, plus the `forage.toml` manifest. Sharing types is a
workspace-level concern — recipes never write `import` directives.

To use types published by someone else:

1. Add the dependency to `forage.toml`:
   ```toml
   [deps]
   "alice/awesome-utils" = 3
   ```
2. `forage update` resolves the dep, downloads the package into
   `~/Library/Forage/Cache/hub/alice/awesome-utils/3/`, and writes the
   resolved version + digest to `forage.lock`.
3. Every recipe in the workspace now sees the package's shared types
   in its merged catalog — no per-recipe declaration needed.

A recipe can redeclare a type with the same name to shadow the
shared one locally (per-platform extension pattern).

## API endpoints (reference)

| Method | Path                                              | Auth     | Returns                                                |
|--------|---------------------------------------------------|----------|--------------------------------------------------------|
| GET    | `/v1/health`                                      | —        | `{"status":"ok"}`                                      |
| GET    | `/v1/packages`                                    | —        | `{items, nextCursor}` — paginated listing              |
| GET    | `/v1/packages/:namespace/:name`                   | —        | full package (metadata + every file body). `?version=N`|
| GET    | `/v1/packages/:namespace/:name/versions`          | —        | version history                                        |
| GET    | `/v1/packages/:namespace/:name/fixtures`          | —        | fixtures.jsonl (if uploaded)                           |
| GET    | `/v1/packages/:namespace/:name/snapshot`          | —        | snapshot.json (if uploaded)                            |
| POST   | `/v1/packages`                                    | Bearer   | publish — returns `{slug, version, sha256}`            |

Hand-roll a publish if you don't want to use the CLI:

```bash
curl -fsSL -X POST https://api.foragelang.com/v1/packages \
    -H "Authorization: Bearer $FORAGE_HUB_TOKEN" \
    -H "Content-Type: application/json" \
    -d @payload.json
```

Where `payload.json` matches:

```json
{
    "slug": "alice/my-recipe",
    "displayName": "My recipe",
    "summary": "What this recipe does",
    "tags": ["dispensary"],
    "files": [
        {"name": "recipe.forage", "body": "recipe \"my-recipe\"\nengine http\n…"},
        {"name": "shared.forage", "body": "type Item { id: String }\n"}
    ],
    "fixtures": "{\"…jsonl content…\"}",
    "snapshot": "{\"…snapshot json…\"}"
}
```

The `slug` is `<namespace>/<name>`. `files` is the package — one entry
per `.forage` file in the workspace; at least one must carry a
`recipe "<name>"` header. Names may include a single `/` separator
(e.g. `<dir>/recipe.forage`). `fixtures` and `snapshot` are optional;
both are stored verbatim in R2 under
`recipes/<namespace>/<name>/<version>/`.
