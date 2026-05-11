# Forage — full-product roadmap

The product is four artifacts on top of one runtime:

1. **Runtime library** (`Sources/Forage/`) — parses, validates, runs recipes; ships as a Swift package.
2. **CLI tool** (`Sources/forage-cli/`) — thin wrapper around the runtime; ships as `forage` binary.
3. **Toolkit app** (`Toolkit/`) — macOS SwiftUI app for interactive recipe authoring; embedded WKWebView; publishes to the hub.
4. **Hub** (`hub-api/` + `hub-site/`) — registry at `hub.foragelang.com` with API at `api.foragelang.com`; includes an in-browser editor for recipes that don't need a real browser engine.

This doc lays out six milestones (M1–M6) that take us from "runtime + half a CLI" today to "all four artifacts shipping and integrated."

Each milestone has a result-statement, concrete deliverables (acceptance-testable), and dependencies. Build top-to-bottom; later milestones expect earlier ones to be landed.

---

## M1 — Finish the CLI

**Status: landed.** All five subcommands (`run`, `capture`, `scaffold`, `test`, `publish`) live in `Sources/forage-cli/`; `Tests/ForageTests/ScaffoldTests.swift` + `TestCommandTests.swift` cover the heuristics + harness; `site/docs/cli.md` is the subcommand reference. `publish` shipped beyond the M1 stub — it actually POSTs via `HubClient` when `FORAGE_HUB_TOKEN` is set (M4 wiring landed alongside).

**Result:** `forage` is a polished single-binary CLI with subcommands `run`, `capture`, `scaffold`, `test`, `publish` (publish prints what it would do until M4 wires it live).

Builds on A1's in-progress work in `Sources/forage-cli/`. Phase 8 (`unhandledAffordances`) is already committed (`68da6bf`); the rename + ArgumentParser skeleton is uncommitted.

**Deliverables**

- **D1.1 — Land the rename.** Commit the uncommitted `Sources/forage-cli/` files + `Package.swift` change + `Sources/forage-probe/` deletion. `swift build` must produce a `forage` binary.
- **D1.2 — `forage run <recipe>`** — full functional parity with old `forage-probe run`. Auto-detects engine kind. `--input k=v` repeatable. Prints `RunResult.snapshot` as JSON, `RunResult.report.stallReason` + sections to stderr.
- **D1.3 — `forage capture <url>`** — full parity with old `forage-probe capture`. Embedded WKWebView, JS-injected fetch/XHR wrapper, JSONL output. `--out`, `--settle`, `--timeout`.
- **D1.4 — `forage scaffold <captures.jsonl>`** — Phase 9. Real heuristics:
  - Parse JSONL captures. Group by URL pattern (strip numeric IDs + query strings; bucket structurally).
  - For the dominant pattern, decode response bodies; find the longest nested array (probable items list).
  - Walk item fields → infer Swift-ish types (`id`/`*Id` → String, `price`/`*Price` → Double, `name`/`title` → String, `available`/`*` boolean → Bool, image-url keys → String).
  - Pick engine: `application/json` content-type → http; `text/html` → browser.
  - Emit a recipe skeleton with type decl + `captures.match` (browser) or `step + paginate.untilEmpty` (http) + emit blocks for inferred fields + a basic `expect { records.where(typeName == "X").count >= 1 }`.
  - `--out path` writes to file; default stdout.
- **D1.5 — `forage test <recipe-dir>`** — Phase 10:
  - Recipe dir layout: `recipe.forage`, `fixtures/captures.jsonl` (and/or HTTP fixtures), `fixtures/inputs.json` (optional), `expected.snapshot.json` (optional golden file).
  - Run via `BrowserReplayer` (browser engines) or `HTTPReplayer` (http engines).
  - If `expected.snapshot.json` exists: structural diff against produced snapshot, exit 0 on identity, 1 on mismatch.
  - If absent: print produced snapshot, exit 0 (suggest `--update` to write).
  - `--update` writes the produced snapshot to `expected.snapshot.json`.
  - Surface `RunResult.report.unmetExpectations` — non-zero exit if any.
- **D1.6 — `forage publish <recipe-dir>` stub** — validates the recipe, prints the JSON payload it would POST to `$FORAGE_HUB_URL/v1/recipes` (default `https://api.foragelang.com`). If `$FORAGE_HUB_TOKEN` is set AND `--no-dry-run` is passed, actually POST (but with friendly "not yet wired" message until M4).
- **D1.7 — Tests.** New `Tests/ForageTests/ScaffoldTests.swift` and `Tests/ForageTests/TestCommandTests.swift`. Synthetic JSONL → assert recipe shape; synthetic recipe + expected → assert diff behavior.
- **D1.8 — Docs.** `site/docs/cli.md` (new) — full subcommand reference. Update site sidebar.

**Acceptance**

- `swift build` clean; `swift test` all pass.
- `swift run forage --help` lists subcommands.
- `swift run forage scaffold tests/fixtures/sample-captures.jsonl` produces a recipe that passes `Parser.parse` + `Validator.validate`.
- `swift run forage test recipes/sweed --update` writes an `expected.snapshot.json`; subsequent `swift run forage test recipes/sweed` exits 0.

