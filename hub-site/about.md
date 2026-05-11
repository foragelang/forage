# About

**Forage Hub** is a registry for [Forage](https://foragelang.com) recipes.

Forage is a DSL for web scraping: a recipe describes *what* to fetch and *what*
to extract, and the runtime handles the HTTP, pagination, browser automation,
and type-directed extraction. Recipes are flat text files, replayable,
diff-able, and tractable to share.

The hub lets you:

- **Discover** recipes for sites and APIs other people have already mapped.
- **Pull** them from your own recipes — `import alice/zen-leaf` and the
  runtime fetches the body, caches it, and unions its declarations into
  yours.
- **Publish** your own with the CLI or the Toolkit.

References are Docker-style: bare `name` resolves to the official `forage`
namespace; `alice/name` is a personal namespace; `hub.example.com/team/name`
or `localhost:5000/me/name` point to alternate registries.

This site (`hub.foragelang.com`) is a thin browser over the registry. The
underlying data lives at `api.foragelang.com`:

- `GET /v1/recipes` — list everything
- `GET /v1/recipes/<namespace>/<name>` — fetch one
- `GET /v1/recipes/<namespace>/<name>/versions` — version history

Source: [github.com/foragelang/forage](https://github.com/foragelang/forage).
