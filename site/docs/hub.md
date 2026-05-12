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
# curl -fsSL https://api.foragelang.com/v1/recipes/forage/my-recipe
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

## Importing from a recipe

Recipes can pull declarations from another published recipe by adding an
`import <ref>` directive at the very top of the file (before `recipe "…"`):

```forage
import shared-types
import alice/awesome-utils v3

recipe "my-recipe" {
    engine http
    // …Item, Product, ProductPrice all live in shared-types now
}
```

Grammar:

```
ImportStatement := "import" Ref [ "v" Integer ]
Ref             := [ Registry "/" ] [ Namespace "/" ] Name
```

### How references resolve

The reference grammar mirrors Docker image references. Given `import a/b/c`,
the first slash-separated component is treated as a **registry hostname**
if it contains `.`, contains `:`, or equals `localhost`. Otherwise it's a
namespace.

| Reference                            | Registry           | Namespace | Name      | Resolves to                                            |
|--------------------------------------|--------------------|-----------|-----------|--------------------------------------------------------|
| `sweed`                              | (default hub)      | `forage`  | `sweed`   | `https://api.foragelang.com/v1/recipes/forage/sweed`   |
| `alice/zen-leaf`                     | (default hub)      | `alice`   | `zen-leaf`| `https://api.foragelang.com/v1/recipes/alice/zen-leaf` |
| `hub.example.com/team/scraper`       | `hub.example.com`  | `team`    | `scraper` | `https://hub.example.com/v1/recipes/team/scraper`      |
| `localhost:5000/me/test`             | `localhost:5000`   | `me`      | `test`    | `http://localhost:5000/v1/recipes/me/test`             |

`v<N>` pins a specific version. Without it, the latest published version is
fetched at run time.

`localhost`-prefixed registries use `http://`; everything else uses
`https://`.

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
`~/Library/Forage/Cache/hub/<registry>/<namespace>/<name>/<version>/recipe.forage`,
with `_default` standing in for the default-hub registry. Pinned versions
are read straight from cache once seen; an `import` without `v<N>` always
hits the network so you get the latest.

## API endpoints (reference)

| Method | Path                                              | Auth     | Returns                                                |
|--------|---------------------------------------------------|----------|--------------------------------------------------------|
| GET    | `/v1/health`                                      | —        | `{"status":"ok"}`                                      |
| GET    | `/v1/recipes`                                     | —        | `{items, nextCursor}` — paginated listing              |
| GET    | `/v1/recipes/:namespace/:name`                    | —        | full recipe (metadata + body). `?version=N` for history|
| GET    | `/v1/recipes/:namespace/:name/versions`           | —        | version history                                        |
| GET    | `/v1/recipes/:namespace/:name/fixtures`           | —        | fixtures.jsonl (if uploaded)                           |
| GET    | `/v1/recipes/:namespace/:name/snapshot`           | —        | snapshot.json (if uploaded)                            |
| POST   | `/v1/recipes`                                     | Bearer   | publish — returns `{slug, version, sha256}`            |

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
    "slug": "alice/my-recipe",
    "displayName": "My recipe",
    "summary": "What this recipe does",
    "tags": ["dispensary"],
    "body": "recipe \"my-recipe\" { engine http … }",
    "fixtures": "{\"…jsonl content…\"}",
    "snapshot": "{\"…snapshot json…\"}"
}
```

The `slug` is `<namespace>/<name>`. `fixtures` and `snapshot` are optional;
both are stored verbatim in R2 under `recipes/<namespace>/<name>/<version>/`.
