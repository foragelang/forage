# Forage — Rust rewrite roadmap

Soup-to-nuts plan to port Forage from Swift to a Rust workspace. Forage Studio gets reimplemented in Tauri (Rust shell + React + Monaco) instead of SwiftUI; the rest of the stack — runtime, CLI, hub client, LSP, web-IDE wasm core — moves to Rust too. Cross-platform webviews via `wry`, `forage-wasm` replaces `forage-ts` in the web IDE. Greenfield: Swift code is archived in git history and deleted as each Rust counterpart lands.

Milestones prefixed `R1`–`R13` to distinguish from the Swift `M1`–`M11` (now history).

---

## The endgame

When the roadmap is fully landed:

- **One cargo workspace** rooted at the repo, with `~12` library crates and 2 binary apps.
- **`forage` CLI** installable via Homebrew, `curl | sh`, and `cargo install`. Runs every published canonical recipe identically to the Swift CLI.
- **Forage Studio (Tauri)** signed + notarized for macOS, published to `foragelang.com/download`. Embeds Monaco with a real Forage LSP (autocomplete, hover, diagnostics, go-to-definition, format).
- **Web IDE at hub.foragelang.com** running `forage-wasm` — full parser/validator/HTTP-runner parity with the native runtime, same source-of-truth Rust code compiled to WebAssembly. `forage-ts` is deleted.
- **api.foragelang.com / hub.foragelang.com** unchanged in shape (Cloudflare Worker stays TypeScript) but polished — structured error envelopes, OAuth fully wired, ownership-checked publish/delete, rate limiting, smoke tests covering every error path.
- **Cross-platform reach**: Windows (.exe / .msi via WebView2) + Linux (AppImage / .deb via WebKitGTK) landed after macOS-first ships.
- **Honest scraping posture preserved**: real WebKit on Mac (via `wry`), real WebView2 on Windows, real WebKitGTK on Linux — JS challenges still passable because the runtime is a real browser. CAPTCHAs still hand off to a human via M10 interactive bootstrap.

---

## Workspace shape

```
forage/
├── Cargo.toml                       # [workspace] members = [...]
├── rust-toolchain.toml              # stable, edition 2024
├── rustfmt.toml
├── clippy.toml
│
├── crates/
│   ├── forage-core/                 # AST, parser (chumsky), validator, evaluator,
│   │                                # snapshot, transforms, JSONValue
│   ├── forage-http/                 # HTTP engine, auth flavors, pagination, session cache
│   ├── forage-browser/              # wry-based browser engine, captures, M10 interactive
│   ├── forage-hub/                  # hub client, import resolver, recipe cache
│   ├── forage-keychain/             # keyring-based cross-platform secret storage
│   ├── forage-replay/               # HTTPReplayer + BrowserReplayer (shared fixture types)
│   ├── forage-lsp/                  # tower-lsp server, reuses forage-core
│   ├── forage-wasm/                 # wasm-bindgen exports for the web IDE
│   └── forage-test/                 # parity-fixture loader + bundled .forage vectors
│
├── apps/
│   ├── cli/                         # `forage` binary, clap-based
│   └── studio/                      # Forage Studio — Tauri app, React + Monaco
│       ├── src/                     # Tauri commands, state, LSP child-process orchestration
│       ├── ui/                      # React 19 + Vite + Tailwind v4 + shadcn/ui + Monaco
│       └── tauri.conf.json
│
├── hub-api/                         # STAYS TypeScript (Cloudflare Worker)
├── hub-site/                        # STAYS VitePress, depends on forage-wasm now
│
└── docs/                            # mdbook — language reference, hosted at foragelang.com/docs
```

Build tooling:
- **`chumsky` 1.x** — parser combinator with error recovery + spans.
- **`ariadne`** — colorful, span-aware diagnostic rendering for the CLI; serialized to LSP `Diagnostic` for editors.
- **`miette`** — alternative diagnostic crate; we'll pick one in R1, both are equivalent for our needs.
- **`reqwest`** + `tower` middleware (rate limit, retry, redirect policy).
- **`wry`** — cross-platform native webview, with `#[cfg(target_os)]` shims for cookie-store / message-handler / WKWebsiteDataStore-equivalent surfaces.
- **`tokio`** — async runtime.
- **`serde`** + `serde_json` — JSON I/O everywhere; AST has `#[derive(Serialize, Deserialize)]` for archives and IPC.
- **`clap`** — CLI argument parsing with derive macros.
- **`tower-lsp`** — LSP server framework.
- **`wasm-bindgen`** + `wasm-pack` — JS bindings for `forage-wasm`.
- **`keyring`** — cross-platform secret storage (macOS Keychain / Windows Credential Manager / Linux Secret Service).
- **`tauri`** 2.x — desktop shell.
- **`aes-gcm`** + `rand` + `argon2` (or `pbkdf2`) — session-cache encryption.

---

## R1 — Workspace scaffold + forage-core

**Result:** All shared-recipes test vectors from the Swift suite parse + validate identically in Rust, with per-character span tracking and `ariadne`-rendered errors.

**Deliverables:**

- **R1.1 — Workspace scaffold.** Root `Cargo.toml` with `[workspace]` member globs. `rust-toolchain.toml` pinned to stable. Workspace-level `Cargo.toml` declares shared deps (serde, tokio, anyhow, thiserror, tracing). `rustfmt.toml` + `clippy.toml` configured. `.github/workflows/ci.yml` runs `cargo test --workspace` on macOS-latest + ubuntu-latest + windows-latest.

- **R1.2 — `forage-core::ast`.** Full AST module mirroring Swift:
  - `Recipe { name, engine, types, enums, inputs, secrets, auth, browser?, http_steps, captures, expectations }`.
  - `TypeDef`, `EnumDef`, `InputDecl`, `SecretDecl`.
  - `AuthBlock` enum with `StaticHeader`, `HtmlPrime`, `SessionFormLogin`, `SessionOAuth`, `SessionCookieJar`, `SessionHtmlPrime` variants.
  - `BrowserConfig { initial_url, age_gate, dismissals, warmup_clicks, observe, pagination, captures, document_capture, interactive }`.
  - `Step { name, method, url, headers, body, extract, pagination }`.
  - `CaptureRule`, `DocumentRule`, `InteractiveConfig`.
  - `Expression` (typed enum: Literal, Path, Template, Pipe, Case, BinOp, Conditional).
  - `PathExpr` (Input, Secret, Variable, Field, Index, Wildcard).
  - `Transform` value type with name + args.
  - `Template` with literal segments + `{$expr}` interpolations.
  - All types `#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]`; spans carried as `Range<usize>` or `Span` struct.

