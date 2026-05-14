# hub-social-api audit findings

Recovery audit of commits `2663569` (hub-api: per-version-atomic packages +
social surface) and `ebef7e0` (hub-site: discovery surfaces against the new
hub-api). PE merged direct to main without an audit pass; this is the
post-hoc review. Plan contract: `plans/hub-social-api.md`.

## Acceptance commands

- `npm test` in `hub-api/` → **PASS** (5 files, 28 tests, ~960 ms).
- `cd hub-site && npm run build` → **PASS** (clean, with build-time API-fetch
  warnings because the live API is unreachable from this machine; see
  Significant 5).
- `wrangler dev` → **SKIPPED** (no wrangler in PATH on the audit host).

## Findings

### Critical

**C1. Fork stores key under raw `caller.login`, breaks lookup for any
mixed-case GitHub login.**
`hub-api/src/routes/forks.ts:118,133,138-140` — `meta.author = caller.login`
and `indexAddPackage(env, caller.login, forkSlug)` use the JWT subject
verbatim. `caller.login` comes from `gh.login` (`hub-api/src/oauth.ts:105`)
which preserves GitHub's case. KV is keyed `pkg:<author>:<slug>`, so a
`John-Doe` fork lands at `pkg:John-Doe:foo`. Every read goes through
`SEGMENT_RE = /^[a-z0-9][a-z0-9-]{0,38}$/` in
`hub-api/src/routes/packages.ts:32`, applied at `index.ts:118` via
`validateSegments`. Result: `/v1/packages/John-Doe/foo` returns 400 bad_slug;
`/v1/packages/john-doe/foo` returns 404. The fork shows up in `/v1/packages`
listings (because `indexAddPackage` stored `John-Doe/foo` in the index and
the listing's per-ref `getPackage` round-trips through the same uppercase key)
but every direct route to it is unreachable. Worst case: the same user later
publishes `john-doe/foo` through the normal path and now owns two distinct
records.
*Fix:* lowercase `caller.login` (or assert against `SEGMENT_RE`) before using
it as a key segment in `forks.ts`. The same normalization is already applied
to the URL segment by the regex on every other path; forks must match.

**C2. `forked_from` can be spoofed on a non-fork v1 publish.**
`hub-api/src/routes/packages.ts:286-294,330` — the `forked_from`-rejection
branch lives inside `if (existing !== null)`. For a brand-new package
(`existing === null`), no check fires, and line 330 writes whatever the
caller sent: `forked_from: existing?.forked_from ?? payload.forked_from`. So
any caller can `POST /v1/packages/me/foo/versions` with
`{ base_version: null, forked_from: { author: "torvalds", slug: "linux",
version: 1 } }` and the package will carry that lineage. Plan explicitly
reserves `forked_from` for the fork endpoint
(`plans/hub-social-api.md:40-41,116-127`). The 400 error code
`forked_from_on_existing` is also misnamed — what's intended is "only the
fork endpoint sets forked_from".
*Fix:* move the rejection out of the `else` branch, or pass an internal flag
through from `forks.ts` and reject in all other callers.

### Significant

**S1. Star / unstar / download / fork endpoints bypass rate limiting.**
`hub-api/src/index.ts:158-178` — every `POST/DELETE` under
`/packages/:a/:s/{stars,downloads,fork}` calls the handler directly with no
`rateLimit(...)` gate, unlike `versions` (line 146 uses `publish`) and every
`GET` (uses `read`). Anonymous `POST .../downloads` is the worst (no auth at
all, unbounded counter bumps from one IP). Star/unstar/fork are
auth-required but a compromised token can hammer them. The `BUCKETS`
declaration in `http.ts:90-96` defines `publish` / `read` only — neither
fits stars/downloads/forks naturally.
*Fix:* add a `social` bucket (or reuse `publish`) on the four POST/DELETE
endpoints. `downloads` in particular wants something anonymous-rate-limited.

**S2. `idx:top_starred` / `idx:top_downloaded` are dead code.**
`hub-api/src/storage.ts:52-53,353-388` declares the keys, the read
accessors (`getTopStarred`, `getTopDownloaded`), and a `recomputeTopIndexes`
writer — none of which are called from anywhere in `src/`. `listPackages`
sorts by full-scan over `idx:packages` instead
(`packages.ts:69-94`). The plan permits eventual consistency
(`plans/hub-social-api.md:158-161`) but PE shipped neither the cron nor a
lazy-recompute call site; it's YAGNI scaffold that will rot.
*Fix:* either wire `recomputeTopIndexes` into a scheduled handler or a
read-time freshness check, or delete the helpers until they're needed.

**S3. `R2` >20 MiB fallback has no test coverage.**
`hub-api/src/storage.ts:35,106-148` implements the inline-vs-R2 split with a
20 MiB threshold and a transparent read path through `isR2Pointer`. The plan
explicitly calls out the fallback as a required behavior
(`plans/hub-social-api.md:138-142,219-220`) and the audit ask asked for either
a >20 MiB exercise or a low-threshold test. Neither exists; `KV_VERSION_MAX_BYTES`
is hard-coded and never overridden in tests. A regression that breaks
`putVersion`'s large-artifact branch ships silently until production hits the
limit.
*Fix:* parameterize `KV_VERSION_MAX_BYTES` via env (or expose a test hook) and
publish a payload over the threshold in a vitest case asserting both the R2
write and the transparent read.

**S4. `ListProfileStarsResponse` wire type missing `next_cursor`.**
`hub-api/src/types.ts:186-188` declares only `{ items: ProfileStar[] }`, but
`users.ts:118-122` returns
`ListProfileStarsResponse & { next_cursor: string | null }` via an inline
intersection. Wire types are the contract for Studio (Rust) and the IDE
(TS); drift here means a Rust struct without the field will silently lose
pagination. Same pattern in `ListProfilePackagesResponse` (line 175-177) —
also unpaginated even though the handler scans all refs without bounds.
*Fix:* add `next_cursor: string | null` to both response types and have the
handlers emit them honestly (packages endpoint may need a cursor, too — the
loop in `users.ts:78-97` walks the entire `idx:user_packages:<author>` array
without an upper bound).

**S5. Dynamic `r/[author]/[slug]` routes silently emit nothing when the
API is unreachable at build.**
`hub-site/r/[author]/[slug].paths.mjs:6-17` /
`hub-site/u/[author].paths.mjs:6-12` / `hub-site/c/[category].paths.mjs:6-9`
all call `fetchPackages` / `fetchCategories`, which swallow transport errors
and return `[]` (`hub-site/.vitepress/api.mjs:25-28`). If the API is offline
at deploy time (or the deploy is partitioned from
`api.foragelang.com`), the build *succeeds* and ships an empty path set.
Every direct package/profile/category URL then 404s — the SEO/deeplink
story the audit ask flagged is broken in this failure mode. The component
fetch in `PackageDetail.vue` etc. is fine at runtime *once the user is on
the site*, but the dynamic-route paths file is what tells VitePress to emit
HTML stubs for crawlers / direct hits.
*Fix:* fail the build (or at least emit a stub `r/[author]/[slug].md` per
known package via a build-time cache file) when `fetchPackages` returns
zero from the API and a fallback list is also empty.

### Minor

**M1. PE divergence on first-publish stale-base code: 409 vs 400.** The
plan (`plans/hub-social-api.md:102-113`) says match → accept, mismatch →
409, and for first publish "no check; succeeds as v1 if (author, slug)
doesn't exist yet. If it does, 409." It does NOT specify the code for
"first publish with non-null base_version". PE routes both stale cases
through `stale_base` 409 with `latest_version: 0` for the first-publish
case (`packages.ts:255-263`); test pins the behavior
(`packages.test.ts:110-122`). Conceptually clean — the caller's mental
model "my base is stale" is true (their base is 1, real latest is 0).
Document this in the plan; no code change.

**M2. Dynamic route choice (PE flagged).** Plan suggested either pre-render
`r/<author>/<slug>.md` files or use a dynamic route via
`.vitepress/config.ts` (`plans/hub-social-api.md:204-209`). PE used
`r/[author]/[slug].md` with a `.paths.mjs` data loader, which is one of the
two paths the plan greenlit. Renders correctly against a live API. SEO
problem is the build-time fallback (see S5), not the route shape itself.

**M3. `created_at: createdAt ?? 0` masks missing timestamp.**
`hub-api/src/routes/users.ts:65`. The Profile wire type
(`types.ts:166-173`) says `created_at: number` (required). When the OAuth
record is missing the `createdAt` field (legacy admin), 0 is emitted. Per
the discipline rule "never fill in arguments with zero-valued defaults",
either make `created_at` nullable in the wire type or surface the absence
honestly. Trivial.

**M4. `getStars` fetches `meta` only to existence-check.**
`hub-api/src/routes/stars.ts:78-86` reads the entire `PackageMetadata`
JSON to assert presence before listing stars. A `KVNamespace.get` with a
type=text and a returns-null check is the right idiom; reading and parsing
the metadata is wasted work. Pre-1.0 traffic — won't matter, but worth
noting.

**M5. `oldCategory` re-categorize path doesn't dedupe `idx:categories`.**
`hub-api/src/routes/packages.ts:319-347`. When a package moves from
`dispensary` to `menu-platform`, `indexRemoveCategory` clears the package
ref from `idx:cat:dispensary`, but `idx:categories` (the list of all
seen-category names) still contains `dispensary` even if no packages
remain in it. `GET /v1/categories` will return phantom categories. Minor
because `idx:categories` is a UI listing, not an integrity invariant —
clicking a phantom category yields an empty page. Easy fix: after
`indexRemoveCategory`, scan `idx:cat:<old>`; if empty, drop the name from
`idx:categories`.

**M6. `corsPreflight` doesn't echo the requested method.** `http.ts:46-48`
returns a fixed `Allow-Methods: GET,POST,DELETE,OPTIONS`. Browsers accept
it for the methods listed, but the wider lesson: future routes that add
PATCH/PUT (none here) would silently CORS-fail. Not a defect now.

### Questions

**Q1.** The fork endpoint immediately reads the full upstream artifact via
`getVersion` (`forks.ts:64-78`) to copy `recipe + decls + fixtures +
snapshot` into the fork's v1. For a 20 MiB R2-backed artifact, the fork
spends a full R2 fetch and a full KV write. Per the plan that's the
intended shape (the fork's v1 *is* the upstream's content). Confirm: is
there an appetite for a "lazy fork" (storing just `forked_from` and
materializing on first read) or is the materialized-copy semantic
permanent? Affects S3 (R2 test design) and roadmap dependencies.

**Q2.** `getProfile` returns 404 when there are no packages AND no OAuth
record (`users.ts:57-59`). A user who only has *stars* (no packages, no
OAuth record because they came in via admin-token publish elsewhere?) is
unreachable. Is this intentional — a profile only exists if there's
something to anchor it to? Plan doesn't specify. Pre-1.0 the case is
probably moot.

## Verdict

**Request fix-up.** Two critical defects (C1 fork-key casing, C2
forked_from spoof) need a follow-up PE pass on main directly (branch is
gone). S1 (rate limits) and S3 (R2 fallback test) round out the same
follow-up. S2 (dead top-N indexes) and S4 (wire type drift) are
borderline-significant and can ride along or hold for a separate cleanup.
Minor and Question items are commentary, not blockers.
