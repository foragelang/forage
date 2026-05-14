# About

**Forage Hub** is a registry for [Forage](https://foragelang.com) packages.

Forage is a DSL for web scraping: a recipe describes *what* to fetch and *what*
to extract, and the runtime handles the HTTP, pagination, browser automation,
and type-directed extraction. The unit of distribution is a **package** — a
workspace's `.forage` files plus a `forage.toml` manifest. Single-file
recipes ship as one-file packages.

The hub lets you:

- **Discover** packages other people have already mapped to a site or API.
- **Depend** on them from your own workspace — add an entry to
  `[deps]` in `forage.toml`, run `forage update`, and every recipe in
  the workspace sees the package's shared declarations.
- **Publish** your own with the CLI or Forage Studio.

Slugs are `<namespace>/<name>`; the namespace is your GitHub login.

This site (`hub.foragelang.com`) is a thin browser over the registry. The
underlying data lives at `api.foragelang.com`:

- `GET /v1/packages` — list everything
- `GET /v1/packages/<namespace>/<name>` — fetch one (metadata + every file body)
- `GET /v1/packages/<namespace>/<name>/versions` — version history

Source: [github.com/foragelang/forage](https://github.com/foragelang/forage).
