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
    - title: Fixtures, snapshots, replay
      details: A recipe directory bundles recipe.forage alongside captured HTTP responses and an expected-output snapshot. Tests run offline. Drift is one command to repair.
    - title: LLM-friendly authoring
      details: Published grammar, fixed transform vocabulary, validator errors that speak the DSL's own terms. Hand the spec plus a few reference recipes to an AI and let it write the rest.
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

## Status

Forage is in early development. The Swift runtime ships in production as the scraping path for `weed-prices` and bundles recipes for three commercial dispensary platforms. Output types are currently scoped to that consumer's schema — designed to be lifted, not yet lifted. See the [GitHub](https://github.com/foragelang/forage) for the current roadmap.
