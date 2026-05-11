# Hub: publish & import

The Forage hub is a registry at `hub.foragelang.com` (UI) and
`api.foragelang.com` (API). It hosts community recipes — `recipe.forage`
source plus optional fixtures and a snapshot — and serves them to:

- the `forage` CLI's `forage publish` command;
- the Toolkit app's Publish tab;
- recipes that declare `import hub://<slug>` at the top of their source.

## Authoring a recipe

A recipe directory is the unit of work — one `.forage` file plus optional
sibling assets.

```
my-recipe/
├── recipe.forage         # required
├── recipe.json           # optional: slug / displayName / summary / tags / author
├── fixtures/
│   └── captures.jsonl    # optional: replayable fixtures
└── expected.snapshot.json # optional: golden snapshot
```

`recipe.json` lets you keep publish metadata out of the CLI invocation:

```json
{
    "slug": "my-recipe",
    "displayName": "My recipe",
    "summary": "What this recipe does",
    "tags": ["dispensary", "sweed"],
    "author": "alice"
}
```

Slug shape: `^[a-z0-9][a-z0-9-]{0,63}$` per segment. Slugs may be
`<name>` or `<author>/<name>`.

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
#   "slug": "my-recipe",
#   "displayName": "My recipe",
#   …
# }
# dry-run — pass --publish to POST
```

Pass `--publish` to actually POST. Use it once you're happy with the
payload:

```bash
forage publish path/to/my-recipe --publish
# published my-recipe v1
# sha256: deadbeef…
# curl -fsSL https://api.foragelang.com/v1/recipes/my-recipe
```

Override metadata from the command line when there's no `recipe.json` (or
when you want to tweak just one field for one run):

```bash
forage publish path/to/my-recipe \
    --slug my-recipe \
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

## Importing from a recipe

Recipes can pull declarations from another published recipe by adding an
`import hub://…` directive at the very top of the file (before
`recipe "…"`):

```forage
import hub://shared-types
import hub://alice/awesome-utils v3

recipe "my-recipe" {
    engine http
    // …Item, Product, ProductPrice all live in shared-types now
}
```

Grammar:

```
ImportStatement := "import" "hub://" Slug [ "v" Integer ]
Slug            := AuthorOrName ("/" Name)?
```

- The slug after `hub://` is either `name` or `author/name`.
- `v<N>` pins a specific version. Without it, the latest published
  version is fetched at run time.

### What gets imported?

In v1 imports are **declaration unions**, not text-concatenation:

- `type`s and `enum`s from the imported recipe become available in the
  importing recipe (referenced by their bare name).
- `input` declarations are unioned — useful for sharing input shapes.
- Body statements (`step`, `for`, `emit`) are **not** imported. Imports
  contribute names, not behavior.

Conflicts (same type / enum / input name across imports) raise a
validation error at run time. A locally-declared type with the same name
as an imported one **wins** — that's how you override an import.

### Caching

Imports are cached on disk at
`~/Library/Forage/Cache/hub/<slug>/<version>/recipe.forage`. Pinned
versions are read straight from cache once seen; `import hub://x` without
a version always hits the network so you get the latest.

## API endpoints (reference)

| Method | Path                                | Auth     | Returns                                                |
|--------|-------------------------------------|----------|--------------------------------------------------------|
| GET    | `/v1/health`                        | —        | `{"status":"ok"}`                                      |
| GET    | `/v1/recipes`                       | —        | `{items, nextCursor}` — paginated listing              |
| GET    | `/v1/recipes/:slug`                 | —        | full recipe (metadata + body). `?version=N` for history|
| GET    | `/v1/recipes/:slug/versions`        | —        | version history                                        |
| GET    | `/v1/recipes/:slug/fixtures`        | —        | fixtures.jsonl (if uploaded)                           |
| GET    | `/v1/recipes/:slug/snapshot`        | —        | snapshot.json (if uploaded)                            |
| POST   | `/v1/recipes`                       | Bearer   | publish — returns `{slug, version, sha256}`            |

Hand-roll a publish if you don't want to use the CLI:

```bash
curl -fsSL -X POST https://api.foragelang.com/v1/recipes \
    -H "Authorization: Bearer $FORAGE_HUB_TOKEN" \
    -H "Content-Type: application/json" \
    -d @payload.json
```

Where `payload.json` matches:

```json
{
    "slug": "my-recipe",
    "displayName": "My recipe",
    "summary": "What this recipe does",
    "tags": ["dispensary"],
    "body": "recipe \"my-recipe\" { engine http … }",
    "fixtures": "{\"…jsonl content…\"}",
    "snapshot": "{\"…snapshot json…\"}"
}
```

`fixtures` and `snapshot` are optional; both are stored verbatim in R2
under `recipes/<slug>/<version>/`.
