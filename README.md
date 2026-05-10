# Forage

A declarative scraping platform: a small DSL for describing how to fetch structured data from a website, plus a Swift runtime that executes recipes against either a plain HTTP engine or a real browser engine (`WKWebView` on macOS).

Will live at **foragelang.com** when the platform stabilizes.

## Status

**Early / experimental.** Architecture and primitives are being shaken out in [`weed-prices`](https://github.com/...) (a personal cannabis-menu price tracker that's the first consumer). The recipe DSL and engine vocabulary are still in design; the engine code is in the early stages of being carved out of `weed-prices/scripts/probe.swift` (the rough-draft reverse-engineering CLI) into the `Forage` library here.

Canonical artifacts at this point:

- [`DESIGN.md`](./DESIGN.md) — design plan: principles, output type catalog, recipe shape, pagination strategies, dev/test workflow, engine responsibilities.
- `Sources/Forage/` — placeholder Swift module; engine code lands here in pieces (parser, HTTPEngine, BrowserEngine, Recipe, OutputCatalog, DiagnosticReport).

## What problems it solves

- **Recipes are data, not code.** A site's scraping logic is a declarative file: HTTP graph + pagination strategy + type-directed extraction binding fields to a fixed output catalog. Engine evolves; recipes don't run code we don't trust.
- **Two engines, one DSL.** HTTP recipes for sites that expose a documented API; browser recipes for sites where the data sits behind a JS SPA + cloudflare bot management. Both target the same output type catalog, so downstream code doesn't care which engine ran.
- **LLM authoring path is first-class.** A future recipe author hands the DSL spec + a few reference recipes + a target site URL to an AI assistant; the AI probes the site, captures fixtures, writes a recipe, snapshots the expected output. Diagnostic reports the engine emits when a recipe stalls (unhandled UI affordances, observed-but-unmatched URLs, expectation gaps) are written in the same vocabulary the recipe uses, so the corrective edit is a direct reading of the report.
- **Hub-friendly review.** Recipe + fixtures + snapshot ship together as a self-contained directory. Reviewers can verify a recipe extracts what its snapshot claims without running anything.

## Out of scope

- Substantive access controls (login, paywall, real CAPTCHA, account-required pages) — recipes don't bypass them. Generic bot-management gates on otherwise-public pages are not in this category.
- Generic-purpose scraping framework — output types are currently fixed to the consumer's schema. Designed to be liftable later, not yet lifted.

## Layout

```
Sources/Forage/        # Swift runtime: engine primitives (capture, BrowserPaginate, …)
Sources/forage-probe/  # CLI: WKWebView-hosted reverse-engineering tool
Tests/ForageTests/     # Engine unit tests
DESIGN.md              # Design plan
```

## Building

```sh
swift build
swift test
```
