---
layout: home

hero:
    name: Forage
    text: A declarative DSL for web scraping
    tagline: Recipes describe what to fetch. The engine runs the HTTP, the pagination, and the type-directed extraction.
    image:
        src: /favicon.svg
        alt: Forage
    actions:
        - theme: brand
          text: Install
          link: /docs/install
        - theme: alt
          text: Get started
          link: /docs/getting-started
        - theme: alt
          text: GitHub
          link: https://github.com/foragelang/forage

features:
    - title: Recipes are data, not code
      details: A recipe describes what to scrape. The engine is the only thing that runs HTTP or emits records.
    - title: Two engines, one DSL
      details: HTTP for documented APIs, a headless browser for JS-rendered sites and bot-management gates.
    - title: Live progress, every run
      details: Stream live status, requests sent, and the current URL into a UI or a log line.
    - title: Diagnostic reports explain stalls
      details: Every run returns a structured report. Why it stopped, which rules never fired, which expectations didn't hold.
    - title: Expectations close the loop
      details: Recipes declare their own coverage invariants ("at least 500 Products"). The engine checks them and reports gaps.
    - title: Archive every run, replay any of them
      details: Every run is archived to disk. Replay against the captures to iterate on extraction. No network needed.
    - title: Hot-reload during development
      details: Edit a recipe and it reloads on save. A failed reload keeps the previous version live.
---

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
// SwiftUI binds directly via @Observable. No polling, no @Published.
Text("phase: \(String(describing: engine.progress.phase))")
Text("records: \(engine.progress.recordsEmitted)")
```

## Status

Forage is in early development. The Swift runtime ships in production as the scraping path for `weed-prices` and bundles recipes for three commercial dispensary platforms. Output types are currently scoped to that consumer's schema, designed to be lifted but not yet. See the [GitHub](https://github.com/foragelang/forage) for the current roadmap.
