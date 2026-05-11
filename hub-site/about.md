# About

**Forage Hub** is a registry for [Forage](https://foragelang.com) recipes.

Forage is a DSL for web scraping: a recipe describes *what* to fetch and *what*
to extract, and the runtime handles the HTTP, pagination, browser automation,
and type-directed extraction. Recipes are flat text files, replayable,
diff-able, and tractable to share.

The hub lets you:

- **Discover** recipes for sites and APIs other people have already mapped.
- **Pull** them from the CLI: `forage import hub://<slug>`.
- **Publish** your own with the CLI or the Toolkit.

This site (`hub.foragelang.com`) is a thin browser over the registry. The
underlying data lives at `api.foragelang.com`:

- `GET /v1/recipes` — list everything
- `GET /v1/recipes/<slug>` — fetch one
- `GET /v1/recipes/<slug>/versions` — version history

Source: [github.com/foragelang/forage](https://github.com/foragelang/forage).