- **R1.3 — `forage-core::lexer`.** Unified with the parser via chumsky's lexer-as-parser pattern, or separate token-stream lexer if cleaner. Keywords: every Swift Forage keyword (`recipe`, `engine`, `type`, `enum`, `input`, `secret`, `step`, `auth`, `browser`, `captures`, `paginate`, `emit`, `for`, `in`, `of`, `case`, `expect`, `observe`, `ageGate`, `dismissals`, `warmupClicks`, `interactive`, `bootstrapURL`, `cookieDomains`, `sessionExpiredPattern`, …). Operators: `←` (binding), `→` (case arm), `|` (pipe), `?` (optional), `[*]` (wildcard).

- **R1.4 — `forage-core::parser`.** Chumsky-based, returns `(Option<Recipe>, Vec<ParseError>)` — best-effort AST even on parse failure for the LSP. Full grammar: types, enums, inputs, secrets, auth blocks (all six variants), HTTP steps, browser config (initial URL, age gate, dismissals, warmup, observe, pagination, captures.match, captures.document, interactive), emit blocks, for loops, case expressions, transform pipelines, templates with interpolation.

- **R1.5 — `forage-core::validate`.** Semantic checker over the AST:
  - Every emit's record type matches a declared type.
  - Every input reference (`$input.X`) refers to a declared input.
  - Every secret reference (`$secret.X`) refers to a declared secret.
  - Every transform name resolves to a registered transform with matching arg-arity.
  - `case` arm coverage for enum match.
  - `paginate` strategy matches engine kind (HTTP vs browser).
  - Browser-only fields (age gate, dismissals, observe) rejected on HTTP engine.
  - Returns `Vec<ValidationIssue>` with severity + span + message.

- **R1.6 — `forage-core::eval`.** Pure-data evaluator:
  - `PathExpr::resolve(scope, path) -> Option<JSONValue>` with `$.x[*].y` wildcards.
  - `Template::render(scope) -> String`.
  - `Transform::apply(input, args, scope) -> Result<JSONValue>` over a registry of built-ins.
  - Registry of `~30` built-in transforms ported from Swift: `lowercase`, `uppercase`, `titleCase`, `dedup`, `toString`, `coalesce`, `length`, `prevalenceNormalize`, `parseSize`, `normalizeOzToGrams`, `normalizeUnitToGrams`, `sizeValue`, `sizeUnit`, `parseJaneWeight`, `janeWeightUnit`, `janeWeightKey`, `getField`, `regex.extract`, …. Each transform: `fn(input: &JSONValue, args: &[JSONValue], scope: &Scope) -> Result<JSONValue, TransformError>`. Registered via a `inventory`-style collect or explicit module function.

- **R1.7 — `forage-core::snapshot`.** Record collection (`HashMap<TypeName, Vec<Record>>` or `Vec<TypedRecord>`), expectation evaluator (`records.where(typeName == X).count >= N`), `DiagnosticReport { stall_reason, unmet_expectations, unfired_capture_rules, unmatched_captures, unhandled_affordances }`.

- **R1.8 — `forage-core::error`.** Unified error types using `thiserror`; `ParseError`, `ValidationIssue`, `EvalError` carry source spans. `ariadne::Report` rendering. Error envelope serializes to LSP `Diagnostic`.

- **R1.9 — Test vectors.** `crates/forage-test/fixtures/*.forage` loaded by the `forage-test` crate. Each vector has `expected.json` with golden parse output. `cargo test -p forage-core` parses every vector + asserts AST shape + validates + checks expected diagnostics.

- **R1.10 — CI green.** Workflow `rust-ci.yml` runs `cargo build --workspace`, `cargo test --workspace`, `cargo clippy --workspace -- -D warnings`, `cargo fmt --check`. Cache `~/.cargo` + `target/`.

**Acceptance:**
- `cargo test -p forage-core` is green with the full shared-recipes suite parsed and validated.
- Parse + validate on a malformed recipe surfaces the same error position the Swift compiler does, rendered by `ariadne`.
- AST `JSONValue` serialization round-trips through serde.

---

## R2 — forage-http

**Result:** Every canonical HTTP-engine recipe runs end-to-end against the live network with identical record output to the Swift `HTTPEngine`. Session caching, auth flavors, pagination, retry, rate limit all work.

**Deliverables:**

- **R2.1 — `forage-http::client`.** `HTTPClient` over `reqwest::Client` with a `tower` middleware stack:
  - **Rate limit** — token bucket, default 1 req/sec per host, configurable per-recipe.
  - **Retry** — exponential backoff with jitter on 429 / 5xx; honors `Retry-After`. Max retries from recipe config (default 3).
  - **Redirect policy** — follow same-origin, configurable.
  - **Cookie jar** — `reqwest_cookie_store` shared across steps within a recipe run.
  - **Honest UA** — `Forage/x.y.z (+https://foragelang.com)` default; recipe can override.

- **R2.2 — `forage-http::auth`.** All six auth flavors:
  - `StaticHeader` — inject `{name}: {value}` on every request after auth.
  - `HtmlPrime` — one-shot GET, regex-extract named groups into scope variables, optional cookie persistence; subsequent steps reference `{$varName}`.
  - `SessionFormLogin` — POST form credentials, extract success token / cookie, attach to subsequent requests; MFA hook on `requiresMFA`.
  - `SessionOAuth` — `client_credentials` and `authorization_code` flows; token refresh on 401; tokens cached.
  - `SessionCookieJar` — login form fills + cookie persistence; expiry detection via response shape.
  - `SessionHtmlPrime` — `HtmlPrime` + session caching of extracted variables.

- **R2.3 — `forage-http::mfa`.** `trait MFAProvider: Send + Sync` with `async fn code(&self) -> Result<String, MFAError>`. `StdinMFAProvider` for CLI; `ChannelMFAProvider` for Studio (sends a request to the Tauri frontend, awaits user input via async channel).

