# Forage

A declarative scraping platform: a small DSL for describing how to fetch structured data from a website, plus a Swift runtime that executes recipes against either a plain HTTP engine or a real browser engine (`WKWebView` on macOS).

Will live at **foragelang.com** when the platform stabilizes.

## Status

**Runtime operational, recipe-authoring tooling lands next.** Phases A-E of the [`PLANS.md`](./PLANS.md) execution plan are complete: parser, HTTP engine, browser engine, validator, fixture replay, snapshot codable. 27 tests green.

What you can do today:

- Write a `.forage` recipe (see [`recipes/examples/`](./recipes/examples/)) and parse it via `Parser.parse(source:)`.
- Run an HTTP-engine recipe end-to-end via `RecipeRunner.run(recipe:inputs:)` against `URLSessionTransport` for live or `HTTPReplayer` for fixture replay.
- Run a browser-engine recipe via `BrowserEngine.run()` on the main actor (consumer drives `NSApplication`).
- Statically validate any recipe via `Validator.validate(_:)` — catches unknown types/fields/transforms, unbound path variables, missing required fields.
- Reverse-engineer a new platform with `forage-probe capture <url>` (legacy mode) and inspect the captured JSONL.
- Encode/decode `Snapshot` values via `SnapshotIO.encode(_:)` / `.decode(_:)` for offline snapshot round-tripping.

What lands next (Phases F-H, see [`PLANS.md`](./PLANS.md)): port the Sweed / Leafbridge / Jane / Dutchie platforms to actual recipes (capturing fixtures alongside), wire Forage as a Swift package dep in the weed-prices consumer, write a `SnapshotPersister` that translates Forage `ScrapedRecord`s into the consumer's SQLite schema by `typeName`, retire the bespoke per-platform Swift scrapers.

Canonical artifacts:

- [`DESIGN.md`](./DESIGN.md) — design plan: principles, output type model, recipe shape, pagination strategies, dev/test workflow.
- [`PLANS.md`](./PLANS.md) — execution plan for phases A-H with files, types, validator checks, anti-patterns.
- [`recipes/examples/`](./recipes/examples/) — canonical example recipes for Sweed / Leafbridge / Jane.
- [`Sources/Forage/`](./Sources/Forage/) — runtime library (parser, engines, validator, fixture replay).
- [`Sources/forage-probe/`](./Sources/forage-probe/) — `forage-probe run <recipe>` and `forage-probe capture <url>` CLI.

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
