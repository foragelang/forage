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

## This is what scraping looks like in Forage

A `.forage` file describes one site: its HTTP graph, how it paginates, and how each response maps into typed records. The engine does the rest.

```forage
recipe "store" {
    engine http

    type Product {
        externalId: String
        name:       String
        brand:      String?
        price:      Double?
        tags:       [String]
    }

    input storeId: String

    auth.staticHeader {
        name:  "X-Store-Id"
        value: $input.storeId
    }

    step products {
        method "POST"
        url    "https://api.example.com/products"
        paginate pageWithTotal {
            items:     $.list
            total:     $.total
            pageParam: "page"
            pageSize:  50
        }
    }

    for $p in $products[*] {
        emit Product {
            externalId ← $p.id | toString
            name       ← $p.name
            brand      ← $p.brand?.name
            price      ← $p.price
            tags       ← $p.tags[*].name | dedup
        }
    }
}
```

Run it with `forage-probe run recipes/store --input storeId=abc123` against either the live site or a directory of saved HTTP fixtures.

## Status

Forage is in early development. The Swift runtime ships in production as the scraping path for `weed-prices` and bundles recipes for three commercial dispensary platforms. Output types are currently scoped to that consumer's schema — designed to be lifted, not yet lifted. See the [GitHub](https://github.com/foragelang/forage) for the current roadmap.