- **R2.4 — `forage-http::paginate`.** All strategies:
  - `cursor { items, next_cursor, cursor_param }`
  - `page { items, page_param, page_size }`
  - `pageWithTotal { items, total, page_param, page_size }`
  - `untilEmpty { items, page_param }`
  - `noPagination` (single request — degenerate case).
  Each strategy: `async fn next_page(&mut self, prev_response) -> Option<NextRequest>`.

- **R2.5 — `forage-http::session_cache`.** Persisted to `~/Library/Forage/Cache/sessions/<recipe-slug>/<fingerprint>.json` on Mac, XDG cache dir on Linux, `%LOCALAPPDATA%` on Windows. Sanitized slug, `chmod 600` (or platform equivalent), optional AES-GCM encryption keyed via `forage-keychain` (Mac/Windows/Linux secret store). Fingerprint = SHA-256 over resolved secrets. Eviction on 401/403.

- **R2.6 — `forage-http::engine`.** The HTTP engine itself: walk the recipe's step graph, resolve templates, drive pagination, accumulate captures, evaluate emit blocks, build the snapshot. Same diagnostic envelope as Swift.

- **R2.7 — `forage-http::replay`.** `HTTPReplayer` transport — reads `fixtures/captures.jsonl`, matches URL + method, returns the stored body. Swappable for live `reqwest::Client` in tests.

- **R2.8 — Tests.** Every canonical HTTP recipe (hacker-news, github-releases, nasa-apod, usgs-earthquakes, onthisday, scotus-opinions, hacker-news-html) has a replay test: load fixtures, run, diff snapshot against `expected.snapshot.json`. Tests run in `cargo test -p forage-http`.

**Acceptance:**
- `cargo test -p forage-http` runs every HTTP recipe via replay and matches expected snapshots byte-for-byte (after canonicalization).
- Live: `cargo run -p forage-cli -- run hacker-news` (from the workspace dir) returns the same record types + counts as the Swift CLI.

---

## R3 — `forage` CLI (HTTP-only first cut)

**Result:** `forage` binary that runs HTTP recipes end-to-end with `ariadne`-rendered diagnostics. `forage test`, `forage scaffold`, `forage publish` (stubbed) wired up.

**Deliverables:**

- **R3.1 — `apps/cli` crate.** clap-derived subcommand structure: `run`, `test`, `capture` (stub), `scaffold` (stub), `publish` (stub), `auth` (stub), `lsp` (stub). Global flags: `--verbose`, `--output {pretty|json}`, `--color {auto|always|never}`.

- **R3.2 — `forage run`.** Loads `<recipe-dir>/recipe.forage`, parses, validates, reads `fixtures/inputs.json`, executes via `forage-http::engine` (or `forage-browser` later), prints the snapshot. `--replay` flag uses `HTTPReplayer` against `fixtures/captures.jsonl`. `--output json` for machine-readable.

- **R3.3 — `forage test`.** Runs recipe via replay, diffs snapshot against `expected.snapshot.json` (recipe directory), exits non-zero on diff. `--update` overwrites the expected file (golden-file workflow). Pretty diff via the `similar` crate.

- **R3.4 — `forage scaffold` (deferred to R5).** Just print "not yet" — fleshed out with the browser engine.

- **R3.5 — Diagnostics.** `ariadne::Report::eprint(sources)` for parse + validate errors; runtime errors include the recipe span where they originated. Exit codes: 0 ok, 1 runtime, 2 parse/validate, 3 expectation unmet.

- **R3.6 — Output formatting.** Pretty mode: per-type record counts, sample of first 3 records, diagnostic summary. JSON mode: full snapshot as a single JSON document.

- **R3.7 — Logs.** `tracing` + `tracing-subscriber` with `--verbose` enabling `forage=debug`. Default level `forage=info`.

**Acceptance:**
- `forage run hacker-news --replay` (from the workspace dir) prints a clean snapshot matching the Swift CLI output.
- `forage test hacker-news` passes against the recipe's expected snapshot.
- A malformed recipe surfaces an `ariadne`-rendered error pointing at the broken span.

---

## R4 — forage-browser (wry, captures, M10 interactive)

**Result:** Every canonical browser-engine recipe (`letterboxd-popular`, `ebay-sold`, `trilogy-rec`/`med`) runs identically to Swift. M10 interactive bootstrap works on macOS (visible WKWebView, overlay button, session persistence, headless reuse with `sessionExpiredPattern` detection).

**Deliverables:**

- **R4.1 — `forage-browser::webview`.** Thin wrapper over `wry::WebView`:
  - Construct a webview (visible or headless) at a given URL.
  - Script injection at document-start (the fetch/XHR interception shim) and document-end (custom recipe scripts).
  - Two-way IPC: host → JS via `evaluate_script`; JS → host via `wry`'s `with_ipc_handler`.
  - Per-platform shims (`#[cfg(target_os)]`):
    - macOS: extract cookies via `WKWebsiteDataStore.httpCookieStore` (Cocoa FFI).
    - Linux: extract cookies via `WebKitCookieManager` (gtk-rs).
    - Windows: extract cookies via `CoreWebView2.CookieManager` (webview2-com).
  - Settle: detect "no new fetch/XHR for N seconds" using the injected shim's bookkeeping.

- **R4.2 — `forage-browser::inject`.** JS shim source files (`.js`) embedded via `include_str!`:
  - **`fetch_intercept.js`** — patches `window.fetch` + `XMLHttpRequest.prototype.send`; on response, posts a message: `{ kind: "capture", url, status, headers, body }`. Shim runs at document-start so it patches before the page loads.
  - **`interactive_overlay.js`** — injects a fixed-position green "✓ Scrape this page" button; on click, posts `{ kind: "interactiveDone", url, html }` to the host.
  - **`dump_localstorage.js`** — returns `localStorage` as an object via `evaluate_script`.
  - **`restore_localstorage.js`** — accepts an object, writes each key.

- **R4.3 — `forage-browser::settle`.** Network-idle detector: track in-flight fetch/XHR counts via shim messages, mark "settled" after `idle_for` seconds with no activity. Recipe-configurable: `paginate { until: noProgressFor(2) }`. Hard cap via `maxIterations` and `iterationDelay`.

