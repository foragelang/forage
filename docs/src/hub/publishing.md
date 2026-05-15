# Publishing

The unit of publication is a **package version** — one atomic
artifact carrying the recipe source, any shared `.forage`
declarations, the replay fixtures, and the snapshot the recipe
produced against those fixtures. Versions ride together; clients
always fetch the whole artifact and never piece it back together from
sub-resources.

Two paths to push to `hub.foragelang.com`:

## CLI

```sh
forage auth login                                              # one-time
forage publish ~/Library/Forage/Recipes/hacker-news            # dry-run
forage publish ~/Library/Forage/Recipes/hacker-news --publish  # POST
```

The dry-run prints the publish envelope (file count, total size, hub
URL, auth status, current `latest_version` for the slug). The live
publish reads every `.forage` file in the workspace plus the most
recent fixtures + snapshot under `.forage/replay/`, packs them into a
single envelope, and POSTs to
`https://api.foragelang.com/v1/packages/<author>/<slug>/versions`.

Other flags:

- `--hub <url>` — alternate hub (self-hosted, staging).
- `--token <jwt>` — explicit bearer token. Default reads from
  `$FORAGE_HUB_TOKEN` then the auth store.

## Forage Studio

Open a recipe in the workspace, click **Publish**:

1. **Signed in as** shows your GitHub login if you've authenticated.
   Click **Sign in with GitHub** otherwise — the device-code flow runs
   inline; the resulting tokens land in macOS Keychain.
2. **Description**, **Category**, **Tags** populate the package
   metadata.
3. **Preview (dry-run)** prints what the request would look like.
4. **Publish** issues the POST. Errors land in the validation panel.

## Ownership

The first publish under your GitHub login stamps `owner_login: <you>`
on the package metadata. Subsequent publishes to the same slug must
come from the same login (or an admin token). Publishes under a slug
that does not match your GitHub login return `403 forbidden`:

```json
{
    "error": {
        "code": "forbidden",
        "message": "you are signed in as @alice; cannot publish to @bob"
    }
}
```

## Versions and stale-base detection

`base_version` is the version the publisher rebased from. On `v1`
it's `null` (and the slug must not exist yet). On `vN+1` it must
match the current `latest_version`. If a teammate ships `vN+1` while
you were drafting against `vN-1`, your publish is rejected:

```json
{
    "error": {
        "code": "stale_base",
        "message": "base is stale, rebase to v3 and retry",
        "latest_version": 3,
        "your_base": 2
    }
}
```

The CLI and Studio handle this by pulling the latest, replaying your
delta on top, and retrying.

## Categories and tags

`category` is required and matches `^[a-z0-9][a-z0-9-]*$`. The hub
maintains the set of seen categories at `GET /v1/categories`.

`tags` is an array of free-form strings (up to 16). They are not
indexed for search yet; treat them as descriptive metadata.

## Forks

`POST /v1/packages/<upstream-author>/<slug>/fork` creates
`@me/<slug>` (or a renamed slug via `{"as": "name"}`) with version 1
carrying the upstream's full content plus `forked_from: {author,
slug, version}`. The fork is independent after creation — there is
no auto-tracking. Pulls from upstream are explicit overwrites done
through the regular publish path.

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
