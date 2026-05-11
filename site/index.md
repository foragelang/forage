---
layout: home

hero:
    name: Forage
    text: A declarative DSL for web scraping
    tagline: Recipes describe what to fetch — the engine runs the HTTP, the pagination, and the type-directed extraction.
    image:
        src: /favicon.svg
        alt: Forage
    actions:
        - theme: brand
          text: Get started
          link: /docs/getting-started
        - theme: alt
          text: GitHub
          link: https://github.com/foragelang/forage

features:
    - title: Recipes are data, not code
      details: A recipe is a declarative file — HTTP graph, pagination strategy, and type-directed extraction. The engine is the only thing that runs HTTP, parses responses, or produces output.
    - title: Two engines, one DSL
      details: HTTP for documented APIs. Headless WKWebView for JS-rendered SPAs and bot-management gates. Both target the same record types, so downstream code doesn't care which engine ran.
    - title: Live progress, every run
      details: Both engines expose an @Observable progress signal — phase, requests sent, captures observed, records emitted, current URL. Wire it into a SwiftUI status strip or a CLI log line without polling.
    - title: Diagnostic reports explain stalls
      details: Every run returns a snapshot plus a structured report — why it stopped, which capture rules never fired, which captures matched nothing, which expect-clauses didn't hold. No more "the snapshot looks thin and I don't know why."
    - title: Expectations close the loop
      details: A recipe declares its own coverage invariants ("at least 500 Products", "every store emits a non-zero variant count"). The engine evaluates them against the produced snapshot and surfaces gaps in the diagnostic report.
    - title: Archive every run, replay any of them
      details: One call writes the snapshot, diagnostic report, captures, and run metadata atomically to disk. Point a replayer at the captures and iterate the recipe's extraction logic against a frozen response set — no network, no SPA, sub-second turnaround.
    - title: Hot-reload during development
      details: The recipe registry polls the recipes directory and reloads on change. A failed reload keeps the previous version in place — typos never blank out a working recipe.
---

## A recipe, end to end

The smallest useful recipe: hit Wikipedia's REST API and emit one typed `Article`.

```forage
recipe "wikipedia" {
    engine http

    type Article {
        title:   String
        extract: String
        url:     String
    }

    input topic: String

    step page {
        method "GET"
        url    "https://en.wikipedia.org/api/rest_v1/page/summary/{$input.topic}"
    }

    emit Article {
        title   ← $page.title
        extract ← $page.extract
        url     ← $page.content_urls.desktop.page
    }
}
```

Run it:

```sh
forage-probe run recipes/wikipedia --input topic=Foraging
```

That's the whole shape: declare the records you want, name the HTTP requests, bind fields to paths in the response. Add `for` loops to iterate, `paginate` blocks for paginated APIs, `auth` strategies for gated endpoints — all on the same template.

## What an engine returns

A run produces a `RunResult`: the typed `Snapshot` plus a `DiagnosticReport` that explains how the run terminated. A short run carries its own receipts.

```swift
let result = try await runner.run(recipe: recipe, inputs: inputs)

print("emitted \(result.snapshot.records.count) records")
print("stallReason: \(result.report.stallReason)")
for unmet in result.report.unmetExpectations {
    print("  expectation gap: \(unmet)")
}
for unfired in result.report.unfiredRules {
    print("  rule never matched: \(unfired)")
}
```

Both engines expose `progress` for live UI:

```swift
// SwiftUI binds directly via @Observable — no polling, no @Published.
Text("phase: \(String(describing: engine.progress.phase))")
Text("records: \(engine.progress.recordsEmitted)")
```

## Status

Forage is in early development. The Swift runtime ships in production as the scraping path for `weed-prices` and bundles recipes for three commercial dispensary platforms. Output types are currently scoped to that consumer's schema — designed to be lifted, not yet lifted. See the [GitHub](https://github.com/foragelang/forage) for the current roadmap.
