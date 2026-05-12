# Forage

A declarative scraping platform: a small DSL for describing how to fetch
structured data from a website, plus a Rust runtime that executes
recipes against an HTTP engine or a real browser engine
(`wry`-backed `WKWebView` on macOS, `WebView2` on Windows, `WebKitGTK`
on Linux).

Lives at **foragelang.com**.

## What you can do today

- Write a `.forage` recipe (see [`recipes/`](./recipes/)) and run it
  end-to-end via the CLI:

  ```sh
  forage run recipes/hacker-news
  forage run recipes/letterboxd-popular --replay
  ```

- Validate / test recipes with rich diagnostics:

  ```sh
  forage test recipes/hacker-news      # diff vs expected.snapshot.json
  forage test recipes/hacker-news --update   # snapshot, golden-file workflow
  ```

- Sign in to the hub and publish a recipe:

  ```sh
  forage auth login                    # GitHub OAuth device-code flow
  forage publish recipes/hacker-news --publish
  ```

- Get LSP-grade editing in any editor that speaks the protocol:

  ```sh
  forage lsp                           # JSON-RPC over stdio
  ```

- Author interactively in [Forage Studio](./apps/studio/) — Tauri +
  React + Monaco, embeds the LSP and drives the browser engine for
  live captures.

## Layout

```
crates/
├── forage-core/        # AST, parser, validator, evaluator, snapshot, transforms
├── forage-http/        # HTTP engine: auth, pagination, session cache, replay
├── forage-browser/     # wry-based browser engine + JS shim + replay
├── forage-hub/         # api.foragelang.com client + OAuth device-code flow
├── forage-keychain/    # cross-platform secret storage
├── forage-replay/      # capture types (HTTP + browser) + JSONL format
├── forage-lsp/         # tower-lsp server reused by Studio + VS Code
├── forage-wasm/        # wasm-bindgen exports for the web IDE
└── forage-test/        # shared-recipes test harness
apps/
├── cli/                # `forage` binary
└── studio/             # Forage Studio (Tauri 2 + React 19 + Monaco)
recipes/                # bundled platform recipes
hub-api/                # Cloudflare Worker (api.foragelang.com)
hub-site/               # VitePress (hub.foragelang.com)
site/                   # VitePress (foragelang.com)
docs/                   # mdbook (foragelang.com/docs)
DESIGN.md               # design plan
ROADMAP.md              # milestones R1–R13
```

## What problems it solves

- **Recipes are data, not code.** A site's scraping logic is a
  declarative file: HTTP graph + pagination strategy + type-directed
  extraction binding fields to a fixed output catalog. Engine evolves;
  recipes don't run code we don't trust.
- **Two engines, one DSL.** HTTP recipes for sites that expose a
  documented API; browser recipes for sites where the data sits behind
  a JS SPA + Cloudflare/Akamai bot management. Both target the same
  output type catalog, so downstream code doesn't care which engine ran.
- **Diagnostics speak recipe vocabulary.** When a run stalls — unmatched
  captures, unfired rules, expectation gaps, unhandled UI affordances —
  the engine surfaces them in the same language the recipe uses (URL
  patterns, type names, capture rule names). The corrective edit reads
  directly off the report.
- **Hub-friendly review.** Recipe + fixtures + snapshot ship together
  as a self-contained directory. Reviewers can verify a recipe extracts
  what its snapshot claims without running anything.

## Out of scope

- **Substantive access controls** (login, paywall, real CAPTCHA,
  account-required pages) — the headless engine doesn't bypass them.
  M10's interactive bootstrap hands off to a human for one-time
  challenges; the resulting session is reused headlessly until expiry.
- **Generic-purpose scraping framework** — output types are
  recipe-declared. Designed to be liftable later, not yet lifted.

## Building

```sh
cargo build --workspace
cargo test --workspace
forage --version
```

For Forage Studio:

```sh
cd apps/studio/ui && npm install
cd .. && cargo tauri dev
```

See [`RELEASING.md`](./RELEASING.md) for the release workflow,
[`ROADMAP.md`](./ROADMAP.md) for milestones, and
[`DESIGN.md`](./DESIGN.md) for the language design.