- **R4.4 — `forage-browser::paginate`.** Browser pagination strategies:
  - `browserPaginate.scroll { until, maxIterations, iterationDelay }` — `window.scrollTo(0, document.body.scrollHeight)` then wait for settle; repeat.
  - `browserPaginate.button { selector, until, maxIterations }` — click selector until disabled or absent.
  - `browserPaginate.numbered { container, until }` — click page-number links sequentially.
  - `browserPaginate.url { from, to, param }` — modify URL param, navigate.

- **R4.5 — `forage-browser::captures`.** Capture rule routing:
  - `captures.match { urlPattern, body extract }` — fired per matching intercepted response; body parsed as JSON (configurable) and the recipe's extract block runs against it.
  - `captures.document { extract }` — fired once after settle; webview's `document.documentElement.outerHTML` parsed via `scraper` (or `kuchiki`) and the extract runs against the DOM.

- **R4.6 — `forage-browser::age_gate`.** Selector-based form fill: locate `input[name=...]` for year/month/day, dispatch input events, click submit. Recipe schema unchanged from Swift.

- **R4.7 — `forage-browser::dismissals`.** List of selectors to click on page load (cookie banners, "I'm 21" buttons, etc.). Click; if element is missing, skip.

- **R4.8 — `forage-browser::interactive` (M10).** Full interactive-bootstrap flow:
  - **Bootstrap mode** (`--interactive`, or programmatic `InteractiveBootstrapMode::ForceBootstrap`):
    1. Open visible webview at `bootstrapURL` (defaults to `initialURL`).
    2. Disable settle timer — wait for human, not idle.
    3. Inject `interactive_overlay.js` after `did_finish_navigation`.
    4. On `interactiveDone` message, snapshot cookies (filter by `cookieDomains`) + `localStorage` (per-origin), serialize as `InteractiveSession`, write to `~/Library/Forage/Sessions/<slug>/session.json` (chmod 600).
  - **Reuse mode** (`Auto` mode with cached session):
    1. Seed the webview's cookie store + per-origin localStorage from the cached session.
    2. Navigate to `initialURL` headlessly.
    3. After navigation, read `document.documentElement.outerHTML`; if it contains `sessionExpiredPattern`, evict the cache and surface `stallReason: "session-expired: re-run with --interactive to refresh"`.
    4. Otherwise proceed with normal pagination + captures.

- **R4.9 — `forage-browser::session_store`.** `InteractiveSession { recipe_slug, bootstrapped_at, expires_at, cookies, local_storage }` Codable JSON; path resolution + chmod 600 write/read/evict; slug sanitization.

- **R4.10 — `forage-browser::engine`.** Top-level driver: build webview, apply config, run captures + pagination, collect snapshot, return `DiagnosticReport`. Same shape as `forage-http::engine`.

- **R4.11 — `forage-browser::replay`.** `BrowserReplayer` reads `fixtures/captures.jsonl` with kind discriminator (`.match` or `.document`), routes each capture through the recipe's rules without spawning a webview.

- **R4.12 — Tests.** Browser-engine recipes replayed:
  - `letterboxd-popular` → ~70 `Film` records.
  - `ebay-sold` → typed `SoldListing` records (under stored interactive session).
  - `trilogy-rec` / `trilogy-med` → `Product` + `Variant` + `PriceObservation` records.

**Acceptance:**
- `forage run letterboxd-popular` (from the workspace dir) returns ~70 Films on Mac, end-to-end live.
- `forage run --interactive ebay-sold --input query=polaroid+sx-70` opens a visible window, accepts the human-solved Akamai challenge, persists the session, returns ~50 sold listings; second run (without `--interactive`) runs headless.
- `forage run trilogy-rec --replay` matches snapshot.

---

## R5 — CLI complete (browser support + capture + scaffold)

**Result:** CLI feature-parity with the Swift CLI. `forage capture` + `forage scaffold` operational.

**Deliverables:**

- **R5.1 — `forage run` browser support.** Engine selected from `recipe.engine`. `--interactive` flag works.

- **R5.2 — `forage capture <url>`.** Opens a visible webview, intercepts fetch/XHR, writes each to `<output-dir>/captures.jsonl`. Flags: `--output`, `--wait <seconds>`, `--user-agent`. Useful for recipe authoring before any recipe exists.

- **R5.3 — `forage scaffold <captures.jsonl>`.** Reads captures, generates a starter `.forage` recipe with one `captures.match` per unique URL pattern, placeholder `Item` type, placeholder extract block. Same shape as Swift `forage scaffold`.

- **R5.4 — Swift CLI retired.** `Sources/forage-cli/` deleted. README + install instructions point at the Rust CLI. Git history is the archive.

**Acceptance:**
- `forage capture https://news.ycombinator.com/ --output /tmp/hn` records the page's fetches.
- `forage scaffold /tmp/hn/captures.jsonl > recipe.forage` produces a valid starter recipe that parses + validates.
- Swift CLI directory is gone.

---

## R6 — forage-hub + forage-keychain + auth

**Result:** Hub publish / fetch / import + OAuth device-code login work end-to-end against `api.foragelang.com`. Secrets land in the OS-native secret store on all three platforms.

**Deliverables:**

- **R6.1 — `forage-keychain` crate.** Thin wrapper over `keyring` crate:
  - `read_secret(service, account) -> Result<Option<String>>`
  - `write_secret(service, account, value) -> Result<()>`
  - `delete_secret(service, account) -> Result<()>`
  - Service identifiers: `com.foragelang.cli`, `com.foragelang.studio`.

- **R6.2 — `forage-hub::client`.** `HubClient` over `reqwest`:
  - `list(query) -> Vec<RecipeMeta>`
  - `get(slug, version?) -> RecipeBlob`
  - `publish(slug, body, metadata) -> PublishedRecipe`
  - `delete(slug)`
  - Bearer-token auth via `FORAGE_HUB_TOKEN` env or auth-store JWT.

- **R6.3 — `forage-hub::importer`.** Resolves `import hub://<author>/<slug>` directives recursively. Caches at `~/Library/Forage/Cache/hub/<author>/<slug>/<version>/recipe.forage`. Unions imported types/enums/inputs into the consuming recipe's catalog. Detects + reports cycles.

- **R6.4 — `forage-hub::auth_store`.** Persistent auth store at `~/Library/Forage/Auth/<host>.json` (chmod 600). Schema: `{ access_token, refresh_token, login, hub_url, issued_at, expires_at }`. Refresh on access-token expiry.

