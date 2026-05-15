# Publish

The unit of publication is a **package version** — one atomic
artifact carrying the recipe source, any shared `.forage` declarations,
captured replay fixtures, and the snapshot that the recipe produced
against those fixtures. The version artifact is what other users
fetch, replay, and fork.

Authentication uses your GitHub identity. Sign in once
(`forage auth login` or the Studio sign-in button) and the CLI / Studio
mint signed publish requests on your behalf.

## With the CLI

Recipes live in your workspace — `~/Library/Forage/Recipes/` on macOS,
`$XDG_DATA_HOME/forage/recipes/` on Linux, `%APPDATA%\Forage\Recipes\`
on Windows. One `<recipe>.forage` file per recipe, at the workspace
root; captured fixtures sit at `_fixtures/<recipe>.jsonl` and the
recorded snapshot at `_snapshots/<recipe>.json`.

```sh
cd ~/Library/Forage/Recipes

# Scaffold a recipe from captured fixtures.
forage scaffold _fixtures/<recipe>.jsonl --name <recipe>

# Run against fixtures to confirm the snapshot.
forage test <recipe>

# Dry-run to see the publish envelope.
forage publish <recipe>

# Live publish — picks up the next version automatically.
forage publish <recipe> --publish
```

The CLI runs the parser + validator locally before posting and rejects
publishes whose `base_version` is stale (the hub returns `409
stale_base` with the current latest; the CLI re-pulls, applies your
delta, and retries).

## With curl

The endpoint:

```
POST https://api.foragelang.com/v1/packages/<author>/<slug>/versions
Authorization: Bearer <token>
Content-Type: application/json
```

Body:

```json
{
    "description": "Sweed dispensary platform recipe",
    "category": "dispensary",
    "tags": ["sweed", "cannabis", "maryland"],
    "recipe": "recipe \"zen-leaf\" {\n  step list { … }\n}\n",
    "decls": [
        {"name": "cannabis.forage", "source": "type Dispensary { … }\n"}
    ],
    "fixtures": [
        {"name": "captures.jsonl", "content": "<jsonl bytes>"}
    ],
    "snapshot": {
        "records": { "Product": [/* … */] },
        "counts":  { "Product": 1346 }
    },
    "base_version": null,
    "forked_from": null
}
```

Both `author` and `slug` match `^[a-z0-9][a-z0-9-]{0,38}$`.

`base_version` is the version the publish was rebased from. `null` on
first publish (succeeds only if `<author>/<slug>` doesn't yet exist);
otherwise the hub requires `base_version == latest_version`. On
mismatch the hub returns `409 stale_base` with the current
`latest_version` in the body so the client can rebase.

`forked_from` is `null` on regular publishes. It is set automatically
on the v1 of a fork via `POST /v1/packages/<upstream>/<slug>/fork`,
and points at the upstream version the fork was cut from. After the
fork the lineage pointer never changes; pulls from upstream go through
the regular publish path on the fork.

Returns `201 {author, slug, version, latest_version}`.

## With Studio

Forage Studio's **Publish** tab carries the same flow: form fields for
description / category / tags, **Validate** runs the runtime
parser+validator, **Preview payload** shows the JSON, **Publish** POSTs
to the API.
