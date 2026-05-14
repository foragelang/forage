# forage-hub-api

Cloudflare Worker backing `api.foragelang.com`. Serves the Forage recipe registry.

## Layout

```
src/
  index.ts          — fetch handler + router
  http.ts           — JSON / CORS / R2-stream helpers
  auth.ts           — Bearer-token check
  storage.ts        — KV + R2 helpers, key conventions, SHA-256
  types.ts          — request / response / storage schema types
  routes/
    recipes.ts      — /v1/packages endpoint handlers
test/
  smoke.sh          — curl-based end-to-end test
  run.sh            — convenience wrapper
```

## Storage

- KV (`METADATA`)
  - `recipe:<slug>` — `RecipeMetadata` (latest pointer)
  - `recipe:<slug>:versions` — `VersionRecord[]`
  - `index:list` — `string[]` of slugs (for paginated listing)
- R2 (`BLOBS`)
  - `recipes/<slug>/<version>/<file>.forage` — one blob per `.forage` file
  - `recipes/<slug>/<version>/fixtures.jsonl` (optional)
  - `recipes/<slug>/<version>/snapshot.json` (optional)
  - `recipes/<slug>/<version>/meta.json`

## Endpoints

- `GET  /v1/health`
- `GET  /v1/packages` — `?author=&tag=&platform=&limit=&cursor=`
- `GET  /v1/packages/:slug` — `?version=N`. Returns metadata + every
  file body in `files: [{name, body}, …]`.
- `GET  /v1/packages/:slug/versions`
- `GET  /v1/packages/:slug/fixtures` — JSONL stream, `?version=N`
- `GET  /v1/packages/:slug/snapshot` — JSON stream, `?version=N`
- `POST /v1/packages` — auth required (`Authorization: Bearer $HUB_PUBLISH_TOKEN`)
- `DELETE /v1/packages/:slug` — auth required (soft-delete)

All `GET` endpoints set `Access-Control-Allow-Origin: *`.

## Local dev

```
npm install
npx wrangler dev
```

`wrangler dev` runs the Worker locally against a simulated KV + R2. For real
storage you need to provision the bindings (below) and use the deployed URL.

## Provisioning

One-time setup, run from `hub-api/`:

```
npx wrangler kv namespace create METADATA
# copy the returned id into wrangler.toml's [[kv_namespaces]] id

npx wrangler r2 bucket create forage-hub-blobs

npx wrangler deploy

# Provide the publish token. Keep it secret — do not commit.
echo "$TOKEN" | npx wrangler secret put HUB_PUBLISH_TOKEN
```

The route `api.foragelang.com/*` is declared as a custom domain in
`wrangler.toml` and is created automatically by `wrangler deploy` because the
zone is in this Cloudflare account.

## Smoke tests

After deploying, run:

```
HUB_URL=https://api.foragelang.com HUB_PUBLISH_TOKEN=... ./test/run.sh
```

The script publishes a sample recipe, fetches it, asks for its versions,
publishes again, then soft-deletes it. Each step asserts the expected HTTP
status code.
