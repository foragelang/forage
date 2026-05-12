# Publishing

Three paths to push a recipe to `hub.foragelang.com`:

## CLI

```sh
forage auth login                                       # one-time
forage publish recipes/hacker-news                      # dry-run
forage publish recipes/hacker-news --publish            # actually POST
```

The dry-run prints what would be sent (size, hub URL, auth status).
The live publish reads the source + optional fixtures from the
recipe directory, hashes the body, POSTs to
`https://api.foragelang.com/v1/recipes/<slug>`.

Other flags:

- `--hub <url>` — alternate hub (self-hosted, staging).
- `--token <jwt>` — explicit bearer token. Default reads from
  `$FORAGE_HUB_TOKEN` then the auth store.

## Forage Studio

Open the recipe, switch to the **Publish** tab:

1. **Hub URL** is `https://api.foragelang.com` by default; edit for
   self-hosted hubs.
2. **Signed in as** shows your GitHub login if you've authenticated.
   Click **Sign in with GitHub** otherwise — the device-code flow runs
   inline; the resulting tokens land in macOS Keychain.
3. **Preview (dry-run)** prints what the request would look like.
4. **Publish** issues the POST. Errors land in the validation panel.

## Web IDE

`hub.foragelang.com/edit`:

1. Click **Sign in with GitHub** in the Publish panel.
2. Fill in metadata (display name, summary, tags, license).
3. **Validate** — runs `parse_and_validate` against `forage-wasm` locally.
4. **Publish** — POST against `/v1/recipes/<slug>`.

The web IDE uses the httpOnly cookie set by the OAuth callback; you
don't see the JWT.

## Ownership

The first publish under your GitHub login stamps `ownerLogin: <you>`.
Subsequent publishes to the same slug must come from the same login
(or an admin). Trying to publish under someone else's slug returns
`403 forbidden` with the structured error envelope:

```json
{
    "error": {
        "code": "forbidden",
        "message": "recipe alice/zen-leaf is owned by alice; sign in as that user to publish a new version"
    }
}
```

## Versions

Each publish bumps the version. The hub keeps old versions queryable
forever. `forage publish` doesn't (yet) let you pin a version manually;
the next published version is always `previous + 1`.

## Metadata

```json
{
    "displayName": "Hacker News front page",
    "summary":     "Top 30 stories via the Algolia search API",
    "tags":        ["hn", "news"],
    "license":     "Apache-2.0"
}
```

The CLI sends a minimal metadata payload today (slug, displayName).
Studio's Publish tab will surface the full metadata fields in a R9.1
followup — until then edit the recipe's `recipe.forage` header
comment to describe it.

## Rate limits

The hub-api throttles publishes at 30/min per authenticated user (or
per anonymous IP, which fails immediately on the auth check anyway).
A 429 response carries:

```json
{
    "error": {
        "code": "rate_limited",
        "message": "too many publish requests; retry in 47s",
        "retryAfter": 47
    }
}
```

…plus an HTTP `Retry-After` header.
