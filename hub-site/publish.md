# Publish

Recipes are flat text. Anyone with a hub publish token can `POST /v1/recipes`.

## With the CLI

```sh
# Author a recipe under recipes/<slug>/
forage scaffold tests/fixtures/captures.jsonl --out recipes/<slug>/recipe.forage

# Run against fixtures to make sure it produces the snapshot you expect.
forage test recipes/<slug>

# Dry-run to see the JSON payload.
forage publish recipes/<slug> --dry-run

# Live publish (requires FORAGE_HUB_TOKEN in env).
FORAGE_HUB_TOKEN=… forage publish recipes/<slug>
```

The CLI runs the parser + validator locally before posting; failures are
caught client-side with full diagnostics.

## With curl

The endpoint:

```
POST https://api.foragelang.com/v1/recipes
Authorization: Bearer <HUB_PUBLISH_TOKEN>
Content-Type: application/json
```

Body:

```json
{
    "slug": "my-recipe",
    "author": "you",
    "displayName": "My recipe",
    "summary": "Short description.",
    "tags": ["dispensary", "json-api"],
    "platform": "sweed",
    "body": "recipe \"my-recipe\" { engine http\n  ... }\n"
}
```

`slug` must match `^[a-z0-9][a-z0-9-]{1,63}$`. Returns `201 {slug, version, sha256}`.
Each publish bumps the version; old versions are kept and queryable via
`GET /v1/recipes/<slug>?version=N`.

## With Studio

Forage Studio's **Publish** tab carries the same flow: form fields for
metadata, **Validate** runs the runtime parser+validator, **Preview payload**
shows the JSON, **Publish** POSTs to the API.