---

## M2 — Hub: backend + frontend

**Status: landed.** `hub-api/` is the Cloudflare Worker (KV `METADATA`, R2 `BLOBS`, Bearer auth in `src/auth.ts`, all `/v1/*` endpoints in `src/index.ts`, smoke-test scripts in `hub-api/test/`). `hub-site/` is the VitePress browse + detail UI (`index.md`, `r/[slug].md` + `[slug].paths.mjs` dynamic loader, `publish.md`, `about.md`). Both deploy via wrangler; production is at api.foragelang.com / hub.foragelang.com.

**Result:** `api.foragelang.com` serves a working recipe registry; `hub.foragelang.com` renders a browse + detail UI on top of it.

Two new directories in this repo: `hub-api/` (Cloudflare Worker), `hub-site/` (VitePress site). Both deploy via wrangler — backend via `wrangler deploy`, frontend via `wrangler pages deploy` (or the dashboard-wired auto-build that already exists for the `forage` site).

**Deliverables**

- **D2.1 — `hub-api/` scaffold.** TypeScript Worker. `wrangler.toml`. Bindings: `METADATA` (KV), `BLOBS` (R2), env `HUB_PUBLISH_TOKEN` (secret). Local dev via `wrangler dev`.
- **D2.2 — Storage schema.**
  - KV: `recipe:<slug>` → JSON `{slug, author, displayName, summary, tags[], version, latestBlobKey, createdAt, updatedAt}`.
  - KV: `recipe:<slug>:versions` → JSON array of `{version, blobKey, publishedAt, sha256}`.
  - KV: `index:list` → JSON array of slugs (denormalized index; rebuilt on every publish).
  - R2: `recipes/<slug>/<version>/recipe.forage` (the body).
  - R2: `recipes/<slug>/<version>/fixtures.jsonl` (optional).
  - R2: `recipes/<slug>/<version>/snapshot.json` (optional).
  - R2: `recipes/<slug>/<version>/meta.json` (snapshot of metadata at publish time).
- **D2.3 — Endpoints.**
  - `GET /v1/health` — `{status: "ok"}`.
  - `GET /v1/recipes` — paginated list. Query: `?author=&tag=&platform=&limit=&cursor=`. Returns `{items: [...meta...], nextCursor?}`.
  - `GET /v1/recipes/:slug` — `{...metadata..., body: "<recipe.forage content>"}`. `?version=` to get historical.
  - `GET /v1/recipes/:slug/versions` — `[{version, publishedAt, sha256}]`.
  - `GET /v1/recipes/:slug/fixtures` — JSONL stream of captures from R2 (if present).
  - `GET /v1/recipes/:slug/snapshot` — JSON snapshot from R2 (if present).
  - `POST /v1/recipes` — publish. Auth: `Authorization: Bearer <HUB_PUBLISH_TOKEN>`. Body: `{slug, displayName, summary, tags, body, fixtures?, snapshot?}`. Validates that `body` parses + validates (server-side: ship the Forage parser+validator as a Wasm module bundled with the Worker, OR run a strict structural check + defer full validation to client). For v1: server-side does a lightweight regex sanity check; client (CLI) does full validation pre-publish.
  - `DELETE /v1/recipes/:slug` — auth required; soft-delete (mark `deleted: true` in KV metadata).
