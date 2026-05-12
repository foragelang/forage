# Forage

Forage is a small declarative DSL for scraping recipes plus a runtime that
walks them: real HTTP, real WebKit, real captures, snapshot-typed output.

A recipe says *what* to scrape, not *how* to drive it. Pick an engine —
`http` for JSON APIs, `browser` for SPAs behind JS-challenge gates — then
declare your types, your inputs, your steps or browser config, and your
emit blocks. The runtime does the rest: rate-limited request shaping,
auth flavors (static header, htmlPrime, session.formLogin / bearerLogin),
pagination strategies, cookie threading, fetch/XHR interception inside
WebKit, settle detection, and snapshot collection.

The same recipe runs three places, identically:
- **`forage` CLI** — Rust binary, fast, scriptable, ships for macOS,
  Linux, Windows.
- **Forage Studio** — macOS desktop app (Tauri + React + Monaco) for
  authoring: live captures, replay against fixtures, snapshot diff,
  publish to the hub.
- **Web IDE** — `hub.foragelang.com/edit` runs the same parser /
  validator / HTTP runner via WebAssembly.

The hub at `api.foragelang.com` distributes recipes, with GitHub OAuth
identity and ownership-checked publish/delete.

## Why declarative

Most scrapers are imperative scripts that drift the day a site changes.
Recipes carry the *intent* — "I want every Product with these fields
from a Sweed-hosted dispensary's API" — and the runtime is the part you
keep up to date. When a transform breaks, you fix the transform once
and every recipe gains the fix.

## What you'll learn

Start with [Install](./install.md) and [Your first recipe](./first-recipe.md).
The language reference covers types, the HTTP and browser engines, auth,
pagination, captures, expressions, transforms, expectations, and imports.
The cookbook walks through the in-tree recipes end-to-end.

The runtime section covers the snapshot format, sessions and the
on-disk cache, the replay-fixture workflow, and M10's interactive
bootstrap (for sites that escalate to a human-verification challenge).

## Project layout

This repository is a cargo workspace plus a Cloudflare Worker and a
VitePress site:

```
forage/
├── crates/
│   ├── forage-core/        # AST, parser, validator, evaluator, snapshot
│   ├── forage-http/        # HTTP engine, auth, pagination, session cache
│   ├── forage-browser/     # wry-based browser engine
│   ├── forage-hub/         # hub client + OAuth device flow
│   ├── forage-keychain/    # cross-platform secret storage
│   ├── forage-replay/      # capture types + replayers
│   ├── forage-lsp/         # language server
│   ├── forage-wasm/        # wasm-bindgen exports for the web IDE
│   └── forage-test/        # shared-recipes test harness
├── apps/
│   ├── cli/                # `forage` binary
│   └── studio/             # Forage Studio (Tauri)
├── hub-api/                # Cloudflare Worker
├── hub-site/               # VitePress
├── docs/                   # this mdbook
└── recipes/                # in-tree recipes
```
