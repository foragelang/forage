# About

**Forage Hub** is a social registry for [Forage](https://foragelang.com)
packages.

Forage is a DSL for web scraping: a recipe describes *what* to fetch
and *what* to extract, and the runtime handles the HTTP, pagination,
browser automation, and type-directed extraction. The unit of
distribution is a **package version** — one atomic artifact carrying
the recipe, any shared declarations, the replay fixtures, and the
snapshot the recipe produced against them.

The hub lets you:

- **Discover** packages by category, popularity, or author profile.
- **Star** packages you like; the count surfaces on each card.
- **Fork** any package into your own namespace and publish independent
  versions.
- **Depend** on packages from your own workspace — add an entry to
  `[deps]` in `forage.toml`, run `forage update`, and every recipe in
  the workspace sees the package's shared declarations.
- **Publish** your own with the CLI or Forage Studio.

Packages live at `<author>/<slug>`; the author is your GitHub login.

This site (`hub.foragelang.com`) is a thin browser over the registry.
The underlying data lives at `api.foragelang.com`:

- `GET /v1/packages` — list everything (filter by `category`, `sort` by `recent`/`stars`/`downloads`)
- `GET /v1/packages/<author>/<slug>` — package metadata
- `GET /v1/packages/<author>/<slug>/versions/<n>` — atomic version artifact
- `GET /v1/users/<author>` — public profile + their packages + stars
- `GET /v1/categories` — every category that has at least one package

Source: [github.com/foragelang/forage](https://github.com/foragelang/forage).
