# Publish

The unit of publication is a **package**: every `.forage` file in a
workspace plus its `forage.toml` manifest. Single-file recipes ship as
one-file packages. Anyone with a hub publish token can `POST
/v1/packages`.

## With the CLI

```sh
# Author a recipe under recipes/<slug>/ (workspace at recipes/)
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
POST https://api.foragelang.com/v1/packages
Authorization: Bearer <HUB_PUBLISH_TOKEN>
Content-Type: application/json
```

Body:

```json
{
    "slug": "alice/my-recipe",
    "author": "alice",
    "displayName": "My recipe",
    "summary": "Short description.",
    "tags": ["dispensary", "json-api"],
    "platform": "sweed",
    "files": [
        {"name": "recipe.forage", "body": "recipe \"my-recipe\"\nengine http\n…"},
        {"name": "shared.forage", "body": "type Item { id: String }\n"}
    ]
}
```

`slug` must match `^[a-z0-9][a-z0-9-]{1,63}\/[a-z0-9][a-z0-9-]{1,63}$`.
`files` is the package — one entry per `.forage` file in the workspace.
At least one must carry a `recipe "<name>"` header. Returns
`201 {slug, version, sha256}`. Each publish bumps the version; old
versions are kept and queryable via
`GET /v1/packages/<slug>?version=N`.

## With Studio

Forage Studio's **Publish** tab carries the same flow: form fields for
metadata, **Validate** runs the runtime parser+validator, **Preview payload**
shows the JSON, **Publish** POSTs to the API.
