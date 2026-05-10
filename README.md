# Forage

A declarative scraping platform: a small DSL for describing how to fetch structured data from a website, plus a Swift runtime that executes recipes against either a plain HTTP engine or a real browser engine (`WKWebView` on macOS).

## Status

**Early / experimental.** Architecture and primitives are being shaken out in `../weed-prices` (a personal cannabis-menu price tracker that's the first consumer). Forage will get its own design docs and identity as it stabilizes; right now the canonical design plan lives at `../weed-prices/notes/scraping-dsl.md` and the empirical platform validation work is in `../weed-prices/notes/jane-platform.md` and `../weed-prices/scripts/probe.swift`.

## What problems it solves

- **Recipes are data, not code.** A site's scraping logic is a declarative file: HTTP graph + pagination strategy + type-directed extraction binding fields to a fixed output catalog. Engine evolves; recipes don't run code we don't trust.
- **Two engines, one DSL.** HTTP recipes for sites that expose a documented API; browser recipes for sites where the data sits behind a JS SPA + cloudflare bot management. Both target the same output type catalog, so downstream code doesn't care which engine ran.
- **LLM authoring path is first-class.** A future recipe author hands the DSL spec + a few reference recipes + a target site URL to an AI assistant; the AI probes the site, captures fixtures, writes a recipe, snapshots the expected output. Diagnostic reports the engine emits when a recipe stalls (unhandled UI affordances, observed-but-unmatched URLs, expectation gaps) are written in the same vocabulary the recipe uses, so the corrective edit is a direct reading of the report.
- **Hub-friendly review.** Recipe + fixtures + snapshot ship together as a self-contained directory. Reviewers can verify a recipe extracts what its snapshot claims without running anything.

## Out of scope

- Substantive access controls (login, paywall, real CAPTCHA, account-required pages) — recipes don't bypass them. Generic bot-management gates on otherwise-public pages are not in this category.
- Generic-purpose scraping framework — output types are currently fixed to the consumer's schema. Designed to be liftable later, not yet lifted.

## Layout (planned)

```
Sources/Forage/        # Swift runtime: parser, engine, type catalog
Tests/ForageTests/     # Engine unit tests
recipes/               # Reference recipes (not yet committed)
DESIGN.md              # Design plan (will move from ../weed-prices/notes/scraping-dsl.md)
```

## Building

```sh
swift build
swift test
```

(Both will be no-ops until the engine code lands.)
