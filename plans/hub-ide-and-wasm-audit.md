# Audit: `hub-ide-and-wasm` (10 commits 4a76c92..ef12fe3)

Reviewed against `plans/hub-ide-and-wasm.md` and `plans/hub-roadmap.md`.

## Acceptance commands

- `cargo check --workspace`: PASS.
- `cargo check --target wasm32-unknown-unknown -p forage-http --no-default-features`: PASS.
- `cargo check --target wasm32-unknown-unknown -p forage-wasm`: PASS.
- `cargo test --workspace`: PASS (330 passed, 0 failed).
- `npm test` in `packages/studio-ui/`: PASS after `npm install` at workspace root (13 tests).
- `npm test` in `hub-site/ide/`: PASS only after `wasm-pack build --target web` for `crates/forage-wasm`; otherwise hard-fails with `Failed to resolve import "forage-wasm"` (see 🟡 #1).
- `cd hub-site && npm run build`: PASS only after `wasm-pack build` + `FORAGE_HUB_PERMISSIVE_BUILD=1`. The `FORAGE_HUB_PERMISSIVE_BUILD` need is pre-existing; the wasm-pack need is new in this PR.
- `rg 'invoke\(' packages/studio-ui/src`: 42 hits — all in `TauriStudioService.ts`. Outside that adapter: zero. The plan's verbatim "zero hits" target is technically violated by the adapter itself, but the spirit (no scattered `invoke()` in components) is met.
- `rg 'forage-ts'` excluding node_modules / ROADMAP / plans: zero hits in live source.
- `find hub-site -name 'RecipeIDE.vue' -o -name 'RecipeList.vue'`: zero.
- `ls hub-site/forage-ts/`: directory absent.

## Structural verification

1. **forage-http WASM gating** (`crates/forage-http/Cargo.toml:11-18`, `src/lib.rs:15-25`): clean `#[cfg(feature = "native")]` on `pub mod client` and `pub use client::{LiveTransport, ...}`. `reqwest`/`tokio` are `optional = true`. No `cfg_attr` smuggling. Engine, request/response types, `ReplayTransport` stay unconditional. `tokio::time::sleep` only appears inside `client.rs` (gated) and tests; `engine.rs` has no production tokio.
2. **`forage-wasm::run_replay`** (`crates/forage-wasm/src/lib.rs:289-328`): export accepts source + decls + captures JSONL + inputs + secrets, returns snapshot JS, throws JS exceptions on error. Native `run_replay_inner` (lines 220-272) is the pure-Rust core the wrapper delegates to; integration tests at `crates/forage-wasm/tests/replay.rs` exercise it on a real tokio runtime (5 tests).
3. **`packages/studio-ui` move**: `apps/studio/ui/` is gone, all under `packages/studio-ui/`. `apps/studio/src-tauri/tauri.conf.json:7-10` points `frontendDist` to `../../../packages/studio-ui/dist` and the dev/build commands shell into `packages/studio-ui`. `.cargo/config.toml` redirects ts-rs export dir.
4. **`StudioService`** (`packages/studio-ui/src/lib/services/StudioService.ts`): interface defined, both impls present, consumed via React Context (`context.tsx`). `TauriStudioService` wraps `invoke()` / `listen()`; outside the adapter, zero `invoke()` calls.
5. **Hub IDE bundle** (`hub-site/ide/`): standalone Vite + React, aliases `@` to `packages/studio-ui/src`, builds with `base: "/edit/"` to `dist/`. Wired into VitePress build at `hub-site/package.json:8` (`cp -r ide/dist .vitepress/dist/edit`). `hub-site/public/_redirects` rewrites `/edit/*` to `/edit/index.html` for SPA routing on Cloudflare Pages.
6. **`HubStudioService` capabilities** (`hub-site/ide/src/HubStudioService.ts:93-99`): `{workspace:false, deploy:false, liveRun:false, hubPackages:true}`. Studio-only methods reject with `NotSupportedByService`. Tested at `HubStudioService.test.ts:138-156`.
7. **Routing** (`hub-site/ide/src/main.tsx:33-57`): path-based `/edit/<author>/<slug>`. Strips `^\/edit\/?` from `window.location.pathname`, splits on `/`, requires both `author` and `slug`. Trailing slash + missing slug both handled correctly. Commit message claims "hash route" but code is path-routed — non-blocking inconsistency.
8. **VitePress integration**: `PackageDetail.vue:123` links `/edit/${author}/${slug}` and `forage://clone/${author}/${slug}` from the discovery page; no iframe — direct link to the same-origin Pages mount. SEO-friendly (server-rendered VitePress page hosts the link).
9. **Vue + forage-ts deletions**: complete. `hub-site/.vitepress/theme/components/RecipeIDE.vue` + `RecipeList.vue` + `hub-site/forage-ts/` all absent. `hub-site/package.json` has no forage-ts dep.
10. **`ef12fe3` server alignment**: `packages/studio-ui/src/lib/services/StudioService.ts:147-156` `PublishPayload` no longer carries `forked_from`. Server-side `hub-api/src/types.ts:141-150` `PublishRequest` matches — both reject the field. The fix is correct and the surrounding comment documents the rationale.
11. **Greenfield discipline**: no `#[serde(default)]`, no `#[allow(dead_code)]` introduced (the one in `paginate.rs:115` is pre-existing). No `unwrap_or_else(|| "default")` masking in new Rust. `as unknown as` casts in HubStudioService are the only TS escape hatches (see 🟡 #4).
12. **Out-of-scope drift**: no `WebTransport`, no `web-sys::fetch` in `forage-http`. `ReplayTransport` is the hub bundle's transport.
13. **Workspaces root**: `package.json:1-9` lists `packages/*` + `hub-site/ide`. `hub-site/` itself is not a workspace member but shells into `ide` for its build script. `forage-wasm` is consumed via `file:../../crates/forage-wasm/pkg` from the `ide` workspace.

## Findings

### 🟡 Significant

**S1. Hub-site build/test depend on an out-of-band `wasm-pack build`.** `hub-site/ide/package.json:20` declares `"forage-wasm": "file:../../crates/forage-wasm/pkg"`, but nothing in any `package.json` script generates that pkg directory. A fresh clone running `npm install && npm test` in `hub-site/ide` fails with `Failed to resolve import "forage-wasm"`; `cd hub-site && npm run build` fails identically (TS2307 + Vite resolve error) until `wasm-pack build --target web` is run by hand in `crates/forage-wasm/`. The hub-site `build` script (`hub-site/package.json:8`) chains `vitepress build && npm --prefix ./ide run build && cp -r ide/dist .vitepress/dist/edit` — adding `wasm-pack build --target web --release ../crates/forage-wasm &&` (or equivalent) at the front would close the gap; alternatively a workspace-root `build:wasm` script that the `ide` and `hub-site` builds depend on. Without this, the Cloudflare Pages CI build will fail.

**S2. `HubStudioService.authWhoami` returns `null` instead of calling `/v1/oauth/whoami`.** `HubStudioService.ts:375-380` admits in a comment "Hub IDE session lives in cookies — read it through the API when implementing the auth banner; for now return null so the UI doesn't claim a signed-in user." The endpoint exists at `hub-api/src/oauth.ts:370-385` (`GET /v1/oauth/whoami`) with the comment "Useful for the web IDE to detect the sign-in state on page load." Fetching with `credentials: 'include'` is one line. As shipped, the hub IDE never recognises a signed-in user — Star, Fork, Publish flows from the IDE will all fail or render as "sign in" even when a session cookie is present. Either wire the call or remove the auth-keyed affordances entirely; "return null for now" is masked incompletion.

**S3. `EventBus<RunEvent>` is dead code (`HubStudioService.ts:68-81`, `:102`).** `runEvents.emit()` is never called — `runRecipe("replay")` returns a final snapshot synchronously, no per-event push. The comment ("the bus is here so the contract stays compatible") admits speculative future-proofing. Per CLAUDE.md YAGNI, delete the class + field; have `onRunEvent` return `() => {}` like the other no-op subscribers. When streaming arrives, reintroduce.

**S4. `as unknown as` casts in `HubStudioService` bypass the binding types** (`HubStudioService.ts:247, 270, 293, 310, 316, 333`). Each cast manufactures a `RecipeOutline` / `LanguageDictionary` / `RunOutcome` / `DaemonStatus` from an object literal that doesn't structurally satisfy the binding shape — the ts-rs binding for `RecipeOutline` (for example) almost certainly has fields beyond `steps`, and the casts hide that. When the Rust-side type evolves, these casts will silently drift instead of failing the build. Two paths: (a) construct full-shape stubs that genuinely match the bindings; (b) widen the service interface to return narrower hub-friendly types and let consumers handle the variance. Option (a) is the lower-disruption fix.

**S5. `runRecipe` constructs `RunOutcome` from an `{ ok, error, snapshot }` shape** (`HubStudioService.ts:289-317`). The actual `RunOutcome` ts-rs binding is generated from a Rust enum — its shape may be a tagged union (`{ kind: "ok", ... }` etc.), not `{ ok, error, snapshot }`. The `as unknown as RunOutcome` cast lets this compile but the consumer rendering it will see undefined fields. Verify the binding and either reshape the literal or extend the cast comment to call out the binding mismatch. Same issue as S4 but the consumer impact is larger because `RunOutcome` is the run pane's main payload.

**S6. `recipeOutline` stub silently returns empty** (`HubStudioService.ts:239-248`). The function calls `parse_recipe` (which already runs in the validate path), discards the result, then returns an `{ steps: [] }` object cast as `RecipeOutline`. Either expose `recipe_outline` through `forage-wasm` (forage-core::progress + the outliner already exist on the Rust side) or surface the stubbing in the UI by throwing or returning a sentinel. The current code makes the gutter look broken with no signal that the feature is absent in the hub variant.

### 🔵 Minor

**M1. `err.message ?? "stale base"` masking** (`TauriStudioService.ts:446`, `HubStudioService.ts:519`). The 409 envelope is server-controlled — a malformed body should surface, not get papered over. Replace with `err.message` and let the value flow through; if it's `undefined` the `StaleBaseError` carries `undefined` for `message`, which is detectable. Low impact because the server contract is fixed, but the pattern is exactly what CLAUDE.md flags.

**M2. Commit message says "hash route", code is path-routed** (`950bab4` body line 7 vs `main.tsx:33-57`). The implementation is correct (path routing with Cloudflare `_redirects` fallback); the commit message is just wrong. Non-blocking, but worth noting because future spelunking will be misled.

**M3. The plan's `rg 'invoke\(' packages/studio-ui/src` verification was loose-worded.** The verbatim command returns 42 hits because `TauriStudioService.ts` legitimately uses `invoke()`. The plan probably meant "outside `services/`" — outside that directory, zero. Update the plan's verification wording for the next time someone audits to this contract.

**M4. `confirm()` in `HubStudioService` discards the custom `okLabel`/`cancelLabel` options** (`HubStudioService.ts:495-500`). `window.confirm` doesn't support them. Acceptable degradation, but worth a `console.debug` if labels are provided, so a developer sees the limitation rather than wondering why their custom labels don't appear.

**M5. `hub-site/ide/src/test-setup.ts` has its own `matchMedia`/`ResizeObserver` stubs** while `packages/studio-ui/src/test-setup.ts` has equivalents. Two near-identical jsdom stubs that will drift. Consider extracting to a shared `packages/studio-ui/src/test-setup-jsdom.ts` (or `vitest-setup` package) the ide imports. Non-blocking; the duplication is small.

### 💭 Questions

**Q1.** `HubStudioService.runRecipe` hardcodes `inputs={}` and `secrets={}` (lines 303-304). The IDE has no inputs UI today, but the underlying `run_replay` accepts them. Is the plan that the hub IDE never supports recipe inputs (per "read + replay + light authoring"), or is this an interim wiring? If permanent, add a comment in the service method body; if interim, file an issue.

**Q2.** `getPackageVersion(author, slug, "latest")` URL-encodes `author` and `slug` but pastes `latest` as a literal. The server's route handler accepts `latest` as a string. Should `version` be `number | "latest"` or always coerced through `encodeURIComponent`? Today's hardcoded set of values is safe; documenting the contract explicitly would head off future breakage if `version` were ever made caller-controlled freeform.

## Verdict

**Request fix-up.** The structural work is sound: the WASM gating is clean, the service interface is well-shaped, the move to `packages/studio-ui` carried correctly, the deeplink/path-routing wiring is consistent, and the `ef12fe3` server-contract fix aligns with the hub-api side. But two issues block a clean ship:

- S1 (CI build will fail without manual `wasm-pack build`) needs a build-script integration before the next `npm run build` from a fresh clone.
- S2 (`authWhoami` returns null instead of calling the documented endpoint) leaves the hub IDE's auth-keyed flows non-functional even when the user is signed in.

S3-S6 are smaller but worth addressing in the same fix-up pass: delete dead `EventBus`, reshape or fix the `as unknown as` casts, decide on `recipeOutline`. M1-M5 + Q1-Q2 are cleanup that can ride along.