- **R6.5 — `forage auth login`.** Device-code flow:
  1. POST `/v1/oauth/device` → `{ user_code, verification_url, device_code, interval, expires_in }`.
  2. Print user code; open verification URL in default browser via `webbrowser` crate.
  3. Poll `/v1/oauth/device/poll` every `interval` seconds until 200 + tokens.
  4. Write tokens to auth store.

- **R6.6 — `forage auth logout [--revoke]`.** Deletes auth-store file. `--revoke` POSTs to `/v1/oauth/revoke` first to invalidate server-side refresh.

- **R6.7 — `forage auth whoami`.** Reads auth store, prints `<login>@<hub-host>` or "not signed in."

- **R6.8 — `forage publish <recipe-dir>`.** Validates locally, builds payload (every `.forage` file in the workspace + metadata + optional fixtures snapshot), POSTs to `/v1/packages`. `FORAGE_HUB_TOKEN` env wins over auth-store JWT. `--dry-run` (default) prints would-send; `--publish` actually POSTs.

- **R6.9 — Hub fetch in `forage run`.** When a recipe has `import hub://...` directives, `RecipeImporter` resolves them transparently before running.

- **R6.10 — Tests.** Mock hub via `wiremock` crate for the publish + auth flows; live hub smoke tests gated on `FORAGE_HUB_LIVE_TEST=1`.

**Acceptance:**
- `forage auth login` against api.foragelang.com → tokens stored in OS keychain on each platform.
- `forage publish hacker-news` (from the workspace dir) → recipe appears on hub.foragelang.com.
- A recipe with `import hub://foragelang/zen-leaf-elkridge` runs end-to-end with the imported types unioned.

---

## R7 — forage-lsp

**Result:** Standalone LSP server binary that VS Code (and Monaco, in R8) talks to over stdio or WebSocket. Autocomplete, hover, diagnostics, go-to-definition, document outline, format all work.

**Deliverables:**

- **R7.1 — `forage-lsp` crate.** `tower-lsp` server. `forage::Lsp` struct implements `LanguageServer` trait.

- **R7.2 — Document store.** Maps URI → source text + parsed AST + diagnostics. Re-parses on `didChange` (or `didSave` if perf demands). Holds the canonical state per open document.

- **R7.3 — Diagnostics.** On every reparse, run `forage-core::parse` + `validate` → serialize to LSP `Diagnostic` (severity, range, message, code) → `publish_diagnostics`. Chumsky's error recovery means partial ASTs still publish whatever can be checked.

- **R7.4 — Completion.** `completion`:
  - Top-level: `recipe`, `import`, comments.
  - Inside `recipe { … }`: `engine`, `type`, `enum`, `input`, `secret`, `auth`, `browser`, `step`, `captures`, `paginate`, `emit`, `expect`.
  - Type position: previously declared type names + primitives (`String`, `Int`, `Double`, `Bool`, `[T]`, `T?`).
  - Inside `step { … }`: `method`, `url`, `headers`, `body.json`, `body.form`, `extract`, `paginate`.
  - Expression position: `$input.<X>` for each declared input (with type hint), `$secret.<X>`, `$<step-name>` for prior step outputs, transform names (`| <transform>`).
  - Transform names: every registered transform from `forage-core::eval::transforms`.

- **R7.5 — Hover.** `hover`:
  - On a type name: show type definition.
  - On an input: show input declaration + type.
  - On a transform: show transform's docstring + signature.
  - On a keyword: short doc explaining the construct.

- **R7.6 — Go-to-definition.** `definition`:
  - From a type reference → the `type X { … }` declaration.
  - From `$input.X` → the `input X: T` declaration.
  - From `$<step>` → the `step <step> { … }` block.
  - From an imported recipe ref → the imported file (resolved through `forage-hub::importer`).

- **R7.7 — Document symbol.** `documentSymbol` returns an outline: types, enums, inputs, secrets, steps, capture rules, expectations.

- **R7.8 — Formatting.** `formatting` runs a canonical Forage formatter (separate function in `forage-core::format`, or in `forage-lsp::format`): consistent indentation (4 spaces), `←` spacing, pipe alignment.

- **R7.9 — `forage lsp` CLI subcommand.** Spawns the server on stdio (default) or WebSocket (`--port 8080`). Studio uses stdio (child process); the web IDE uses WebSocket via a Cloudflare Worker bridge (or compiles the LSP itself to wasm — see R8).

- **R7.10 — VS Code extension stub.** `editors/vscode-forage/` — minimal extension that registers the `.forage` language, launches `forage lsp` over stdio, supplies syntax highlighting via TextMate grammar. Published as `foragelang.forage` in the marketplace later (R12).

**Acceptance:**
- Open a `.forage` file in VS Code with the extension installed: errors squiggled, completion works, hover shows type info, ⌘-click jumps to declarations.
- LSP server handles 1000+ document changes per minute without leaking memory.

---

## R8 — forage-wasm + hub-site swap

**Result:** `forage-ts/` is deleted. The hub site's Monaco-backed web IDE runs Forage's actual Rust parser/validator/HTTP-runner via WebAssembly, with the LSP also running in a Web Worker.

**Deliverables:**