- **D2.4 — Auth.** API-key based for v1. `Authorization: Bearer <key>`. Single shared `HUB_PUBLISH_TOKEN` secret in the Worker; v2 will go to per-author OAuth keys.
- **D2.5 — Deploy.**
  - Create the Worker via `npx wrangler deploy`.
  - Provision KV namespace + R2 bucket via wrangler.
  - Bind `api.foragelang.com` as a custom domain on the Worker (`wrangler` supports this; DNS records auto-created since the zone is in CF).
  - Set the secret: `npx wrangler secret put HUB_PUBLISH_TOKEN` (user picks the value; I'll prompt at the right moment).
- **D2.6 — Integration tests.** `hub-api/test/` directory with curl-based smoke tests: publish a recipe, list it, get it, get a version, delete, list again. Documented in `hub-api/README.md`.
- **D2.7 — `hub-site/` scaffold.** New VitePress site (consistent with foragelang.com). `package.json`, `.vitepress/config.mjs`, public/favicon. Linked Forage grammar for syntax highlighting on detail pages.
- **D2.8 — Hub-site pages.**
  - `index.md` — home: list recent recipes (fetched client-side from api.foragelang.com), search box, filter by tag/platform.
  - `r/[slug].md` (dynamic via VitePress's data loader) — recipe detail: metadata, source with Forage syntax highlighting, fixtures + snapshot summaries, "use in CLI" / "open in Toolkit" code blocks.
  - `publish.md` — instructions for publishing (CLI + Toolkit flows).
  - `about.md` — what is forage, link to foragelang.com.
- **D2.9 — Deploy hub-site.** Create CF Pages project `forage-hub` pointed at this repo, build command `cd hub-site && npm ci && npm run build`, output `hub-site/.vitepress/dist`, custom domain `hub.foragelang.com`.

**Acceptance**

- `curl https://api.foragelang.com/v1/health` returns `{"status":"ok"}`.
- `curl -X POST -H "Authorization: Bearer $TOKEN" https://api.foragelang.com/v1/recipes -d @sample-payload.json` returns `{slug, version, ...}`.
- `curl https://api.foragelang.com/v1/recipes/<slug>` returns the recipe.
- Visiting `https://hub.foragelang.com/r/<slug>` renders the recipe with syntax-highlighted source.

---

## M3 — Toolkit app

**Status: mostly landed.** `Toolkit.app` builds via `xcodegen` + `xcodebuild` and launches; all five editor tabs (Source/Fixtures/Snapshot/Diagnostic/Publish), the capture scene, run controller (live + replay), library sidebar, MFA prompt, Keychain, and Preferences are wired. PublishTab actually POSTs via `HubClient`. Remaining gaps, in decreasing order of visible:

- **D3.2: "Import from hub" button is a disabled placeholder** (`LibrarySidebar.swift`). Hub-side browse + import flow not yet built.
- **D3.7: AppIcon ships no PNG slices** — only the `Contents.json` manifest. Default Xcode placeholder icon today.
- **D3.7: app menu set is incomplete** — New Recipe / Run Live / Run Replay / Capture / Save are wired; no explicit "Publish" / "Validate" / "Import from Hub" menu commands.

The Toolkit is operational for everything except hub-import; users can still pull a recipe by checking out the file or by `gh api` against `api.foragelang.com`.

**Result:** A `Toolkit.app` (macOS SwiftUI) that authors recipes interactively and publishes to the hub.

New directory `Toolkit/` in the forage repo. xcodegen-generated `Toolkit.xcodeproj`. Depends on the local Forage package (relative path).

**Deliverables**

- **D3.1 — `Toolkit/` scaffold.**
  - `Toolkit/project.yml` (xcodegen).
  - `Toolkit/Sources/Toolkit/` SwiftUI sources.
  - `Toolkit/Sources/Toolkit/ToolkitApp.swift` — `@main App` with NavigationSplitView.
  - Depends on `../` (the Forage package) via local SwiftPM ref.
- **D3.2 — Recipe library scene.**
  - Sidebar: list of local recipes (under `~/Library/Forage/Recipes/<slug>/`) + recently-pulled hub recipes.
  - "New recipe" button → create a new local slug, scaffold blank recipe.
  - "Import from hub" button → search hub via the API, pick one, copy to local.
- **D3.3 — Recipe editor scene.**
  - Tabbed view: Source / Fixtures / Snapshot / Diagnostic / Publish.
  - **Source tab:** text editor for `recipe.forage`. NSTextView-based with a custom syntax-highlighting layer derived from the Forage grammar. Cmd-S saves; Cmd-R runs.
  - **Fixtures tab:** list of captures in the recipe's `fixtures/captures.jsonl`. Each row: method, URL, status, body size. Click → inspector showing decoded body. "Capture fresh" button → opens Capture scene.
  - **Snapshot tab:** record counts grouped by type. Click into a type → table view of records' fields.
  - **Diagnostic tab:** sections from `DiagnosticReport` (stallReason / unmatchedCaptures / unfiredRules / unmetExpectations / unhandledAffordances). Each section has expandable rows.
  - **Publish tab:** form (slug, display name, summary, tags, license). "Validate" runs `Validator.validate`. "Preview payload" shows the JSON that would POST. "Publish" actually POSTs (requires API key configured in app prefs).
- **D3.4 — Capture scene.**
  - Modal sheet (or separate window).
  - Address bar: URL input.
  - Embedded WKWebView (using `BrowserEngine`-style capture wrapper but live, not driven by a recipe).
  - Live capture feed: list of observed fetch/XHR calls. Each row toggleable to mark "keep" vs "skip."
  - Save button → writes selected captures to the open recipe's `fixtures/captures.jsonl`.
- **D3.5 — Run scene.**
  - Run modes: "Run live" (URLSessionTransport / live BrowserEngine) / "Run against fixtures" (replayer).
  - Live progress (read from `BrowserProgress` / `HTTPProgress`).
  - On finish: update Snapshot + Diagnostic tabs.
- **D3.6 — Hub integration.**
  - `HubClient.swift` in `Toolkit/Sources/Toolkit/Networking/`. List / get / publish via `api.foragelang.com`.
  - API key stored in macOS Keychain (`SecItemAdd` / `SecItemCopyMatching`).
  - Preferences pane (Cmd-,): set hub URL (default), set API key.
- **D3.7 — App resources.**
  - App icon (placeholder; SVG → PNG slices).
  - Menu commands: New Recipe, Import from Hub, Publish, Run, Capture, Validate.
- **D3.8 — Local recipe storage convention.**
  - `~/Library/Forage/Recipes/<slug>/recipe.forage`
  - `~/Library/Forage/Recipes/<slug>/fixtures/captures.jsonl`
  - `~/Library/Forage/Recipes/<slug>/snapshots/<ts>.json`
  - `~/Library/Forage/Cache/hub/<slug>/<version>/recipe.forage`
- **D3.9 — Docs.** `site/docs/toolkit.md` — user guide with screenshots.

**Acceptance**

- `xcodegen` in `Toolkit/`; build via `xcodebuild`; `Toolkit.app` launches.
- Create a new recipe, capture from a URL, run against fixtures, view snapshot.
- Configure hub API key, publish to hub, see the recipe on `hub.foragelang.com`.

---

## M4 — Integration: runtime `hub://` imports + `forage publish` live

**Status: landed.** `Sources/Forage/Hub/HubClient.swift` (get / list / publish, reads `FORAGE_HUB_URL` + `FORAGE_HUB_TOKEN`). `Sources/Forage/Hub/RecipeImporter.swift` resolves `import hub://author/slug` directives recursively, unions types/enums/inputs, caches at `~/Library/Forage/Cache/hub/`. CLI publish goes live via `HubClient.publish` (with `--no-dry-run`). Toolkit publish goes live. `scripts/e2e-publish.sh` is the documented round-trip flow. `site/docs/hub.md` covers it.

**Result:** The runtime can pull recipes from the hub; CLI `forage publish` and Toolkit's publish button both write to the live hub.

**Deliverables**

- **D4.1 — `HubClient` in the runtime.** `Sources/Forage/Hub/HubClient.swift`. Get / list / publish. Reads `FORAGE_HUB_URL` (default `https://api.foragelang.com`). Auth via `FORAGE_HUB_TOKEN` or app-supplied key.
- **D4.2 — Recipe `import` directive.** Parser support for `import hub://author/slug` as a top-level recipe-file statement. Validator resolves the import via `HubClient.get(slug:)`; recipe is fetched + cached at `~/Library/Forage/Cache/hub/<author>/<slug>/<version>/recipe.forage`. The imported recipe's types + transforms + emit blocks become available in the importing recipe. (Simpler v1: imports are "include" — flat text concatenation pre-parse. v2: namespaced.)
- **D4.3 — CLI publish goes live.** `forage publish <recipe-dir>` actually POSTs. `--dry-run` keeps the M1 behavior.
- **D4.4 — Toolkit publish goes live.** Same — Publish button writes to api.foragelang.com.
- **D4.5 — End-to-end smoke test.** A `scripts/e2e-publish.sh` that:
  1. Builds `forage`.
  2. Runs `forage scaffold` on a checked-in synthetic captures file.
  3. Runs `forage publish --dry-run` against the resulting recipe.
  4. Then runs `forage publish` for real (requires `FORAGE_HUB_TOKEN`).
  5. Curls the resulting `GET /v1/recipes/<slug>` and asserts the body round-trips.
- **D4.6 — Docs.** `site/docs/hub.md` — how publish + import work.

**Acceptance**

- `forage publish recipes/sample/` succeeds.
- `import hub://forage/sample` resolves; importing recipe runs.

---

## M5 — Distribution

**Status: mostly landed.** `.github/workflows/release.yml` builds + signs + notarizes + packages on `v*.*.*` tags. `site/public/install.sh` is the curl-pipe-sh installer; `site/docs/install.md` documents brew / curl / build-from-source paths. Remaining gaps:

- **D5.2: external `foragelang/homebrew-tap` GitHub repo doesn't exist yet.** The local `homebrew-tap/` directory contains the formula with a `PLACEHOLDER_SHA256_FROM_RELEASE` placeholder; the `update-homebrew-tap` job in the release workflow is gated behind `ENABLE_HOMEBREW_TAP_UPDATE` + `HOMEBREW_TAP_TOKEN`. `brew install foragelang/forage/forage` will work only once the tap repo is created and the workflow flag is flipped on.
- **D5.5: homepage `/download` CTA / nav entry not yet added to `site/`.** The install page exists at `/docs/install`; no top-level "Download" entry from the nav, no homepage hero CTA pointing at it.

**Result:** `forage` and `Toolkit.app` are installable via `brew`, `curl | sh`, and `.dmg`.

**Deliverables**

- **D5.1 — Release workflow.** `.github/workflows/release.yml`. Triggers on tag `v*`. Steps:
  - Build CLI: `swift build -c release --arch arm64` (+ x86_64 if cheap).
  - Build Toolkit: `xcodebuild -project Toolkit/Toolkit.xcodeproj -scheme Toolkit -configuration Release archive`.
  - Codesign + notarize Toolkit (requires `APPLE_DEVELOPER_ID`, `APPLE_API_KEY`, `APPLE_TEAM_ID` secrets).
  - Package: tar.gz the CLI, DMG the Toolkit (`create-dmg` script).
  - Compute sha256 for each artifact.
  - Create GitHub Release with artifacts + sha256s.
- **D5.2 — Homebrew tap.** New repo `foragelang/homebrew-tap`. Formula `Formula/forage.rb` references the latest release's tarball + sha256. Release workflow updates the formula automatically via a PR or direct push.
- **D5.3 — curl-pipe-sh installer.** `site/public/install.sh`. Detects macOS arm64; fetches latest release tarball via GitHub API; verifies sha256; installs to `~/.local/bin/forage`; prints PATH hint. Lives at `https://foragelang.com/install.sh`.
- **D5.4 — Download page.** `site/docs/install.md` (or `/download`). Three install paths: brew, curl, build-from-source. Toolkit `.dmg` direct download link.
- **D5.5 — Site updates.** Homepage CTA points at `/download`; nav adds a "Download" entry.

**Acceptance**

- `brew install foragelang/forage/forage` works.
- `curl -fsSL https://foragelang.com/install.sh | sh` works.
- Visiting foragelang.com/download shows three flows.
- Downloading `Toolkit.dmg`, mounting, dragging to Applications, launching — works without Gatekeeper rejection.

---

## M6 — In-browser tooling (web IDE on hub.foragelang.com)

**Status: mostly landed (architecture diverged).** The user-facing goal is met — `hub.foragelang.com/edit` and `/r/<slug>/edit` host a Monaco-based editor with live parse + validate + run + publish via the `<RecipeIDE />` Vue component (`hub-site/.vitepress/theme/components/RecipeIDE.vue`). The parser/validator/runner run in-browser through the **TypeScript port** (`hub-site/forage-ts/src/`), not the SwiftWasm artifact originally specified in D6.1. Same goal, cheaper-to-maintain route. Remaining gaps relative to the original deliverable shape:

- **D6.1 SwiftWasm artifact not built.** Replaced by `forage-ts`, which is kept in lockstep with the Swift implementation via `Tests/shared-recipes/` drift-detection vectors. The original ROADMAP wording is wrong on the route; the spirit is met.
- **D6.3 auth flow is bearer-token in localStorage**, not GitHub OAuth. Acceptable for a v1.
- **D6.4 "New recipe" entry point**: editable via direct nav to `/edit`, hero CTA in place; haven't verified a "New" link from the recipe-list view.

**Result:** Recipes can be browsed, edited, validated, and published from `hub.foragelang.com` without installing anything.

**Deliverables**

- **D6.1 — Wasm build of parser+validator.**
  - SwiftWasm toolchain installed in CI.
  - New target in `Package.swift`: `ForageWasm` (or similar) — a stripped-down library that excludes `AppKit`/`WebKit` deps. Just parser + validator + JSONValue + Recipe types + transform impls.
  - Build target: `swift build --triple wasm32-unknown-wasi -c release`, output a `.wasm` artifact.
  - Export functions: `parse(source: string) -> Recipe | error`, `validate(recipe: Recipe) -> issues[]`.
  - Bundle into `hub-site/public/forage-wasm/forage.wasm` + a JS shim.
- **D6.2 — Web IDE page.** `hub-site/r/[slug]/edit.md` (or React/Svelte sub-app embedded into VitePress).
  - Monaco editor with custom Forage tokens (mode definition derived from the Shiki grammar).
  - Live validation: as you type, run the Wasm parser+validator, display errors inline.
  - Fixture inspector pane.
  - Snapshot diff pane (compares produced vs expected — when "Run" succeeds).
  - "Run" button: for HTTP-engine recipes, executes against fixtures (in-browser using fetch + the JS shim around the Wasm runtime). Browser-engine recipes are unsupported in-browser; show "Open in Toolkit" deep link.
  - "Publish" button: POST to api.foragelang.com with the edited body.
- **D6.3 — Auth flow for web.** v1: paste API key into a localStorage-backed pref. v2: GitHub OAuth flow with the api Worker as the OAuth client.
- **D6.4 — Update hub home + recipe detail pages.** "Edit on web" button on each recipe; "New recipe" entry on the home.
- **D6.5 — Docs.** `site/docs/web-ide.md` (or `hub-site/about.md`) — what's possible in the IDE vs the Toolkit.

**Acceptance**

- Visit `hub.foragelang.com/r/<slug>/edit`, see the recipe in Monaco, edit, see live validation errors, save, see the new version on `hub.foragelang.com/r/<slug>`.

---

## M7 — Authenticated sessions

**Status: landed.** `auth.session.{formLogin,bearerLogin,cookiePersist}` parses end-to-end. `SecretResolver` resolves `$secret.*` from `FORAGE_SECRET_<NAME>` env vars (CLI) or Keychain (Toolkit). `MFAProvider` protocol with stdin-prompt (CLI) and modal-sheet (Toolkit) implementations. Session caching at `~/Library/Forage/Cache/`; re-auth + retry on 401/403 with configurable `maxReauthRetries`. `Tests/ForageTests/SessionAuthTests.swift` covers the cases; `site/docs/auth-sessions.md` is the guide; `recipes/sample-login/` is the exemplar.

One small caveat: **D7.10's `cacheEncrypted` flag** is wired through the parser/AST but the encryption-at-rest step in the engine path may still need the file-mode + key-derivation hardening described in the deliverable. Worth a focused security pass before recommending the cache for sensitive credentials.

**Result:** recipes can declare a login flow and maintain authenticated state across requests. Today's `auth` block supports `staticHeader` (API key in a header) and `htmlPrime` (one-shot GET to extract a CSRF token + set cookies). Neither covers "log in as me, maintain a session across requests, refresh when it expires." M7 adds that as a first-class DSL feature.

**Deliverables**

- **D7.1 — `auth.session { … }` block in the DSL.** New top-level auth strategy. Three variants, each with its own block:
  - `auth.session.formLogin { url, method, body, captureCookies }` — POST credentials to a login endpoint; capture `Set-Cookie`s; reuse in subsequent step requests automatically.
  - `auth.session.bearerLogin { url, method, body, tokenPath, headerName: "Authorization", headerPrefix: "Bearer " }` — POST credentials to a token endpoint; extract token from the response via `tokenPath`; inject as `Authorization: Bearer <token>` on every subsequent step.
  - `auth.session.cookiePersist { sourcePath }` — load cookies from a JSON or Netscape-format file. Escape hatch for sites that need MFA the recipe can't navigate.
- **D7.2 — Credential references.** Credentials never live in the recipe text. The DSL gains a `$secret.<name>` path resolver. Runtime resolves at execution time:
  - CLI: reads from `FORAGE_SECRET_<NAME>` environment variables.
  - Toolkit: reads from macOS Keychain under a per-recipe service identifier.
  - Web IDE: prompts for each `$secret.*` reference inline before run; never persisted to the hub.
- **D7.3 — Session lifecycle.**
  - Engine detects `401` / `403` mid-run, re-runs the login flow once, retries the failed request. Configurable: `auth.session.maxReauthRetries: 1` (default 1, 0 to disable).
  - On total auth failure (re-auth itself fails), `DiagnosticReport.stallReason` becomes `auth-failed: <details>`.
- **D7.4 — Session persistence (optional cache).** `auth.session.cache: <duration>` — caches the session token/cookies for `duration` seconds keyed by `(recipe-slug, credential-fingerprint)` at `~/Library/Forage/Cache/sessions/`. Subsequent runs reuse without re-logging-in. Eviction on expiry or 401.
- **D7.5 — MFA hook.** `auth.session.requiresMFA: true` — engine pauses the run and emits a `mfaChallenge` event the host handles:
  - CLI: blocks on `stdin`, prompts "Enter MFA code:".
  - Toolkit: shows a modal sheet asking for the code; resumes on submit.
  - Web IDE: same modal; submits via JS.
- **D7.6 — Parser + Validator.** `auth.session.*` parses to a new `AuthStrategy` case. Validator checks that credential references match declared `$secret.*` references (warning if a referenced secret has no obvious source).
- **D7.7 — Runtime support.**
  - `HTTPEngine` runs the login flow before the first step; threads cookies/headers automatically.
  - `BrowserEngine` writes the captured cookies into the `WKWebView`'s data store (`HTTPCookieStorage`) so SPA fetches inherit them.
- **D7.8 — Tests.**
  - Unit: a synthetic recipe with `auth.session.formLogin` + a mock URLSession that returns `Set-Cookie` on POST and 200 with the cookie on GET → snapshot shows the right records.
  - Unit: 401 mid-run triggers a single re-auth + retry; on re-auth failure, `stallReason: "auth-failed: …"`.
  - Unit: MFA hook called with a synthetic code provider.
  - Integration: a real recipe against a documented test-login API (e.g. httpbin or a self-hosted mock).
- **D7.9 — Docs.**
  - New `site/docs/auth-sessions.md` — concrete examples per variant.
  - Update `recipes/` with a `recipes/sample-login/` exemplar.
- **D7.10 — Security review.**
  - Credentials never logged. Diagnostic reports redact `$secret.*` resolved values.
  - Cache files are `chmod 600`. Cookie cache encrypted with a per-machine key (use Keychain on macOS to derive).
  - The web IDE's runtime never persists secrets to localStorage or the hub.

**Acceptance**

- A recipe with `auth.session.formLogin { … } / auth.session.bearerLogin { … }` runs end-to-end against a mock server in tests.
- 401 mid-run triggers exactly one re-auth attempt; second 401 fails with the right diagnostic.
- MFA hook fires; CLI prompts; Toolkit shows a sheet; web IDE shows a modal.
- Recipe text contains zero credential material; all references are `$secret.*`.

---

## Current state (2026-05-11)

M1–M9 are all in some state of landed. The original M1→M7 path was followed serially; M6 took a TypeScript-port route instead of SwiftWasm; M8 and M9 followed up after M7 to handle HTML extraction and browser-engine document capture. The audit notes per milestone above are the up-to-date truth — the original "Order of execution" notes below are kept for the historical record.

Followups that *should* land but haven't:

- M3 hub-import sidebar action (currently disabled placeholder).
- M3 AppIcon real PNG slices + Publish/Validate/Import menu commands.
- M5 external `homebrew-tap` repo + the `update-homebrew-tap` workflow flag flipped on.
- M5 homepage `/download` CTA + nav entry.
- M6 OAuth replacement for the bearer-token-in-localStorage in the web IDE.
- M7 D7.10 cache-encryption-at-rest review (parser/AST exposes `cacheEncrypted` but engine implementation deserves a focused pass).

Next milestone in flight:

- **M10 — Interactive session bootstrap.** Human-in-the-loop CAPTCHA / age-gate / sign-in handshake; persisted session is reused headlessly until expiry. Covers eBay-class sites without violating the "no bypassing technical controls" policy (a human *did* pass the control; the bot reuses the human-authorized session). See bottom of file once landed.

---

## Order of execution (historical)

Serial, top-to-bottom. Each milestone gets a `product-engineer` agent dispatch with the full milestone brief, followed by a `code-review-auditor` pass on the resulting diff. Findings from the auditor get a focused fixup pass before moving on.

Milestone 6 (web IDE) is gated on M5 only by docs convenience — it could land in parallel with M3/M4/M5 once M2 is up. But sequential is simpler to manage.

M7 (authenticated sessions) is independent — it can land any time after M1, since it's purely runtime + DSL. Queued until the M1–M6 path is live and we know what real-world recipe authors hit walls on.

Once M1-M7 are all landed, forage covers static-key, CSRF-priming, AND session-based auth — i.e. essentially any non-CAPTCHA public-web flow.

M8 (HTML / DOM extraction) is also independent and is **landed** — added below for the historical record. M9 (browser-engine document capture) is the natural follow-on that lights up eBay-class sites; queued similarly to M7.

---

## M8 — HTML / DOM extraction

**Result:** Recipes can extract typed records from server-rendered HTML the same way they extract from JSON. No second mini-language; the same path-and-pipe grammar works against parsed DOM trees.

The model: any parseable content type becomes a queryable `JSONValue` tree. The `.node` variant wraps a parsed HTML/XML element (SwiftSoup on the Swift runtime, cheerio on the TS port). A handful of transforms — `parseHtml`, `parseJson`, `select`, `text`, `attr`, `html`, `innerHtml`, `first` — let recipes walk DOM trees, materialize text/attributes, and emit typed records, with no grammar change beyond extending for-loop collections from `PathExpr` to `ExtractionExpr` so iteration can drive off a pipeline result.

**Status: landed.** Documented here as the canonical reference for what M8 covers; the runtime, parser, validator, and TS port all carry the primitive today.

**What the primitive looks like (recipe-side):**

```forage
for $row in $page | parseHtml | select("table.opinions tr:has(a)") {
    emit Opinion {
        date     ← $row | select("td:nth-child(2)") | text
        caseName ← $row | select("td:nth-child(4) a") | text
        pdfUrl   ← $row | select("td:nth-child(4) a") | attr("href")
    }
}
```

**Deliverables (all landed):**

- **D8.1 — `JSONValue.node(HTMLNode)` variant.** A queryable element. Hashable by outerHTML; `@unchecked Sendable` (the underlying parser element is a reference type, but recipe evaluation treats nodes as immutable). Non-Codable in the round-trip sense — nodes encode as outerHTML strings and never decode back, so they're runtime-only.
- **D8.2 — HTML/DOM transforms.** `parseHtml` (string → node), `parseJson` (string → JSON), `select(selector)` (node → [node]), `text` / `attr` / `html` / `innerHtml` (node → string), `first` (array → first element). `text`/`attr`/`html`/`innerHtml` auto-flatten a single-element node array (jQuery convention) so `select(".x") | text` works without an explicit `| first`.
- **D8.3 — `for $x in <ExtractionExpr>`.** For-loop collections were `PathExpr` only; now accept the full extraction grammar so pipelines like `for $card in $page | parseHtml | select(".card")` drive iteration directly. Bare-path collections still parse cleanly. `CaptureRule.iterPath` extends the same way.
- **D8.4 — Content-type-aware response body decode.** `JSONValue.decodeBody(_:contentType:)` returns `.string(body)` for `text/html`, `text/xml`, etc. — non-JSON bodies don't throw the engine; recipes pipe through `parseHtml` to materialize the node.
- **D8.5 — Mirrored in `forage-ts`.** Cheerio dep, mirrored `.node` variant, mirrored transforms, mirrored for-loop grammar. Shared test vector `Tests/shared-recipes/07-html-extraction.forage` checks both implementations agree on parse + validate.
- **D8.6 — Tests.** `Tests/ForageTests/HTMLExtractionTests.swift` (14 unit + end-to-end tests on the Swift side); `hub-site/forage-ts/test/html-extraction.test.ts` (13 mirroring tests on the TS side); 7th shared recipe vector.
- **D8.7 — Reference recipes.** `recipes/hacker-news-html/` (HTML scrape of news.ycombinator.com — the "no API needed" companion to the JSON-API `recipes/hacker-news/`). `recipes/scotus-opinions/` (US Supreme Court slip opinions for a term — typed `Opinion` records extracted from supremecourt.gov's HTML table; a civic-data example with no API and no anti-bot).
- **D8.8 — Docs.** New `site/docs/html-extraction.md` page; transform reference in `site/docs/syntax.md` extended; engine-selection notes in `site/docs/engines.md` updated to describe content-type dispatch.

**Out of scope (intentional follow-ups):**

- Browser-engine recipes that need to extract from the *initial document body* (eBay search results, Cloudflare-gated sites). The browser engine today captures fetch/XHR responses; capturing the rendered document body after navigation is M9.
- HTML form submissions / multipart bodies (would compose with M7 sessions for login flows).
- XML-namespaced parsing (RSS / Atom with prefixed elements). `parseHtml` handles most loose XML already; tightening up Atom-specific access patterns is a follow-up.

---

## M9 — Browser-engine document capture

**Result:** Browser-engine recipes can extract from the rendered document body itself, not just from XHR/fetch responses. A real WebKit instance walks through Cloudflare-style JS challenges, and the post-navigation document body becomes a synthetic capture the recipe extracts from via M8's HTML primitives.

**Status: landed.**

**What the primitive looks like (recipe-side):**

```forage
recipe "letterboxd-popular" {
    engine browser

    type Film { title: String, url: String? }

    browser {
        initialURL: "https://letterboxd.com/films/popular/this/week/"
        observe:    "letterboxd.com"
        paginate browserPaginate.scroll {
            until: noProgressFor(2)
            maxIterations: 0
        }
        captures.document {
            for $poster in $ | select("div.poster.film-poster") {
                emit Film {
                    title ← $poster | select("span.frame-title") | text
                    url   ← $poster | select("a.frame") | attr("href")
                }
            }
        }
    }
}
```

The `$` inside `captures.document { … }` is the parsed root node of the post-settle document — recipes walk it with `select(...)` directly, no `parseHtml` call needed.

**Deliverables (all landed):**

- **D9.1 — `captures.document { … }` block.** Sibling to `captures.match`. Fires once after the browser has finished settling. The capture's body is `document.documentElement.outerHTML`; in the rule's scope `$` is pre-parsed as a node so recipes can `select` immediately.
- **D9.2 — Synthetic capture plumbing.** `BrowserEngine.captureDocumentBody` evaluates JS to fetch the outerHTML, wraps it as `Capture(kind: .document, …)`, appends it to the run's captures list (so it survives into archived `captures.jsonl`), and routes it through the document rule.
- **D9.3 — AST additions.** `Capture.Kind.document` variant. New `DocumentCaptureRule` value type. `BrowserConfig.documentCapture: DocumentCaptureRule?` field — one document rule per recipe (multiple XHR `captures.match` rules continue to coexist).
- **D9.4 — Iteration semantics.** Document rules take the same body shape as XHR rules: `for $x in <ExtractionExpr> { emit … }`, where `<ExtractionExpr>` typically uses M8 transforms (`select`, `text`, `attr`).
- **D9.5 — Replayer support.** `BrowserReplayer` routes `kind: .document` captures to the document rule (matching how live runs handle them). Archived runs round-trip cleanly.
- **D9.6 — Reference recipes.**
  - **`recipes/letterboxd-popular/`** — the flagship live demo. Letterboxd's "films popular this week" page is Cloudflare-gated (`curl` gets a 403); the browser engine drives a WKWebView through the gate, `captures.document` extracts ~70 typed `Film` records per run. End-to-end working.
  - **`recipes/ebay-sold/`** — kept as a shape reference. eBay's Akamai layer serves a CAPTCHA challenge to WKWebView, which our scraping policy (no bypassing technical controls) rules out solving. The recipe parses and validates; it documents what an eBay completed-listings recipe would look like, with a note about the CAPTCHA limitation.
- **D9.7 — Tests.** `Tests/ForageTests/DocumentCaptureTests.swift`: parser accepts `captures.document`, rejects duplicates, replayer routes a synthetic `.document` capture through the rule and emits expected records.
- **D9.8 — Docs.** `site/docs/html-extraction.md` extended with a browser-engine section that pairs M8 (the extraction primitive) with M9 (the document-capture source).

**Out of scope (intentional follow-ups):**

- **CAPTCHA-walled sites** (eBay's Akamai layer, Datadome on hot-ticket sites, sites that require interactive proof-of-humanity). Bypassing these violates our scraping policy. A *user-driven* "show me the page so I can solve the challenge" mode in the Toolkit is a possible future, but it lives outside the headless DSL.
- **Form submissions** in browser recipes (filling search boxes, posting filter forms). Today's recipes navigate via `initialURL`; multi-page flows through forms would need a new primitive.

**Acceptance**

- `forage run recipes/letterboxd-popular` returns ≈70 typed `Film` records with `title`, `url`, `posterUrl`.
- `captures.document` survives the `BrowserReplayer` round-trip (covered by `DocumentCaptureTests.browserReplayerRoutesDocumentCaptureToDocumentRule`).
