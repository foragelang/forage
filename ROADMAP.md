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

**Result:** Recipes can be browsed, edited, validated, and published from `hub.foragelang.com` without installing anything.

This is the most exotic engineering — it requires the parser+validator to run in the browser.

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

## Order of execution

Serial, top-to-bottom. Each milestone gets a `product-engineer` agent dispatch with the full milestone brief, followed by a `code-review-auditor` pass on the resulting diff. Findings from the auditor get a focused fixup pass before moving on.

Milestone 6 (web IDE) is gated on M5 only by docs convenience — it could land in parallel with M3/M4/M5 once M2 is up. But sequential is simpler to manage.

Once M1-M6 are all landed, the website / hub / CLI / toolkit story is end-to-end.
