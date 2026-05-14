# forage-hub-api

Cloudflare Worker backing `api.foragelang.com`. Serves the Forage
package registry — atomic per-version artifacts plus the social
surfaces (stars, downloads, forks, profiles, categories).

## Layout

```
src/
  index.ts            — fetch handler + router
  http.ts             — JSON / CORS / rate-limit helpers
  auth.ts             — JWT + admin bearer-token check
  jwt.ts              — HS256 sign + verify
  oauth.ts            — GitHub OAuth web + device flows
  storage.ts          — KV + R2 helpers, key conventions, indexes
  types.ts            — wire types (PackageMetadata, PackageVersion, …)
  routes/
    packages.ts       — list / detail / versions / publish
    stars.ts          — star / unstar / list stars
    downloads.ts      — download counter
    forks.ts          — fork with one-shot lineage
    users.ts          — profile + their packages + their stars
    categories.ts     — list of seen categories
test/
  *.test.ts           — vitest tests against a real miniflare worker
  smoke.sh            — curl-based end-to-end smoke (live deployment)
vitest.config.ts      — wires the cloudflare:test pool
```

## Storage

- KV (`METADATA`)
  - `pkg:<author>:<slug>` — `PackageMetadata`
  - `ver:<author>:<slug>:<n>` — `PackageVersion` (inline) *or*
    `{"r2_key": "..."}` pointer (when the artifact exceeds 20 MiB
    serialized)
  - `star:<author>:<slug>:<user>` — presence + timestamp
  - `stars_by:<user>:<author>:<slug>` — reverse index for profile pages
  - `idx:packages` — JSON list of `<author>/<slug>` refs (paginated
    catalog)
  - `idx:cat:<category>` — refs per category
  - `idx:user_packages:<author>` — refs per author
  - `idx:categories` — seen category names
  - `idx:top_starred`, `idx:top_downloaded` — eventually-consistent
    rankings, recomputed lazily
- R2 (`BLOBS`)
  - `versions/<author>/<slug>/<n>.json` — atomic version artifacts
    that overflow KV's per-value ceiling

## Endpoints

| Method | Path                                              | Auth     |
|--------|---------------------------------------------------|----------|
| GET    | `/v1/health`                                      | —        |
| GET    | `/v1/packages`                                    | —        |
| GET    | `/v1/packages?sort=&category=&q=&cursor=&limit=`  | —        |
| GET    | `/v1/packages/:author/:slug`                      | —        |
| GET    | `/v1/packages/:author/:slug/versions`             | —        |
| GET    | `/v1/packages/:author/:slug/versions/:n`          | —        |
| POST   | `/v1/packages/:author/:slug/versions`             | Bearer   |
| POST   | `/v1/packages/:author/:slug/stars`                | Bearer   |
| DELETE | `/v1/packages/:author/:slug/stars`                | Bearer   |
| GET    | `/v1/packages/:author/:slug/stars`                | —        |
| POST   | `/v1/packages/:author/:slug/downloads`            | —        |
| POST   | `/v1/packages/:author/:slug/fork`                 | Bearer   |
| GET    | `/v1/users/:author`                               | —        |
| GET    | `/v1/users/:author/packages`                      | —        |
| GET    | `/v1/users/:author/stars`                         | —        |
| GET    | `/v1/categories`                                  | —        |
| `POST` | `/v1/oauth/...`                                   | —        |

All responses set CORS headers (allowlisted origins only).

## Local dev

```
npm install
npm test            # 28 vitest tests against miniflare
npx wrangler dev    # boot the worker locally
```

`wrangler dev` runs the Worker against a simulated KV + R2; the
`vitest-pool-workers` test pool uses the same simulator.

## Provisioning

One-time setup, run from `hub-api/`:

```
npx wrangler kv namespace create METADATA
# copy the returned id into wrangler.toml's [[kv_namespaces]] id

npx wrangler r2 bucket create forage-hub-blobs

npx wrangler deploy

# OAuth + admin secrets — keep secret, do not commit.
echo "$TOKEN" | npx wrangler secret put HUB_PUBLISH_TOKEN
echo "$KEY"   | npx wrangler secret put JWT_SIGNING_KEY
echo "$ID"    | npx wrangler secret put GITHUB_CLIENT_ID
echo "$SEC"   | npx wrangler secret put GITHUB_CLIENT_SECRET
```

The route `api.foragelang.com/*` is declared as a custom domain in
`wrangler.toml`; `wrangler deploy` creates it automatically because
the zone is in this Cloudflare account.

## Smoke tests

```
HUB_URL=https://api.foragelang.com HUB_PUBLISH_TOKEN=... ./test/run.sh
```

The script exercises the per-version-atomic publish path, the
stale-base 409, the downloads counter, the categories list, and
confirms the old singleton sub-resource routes return 404.