- **R8.1 — `forage-wasm` crate.** `wasm-bindgen` exports:
  - `parseRecipe(source: &str) -> JsValue` — returns AST JSON or error.
  - `validate(recipe_json: &str) -> JsValue` — returns diagnostics array.
  - `runHTTP(recipe_json: &str, inputs: JsValue, fixtures_jsonl: &str) -> JsValue` — replay-only (no live fetch — browsers' CORS prevents that), returns snapshot.
  - `formatRecipe(source: &str) -> String` — canonical format.
  - `lspHandle()` — start an in-Worker LSP session, returns a message-port handle.

- **R8.2 — Build pipeline.** `wasm-pack build crates/forage-wasm --target web --out-dir hub-site/forage-wasm/pkg`. Generated `.wasm` + TypeScript bindings consumed by hub-site.

- **R8.3 — hub-site/.vitepress refactor.** `RecipeIDE.vue` imports `parseRecipe`, `validate`, `runHTTP` from `forage-wasm/pkg/`. `forage-ts/` directory deleted entirely. `package.json` removes `forage-ts` dep.

- **R8.4 — LSP in Web Worker.** A dedicated `Worker` script loads `forage-wasm`, calls `lspHandle()`, and bridges Monaco's `monaco-languageclient` to the in-Worker LSP via the message port. Studio uses the standalone `forage lsp` binary over stdio; the web IDE uses the wasm + Worker variant. Same `forage-core` underneath both.

- **R8.5 — Monaco wiring.** `monaco-editor` + `monaco-languageclient` configured for the `.forage` language. Workers configured via `MonacoEnvironment.getWorkerUrl` (TextMate / JSON / typescript-worker etc. need URL config under VitePress's base path).

- **R8.6 — Syntax highlighting.** TextMate grammar in `editors/vscode-forage/syntaxes/forage.tmLanguage.json` (shared with the VS Code extension), loaded into Monaco via `monaco-textmate`.

- **R8.7 — Tests.** Vitest tests for the web-IDE flows (parse a recipe, validate, run replay) — using `forage-wasm` directly from Node.

**Acceptance:**
- Visiting hub.foragelang.com/edit shows a Monaco editor with full Forage syntax highlighting + diagnostics + completion + hover.
- The "Run replay" button runs HTTP recipes against fixtures entirely in the browser.
- `forage-ts/` is gone.

---

## R9 — Forage Studio (Tauri rewrite)

**Result:** `Forage Studio.app` on macOS, built from `apps/studio/`, with feature parity to the SwiftUI Studio (Recipe library, editor, capture, run, fixtures, snapshot, diagnostic, publish, MFA, OAuth, M10 interactive). Embeds Monaco + the Forage LSP.

**Deliverables:**

- **R9.1 — `apps/studio/` scaffold.** `cargo tauri init`; Tauri 2.x. `tauri.conf.json`: bundle id `com.foragelang.Studio`, display name "Forage Studio", URL scheme `forage-studio://`, min window size 1100×700.

- **R9.2 — Tauri commands.** Rust `#[tauri::command]` fns invoked from the frontend:
  - `list_recipes() -> Vec<RecipeEntry>`
  - `load_recipe(slug) -> RecipeContent`
  - `save_recipe(slug, source)`
  - `create_recipe() -> String` (returns new slug)
  - `delete_recipe(slug)`
  - `run_recipe(slug, mode: "live" | "replay" | "interactive") -> RunResult`
  - `capture_url(url, output_dir) -> CapturesPath`
  - `publish_recipe(slug, metadata) -> PublishResult`
  - `auth_start_device_flow() -> DeviceCodeResp`
  - `auth_poll_device(device_code) -> AuthStatus`
  - `auth_logout(revoke: bool)`
  - `auth_whoami() -> Option<String>`
  - `get_preferences() -> Preferences`
  - `set_preferences(prefs)`
  - `mfa_provide_code(request_id, code)` (used by the MFA channel provider)

- **R9.3 — LSP child process.** Studio spawns `forage lsp` as a child process on startup. Tauri command `lsp_request(payload)` forwards JSON-RPC over the child's stdio. The frontend's `monaco-languageclient` talks to a custom WebSocket-shaped transport that proxies through Tauri commands.

- **R9.4 — Browser engine in Tauri.** The same `forage-browser` crate drives live runs + captures. A separate Tauri `WebviewWindow` is opened for each run (visible for interactive bootstrap + capture, headless for normal live runs). Tauri 2.x supports multiple windows natively.

- **R9.5 — Frontend stack.** React 19 + Vite + TypeScript + Tailwind v4 + shadcn/ui + `monaco-editor-react` + `monaco-languageclient`. TanStack Query for state derived from Tauri commands; Zustand for cross-component state (active recipe slug, dirty flag). Component tree:
  - `App.tsx` — root, two-pane resizable split.
  - `Sidebar.tsx` — recipe library list, new/import buttons.
  - `Editor.tsx` — Monaco-hosted recipe editor with LSP wired up; tabs across the top: Source, Fixtures, Snapshot, Diagnostic, Publish.
  - `SourceTab.tsx` — Monaco + validate panel below.
  - `FixturesTab.tsx` — list of fixture files with previews.
  - `SnapshotTab.tsx` — per-type record tables.
  - `DiagnosticTab.tsx` — stall reason + unmet expectations + unmatched captures + unhandled affordances.
  - `PublishTab.tsx` — publish form + Validate / Preview / Publish buttons.
  - `CaptureSheet.tsx` — modal sheet with a `<webview>`-equivalent (Tauri webview window) and a capture list panel.
  - `MFAPrompt.tsx` — modal sheet with a SecureField for one-time code.
  - `Preferences.tsx` — Cmd-, settings; hub URL, account, API-key fallback.

- **R9.6 — Native menus.** Tauri menu API: File (New `⌘N`, Save `⌘S`, Open Folder, Quit), Recipe (Run Live `⌘R`, Run Replay `⇧⌘R`, Capture `⌘K`, Validate `⇧⌘V`, Publish `⇧⌘P`, Import from Hub `⇧⌘I`), Edit (Cut/Copy/Paste/Undo/Redo from system), View (full-screen, dev tools).

- **R9.7 — IPC for MFA.** When the runtime hits an `auth.session` step with `requiresMFA`, it calls the `ChannelMFAProvider`, which sends a Tauri event to the frontend; `MFAPrompt.tsx` opens; user submits; `mfa_provide_code` Tauri command resumes the runtime.

- **R9.8 — Recipe library.** Reads `~/Library/Forage/Recipes/` (XDG / `%APPDATA%` on other platforms). New recipe creates an `untitled-N` dir with a minimal template + `fixtures/` dir.

- **R9.9 — URL-scheme handler.** `forage-studio://recipe/<slug>` deep links route to the import flow (download from hub, save under `~/Library/Forage/Recipes/`).

- **R9.10 — App icon + assets.** Icon slices from `site/public/favicon.svg` (ten sizes), shipped under `apps/studio/icons/`.

- **R9.11 — Build & sign.** `cargo tauri build` produces `Forage Studio.app`. macOS signing + notarization via Apple Developer ID (release workflow). DMG via `create-dmg`.

- **R9.12 — SwiftUI Studio retired.** `Studio/` directory deleted. `open-studio.sh` updated to `cargo tauri dev` (or similar). README updated.

**Acceptance:**
- Double-click `Forage Studio.app`; window opens; sidebar lists `untitled-1` (or pre-existing recipes); editor shows Monaco with full LSP behavior; `⌘R` runs the recipe; sheet flows work; Sign-in-with-GitHub completes the device-code flow.
- SwiftUI directory is gone.

---

## R10 — hub-api polish (errors, edge cases, OAuth activation)

**Result:** `api.foragelang.com` has clean structured error envelopes for every endpoint, rate limiting that actually throttles, OAuth fully activated (GitHub OAuth App credentials wired), and a smoke-test suite that hits every error path. The hub stays TypeScript.

**Deliverables:**

- **R10.1 — Error envelope.** Standardize on `{ error: { code, message, details?, retryAfter? } }` for all 4xx/5xx responses. Codes: `PARSE_ERROR`, `VALIDATION_ERROR`, `AUTH_REQUIRED`, `INVALID_TOKEN`, `FORBIDDEN`, `NOT_FOUND`, `CONFLICT`, `RATE_LIMITED`, `INTERNAL`. Hub client (Rust + TS) reads `code` for branching.

- **R10.2 — OAuth activation.** Register the GitHub OAuth App at `github.com/settings/developers` (manual user step). Set Worker secrets `GITHUB_OAUTH_CLIENT_ID`, `GITHUB_OAUTH_CLIENT_SECRET`, `JWT_SIGNING_KEY` via `wrangler secret put`. Verify the six flows (start, callback, device, device/poll, refresh, revoke) end-to-end with `hub-api/test/oauth-smoke.sh`.

- **R10.3 — Rate limiting.** Per-user + per-IP token-bucket via Cloudflare's Durable Objects (or KV-backed counters). Configurable per-endpoint. 429 with `Retry-After`.

- **R10.4 — Request size limits.** Cap recipe body at 1 MiB, fixtures at 16 MiB. Reject larger uploads with 413 + clear `code: PAYLOAD_TOO_LARGE`.

- **R10.5 — CORS hardening.** Allowlist: `https://hub.foragelang.com`, `https://foragelang.com`, `http://localhost:5173` (dev). Reject others. Preflight handling baked in.

- **R10.6 — Smoke tests.** `hub-api/test/smoke.sh` covers: list, get, publish (admin + user), delete (owner + non-owner = forbidden), OAuth device + web flows, refresh, revoke, rate-limit trigger, 404 paths, malformed JSON, unauthenticated calls. Tests run against a deployed staging Worker.

- **R10.7 — Observability.** Wrangler tail wired up; structured logs (JSON) for every request with `{ timestamp, path, status, user?, took_ms, error_code? }`. Cloudflare Analytics enabled.

- **R10.8 — Ownership migration.** Existing legacy recipes (published with `HUB_PUBLISH_TOKEN`) get `ownerLogin: "admin"`. New OAuth publishes stamp `ownerLogin: <gh-login>`. `callerCanWrite` enforces ownership on PUT/DELETE.

- **R10.9 — Hub docs.** `site/docs/hub.md` updated with full endpoint reference + error catalog + OAuth flow diagrams.

**Acceptance:**
- Every error response matches the envelope.
- Sign in with GitHub from Studio Preferences → publish a recipe → recipe appears under your login on hub.foragelang.com.
- Hammering `/v1/packages` from one IP triggers 429 with `Retry-After`.
- All smoke tests green.

---

## R11 — Release pipeline (macOS-first)

**Result:** Tagging `v0.1.0` triggers a CI workflow that builds `forage` CLI for macOS arm64+x86_64 and `Forage Studio.app` (signed + notarized), publishes a GitHub Release with both artifacts, updates the Homebrew tap, and refreshes `foragelang.com/download`.

**Deliverables:**

- **R11.1 — `.github/workflows/release.yml` rewrite.** Rust-based:
  - Trigger on tag `v*`.
  - Job `build-cli`: matrix over `(macos-15, aarch64-apple-darwin)` + `(macos-15, x86_64-apple-darwin)`. `cargo build -p forage-cli --release --target <triple>`. Strip + tar.gz. SHA-256.
  - Job `build-studio`: `macos-15`. `cargo tauri build --target universal-apple-darwin`. Output: `.app` + `.dmg` (via `create-dmg`). Codesign with Developer ID, notarize via `xcrun notarytool`, staple.
  - Job `release`: assemble GitHub Release, attach CLI tarballs + Studio DMG + SHA-256 files. Release notes from CHANGELOG.
  - Job `update-homebrew-tap`: gated on `ENABLE_HOMEBREW_TAP_UPDATE=1`. Bumps `Formula/forage.rb` in `foragelang/homebrew-tap` via fine-grained PAT.

- **R11.2 — `site/public/install.sh`.** Detects macOS arm64/x86_64, fetches latest release asset from GitHub API, verifies SHA-256, installs to `~/.local/bin/forage`, prints PATH hint. Hosted at `https://foragelang.com/install.sh`.

- **R11.3 — Homebrew formula.** `foragelang/homebrew-tap`, `Formula/forage.rb`. References latest release tarball + sha. `brew install foragelang/forage/forage`.

- **R11.4 — Download page.** `site/docs/install.md` (and the `/download` route) lists: Homebrew, curl|sh, Studio DMG direct link, build-from-source.

- **R11.5 — Codesign secrets.** Workflow uses `APPLE_DEVELOPER_ID_CERT`, `APPLE_DEVELOPER_ID_PASSWORD`, `APPLE_API_KEY_ID`, `APPLE_API_KEY_ISSUER_ID`, `APPLE_API_KEY` repo secrets. If any are missing, build still succeeds but artifacts are ad-hoc signed and the workflow flags it in the release notes.

- **R11.6 — Versioning.** Workspace-level `[workspace.package] version = "0.1.0"` propagates to all crates via `version.workspace = true`. Tag matches.

**Acceptance:**
- `git tag v0.1.0 && git push origin v0.1.0` → 10 minutes later, GitHub Release with signed Mac DMG + CLI tarballs + Homebrew formula bumped.
- `brew install foragelang/forage/forage && forage --version` prints `0.1.0`.
- `curl -fsSL https://foragelang.com/install.sh | sh` installs the CLI to `~/.local/bin/forage`.

---

## R12 — Cross-platform: Windows + Linux

**Result:** Studio + CLI ship for Windows (WebView2) and Linux (WebKitGTK). Recipes that work on macOS work on at least one of the other two; gaps are documented.

**Deliverables:**

- **R12.1 — CLI cross-compile.** Release workflow matrix expanded:
  - macOS: arm64 + x86_64 (already).
  - Linux: x86_64-unknown-linux-gnu + aarch64-unknown-linux-gnu.
  - Windows: x86_64-pc-windows-msvc.

- **R12.2 — Studio Windows.** `cargo tauri build` on `windows-latest`. WebView2 runtime detection / installer prompt. MSI package via WiX bundler.

- **R12.3 — Studio Linux.** `cargo tauri build` on `ubuntu-22.04`. AppImage + .deb. Flathub manifest stretch goal.

- **R12.4 — Browser-engine portability.** Per-platform shims for cookie extraction etc. (R4.1) actually exercised. Test all recipes on each platform; document fingerprint variances.

- **R12.5 — Distribution channels.** Chocolatey package for Windows, Snap or Flatpak for Linux. Stretch.

- **R12.6 — VS Code extension publish.** `editors/vscode-forage/` published to the marketplace as `foragelang.forage`.

**Acceptance:**
- `forage` CLI on Win/Linux runs every HTTP recipe; runs browser recipes that don't depend on platform-specific webview quirks.
- Studio launches on Win + Linux; can author + run recipes.
- Cross-platform parity issues documented in `docs/cross-platform.md`.

---

## R13 — Docs site + language reference

**Result:** `foragelang.com/docs` hosts an mdbook with the full Forage language reference, runtime guide, recipe cookbook, and contribution docs.

**Deliverables:**

- **R13.1 — `docs/` mdbook scaffold.** `book.toml` + chapter structure.
- **R13.2 — Language reference.** Every keyword, every transform (auto-generated from `forage-core::eval::transforms` doc comments), every auth flavor, every pagination strategy, every capture rule.
- **R13.3 — Runtime guide.** How the engine walks the recipe; auth/session lifecycle; rate-limit + retry behavior; sessions on disk; encryption.
- **R13.4 — Recipe cookbook.** Worked walk-throughs of each in-tree recipe.
- **R13.5 — Contribution guide.** How to clone, build, run tests; how to add a transform / auth flavor / pagination strategy; CI expectations.
- **R13.6 — Embedded in site.** mdbook output published to a Cloudflare Pages site at `docs.foragelang.com` or `foragelang.com/docs`.

**Acceptance:**
- `foragelang.com/docs` is navigable, searchable, and links from the main site's nav.
- Adding a new transform to `forage-core` auto-surfaces a doc entry on the next build.

---

## What gets deleted along the way

Greenfield: the Swift code is in git history. Each Rust counterpart deletes its predecessor.

| When | What dies |
|---|---|
| R3 ships | `Sources/forage-cli/**` (Swift CLI) |
| R5 ships | Last vestiges of `Sources/forage-cli/**` if any survived R3 |
| R6 ships | `Sources/Forage/Hub/**` (Swift HubClient + RecipeImporter) |
| R7 ships | — (LSP is new — nothing to delete) |
| R8 ships | `hub-site/forage-ts/**` (TS port) |
| R9 ships | `Studio/**` (SwiftUI app) |
| R9 ships | `Sources/Forage/**` (Swift core), `Tests/ForageTests/**` (Swift tests), `Package.swift` |
| R9 ships | `ROADMAP.md` (Swift roadmap) becomes `ROADMAP-history.md`; `ROADMAP-RUST.md` → `ROADMAP.md` |

`notes/`, `hub-api/`, `hub-site/.vitepress/`, `site/.vitepress/` all survive. The repo's shape pivots from "Swift package + apps" to "cargo workspace + apps + TS workers."

---

## Dependency graph (which crate needs which)

```
forage-core ─────┬─→ forage-http ──┬─→ forage-hub
                 ├─→ forage-browser┘
                 ├─→ forage-lsp
                 └─→ forage-wasm

forage-keychain ──┬─→ forage-http (encryption key)
                  └─→ forage-hub (auth store)

apps/cli depends on: forage-core, forage-http, forage-browser, forage-hub, forage-keychain, forage-lsp
apps/studio depends on: all of the above
forage-wasm depends on: forage-core, forage-http (replay-only mode)
```

This is the dependency order — R1 → R2 → R4 → R6 are the longest path. R3/R5/R7/R8/R9 fall out as soon as their dependencies are met. R10/R11/R12/R13 are independent polish/distribution work.

---

## Architectural decisions worth pinning now

(Decisions where flexibility is cheap but having one default helps the rewrite proceed without bikeshedding.)

- **Edition:** Rust 2024.
- **Async runtime:** `tokio` everywhere; no `async-std` mixing.
- **Error handling:** `thiserror` for library crates (typed errors), `anyhow::Result` for binary crates (CLI). LSP errors are typed.
- **Logging:** `tracing` + `tracing-subscriber` everywhere; `--verbose` flag toggles per-target levels.
- **Serialization:** `serde` + `serde_json`. AST is fully Serialize/Deserialize. `JSONValue` uses `serde_json::Value` underneath but typed-exposed.
- **Span representation:** `Range<usize>` (chumsky-native). Conversion to LSP `Range` happens at the LSP layer (line/column resolved from the source text).
- **Strings:** `String` everywhere except hot-path internals where `Cow<'a, str>` makes sense.
- **HTTP body:** `Vec<u8>` (the body of a capture / response). JSON bodies parsed lazily into `serde_json::Value`.
- **Recipe DSL on disk:** unchanged — same `.forage` syntax.
- **Recipe storage:** `~/Library/Forage/Recipes/<slug>/` on Mac, `$XDG_DATA_HOME/forage/recipes/` on Linux, `%APPDATA%\Forage\Recipes\` on Windows. Helper `dirs` crate.
- **Cache/Sessions root:** same pattern with `Cache` / `Sessions` / `Auth` subdirs.
- **Codesign identity:** "Developer ID Application: Dmitry Minkovsky (TEAM_ID)" — same as planned for Swift Studio.
- **License:** kept as-is (whatever the current Swift project uses).
- **Communication style:** every diagnostic uses ariadne for human output + serializes to LSP `Diagnostic` for machine consumption. One source of truth, two renderers.

---

## Tracking

Each `Rn.m` deliverable gets a checkbox in this file as it lands. Each milestone closes with a `Status: landed.` line, the commit SHA of the closing commit, and a 1–2 sentence "Result" paragraph describing what actually shipped (matching the Swift roadmap's style).
