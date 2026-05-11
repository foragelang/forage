---
layout: home

hero:
    name: Forage
    text: A declarative DSL for structured web extraction
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
    - title: Share recipes via the hub
      details: Publish once, anyone can use it. Browse, import, and version recipes across projects without copy-paste or forks.
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
