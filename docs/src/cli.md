# `forage` CLI

The Rust binary at `target/release/forage` (or wherever the installer
dropped it). All subcommands are flat — `forage <subcommand>
[options]`.

```text
forage run        Parse and execute a .forage recipe; print the snapshot
forage test       Run a recipe against fixtures and diff vs expected snapshot
forage capture    Launch a webview and record fetch/XHR exchanges to JSONL
forage scaffold   Build a starter .forage recipe from a captures JSONL file
forage publish    Push a recipe to the Forage hub
forage auth       Sign in / out / check status against the Forage hub via GitHub
forage lsp        Start the Forage Language Server on stdio
```

## `forage run <dir>`

Loads `<dir>/recipe.forage`, validates, loads
`<dir>/fixtures/inputs.json`, runs the engine, prints the snapshot.

Flags:
- `--replay` — use `<dir>/fixtures/captures.jsonl` instead of hitting
  the network.
- `--output {pretty|json}` — output format. Default `pretty`.

Exit codes:
- `0` — clean run.
- `1` — runtime / transport error.
- `2` — parse or validation error.
- `3` — one or more `expect { … }` blocks failed.

```sh
forage run ~/Library/Forage/Recipes/hacker-news
forage run ~/Library/Forage/Recipes/hacker-news --output json | jq '.records | length'
forage run ~/Library/Forage/Recipes/letterboxd-popular --replay
```

## `forage test <dir>`

Replay-mode run, then diffs the produced snapshot against
`<dir>/expected.snapshot.json` using the `similar` crate.

- `--update` (or no expected file) writes the produced snapshot as the
  new golden.

```sh
forage test ~/Library/Forage/Recipes/hacker-news
forage test ~/Library/Forage/Recipes/hacker-news --update
```

## `forage capture <url>`

Open a WebView, navigate to `<url>`, record every fetch/XHR + the
final document HTML to `captures.jsonl`. Useful for reverse-engineering
a new site before any recipe exists.

Today the CLI's capture command points users at Forage Studio (it needs
a tao event loop the standalone CLI doesn't host). Use Studio's
**Capture** button.

## `forage scaffold <captures.jsonl>`

Read a JSONL file of captures and print a starter recipe. Groups by
URL path, emits one `step` per unique request shape, scaffolds a
placeholder `Item` type and emit loop.

```sh
forage scaffold ~/captures.jsonl --name my-new-recipe > ~/Library/Forage/Recipes/my-new-recipe/recipe.forage
```

## `forage publish <dir>`

Push a recipe to the Forage hub.

```sh
forage publish ~/Library/Forage/Recipes/hacker-news                    # dry-run
forage publish ~/Library/Forage/Recipes/hacker-news --publish          # actually POST
forage publish ~/Library/Forage/Recipes/hacker-news --hub https://...  # alternate hub
forage publish ~/Library/Forage/Recipes/hacker-news --token $TOKEN     # explicit bearer
```

Auth source precedence: `--token` flag → `FORAGE_HUB_TOKEN` env →
`AuthStore` at `~/Library/Forage/Auth/<host>.json`.

## `forage auth`

```sh
forage auth login            # GitHub OAuth device-code flow
forage auth logout           # delete local tokens
forage auth logout --revoke  # also POST /v1/oauth/revoke (server-side invalidate)
forage auth whoami           # print <login>@<host>
```

`--hub <url>` selects which hub; default `https://api.foragelang.com`.

## `forage lsp`

Start the Forage Language Server on stdio — JSON-RPC over
stdin/stdout. Editors connect to it directly:

```jsonc
// VS Code settings.json
{
    "forage.lsp.path": "/usr/local/bin/forage",
    "forage.lsp.args": ["lsp"]
}
```

See [LSP](./lsp.md) for the capability list.

## Environment

| Var | Effect |
|---|---|
| `FORAGE_HUB_URL` | default hub for `publish` / `auth` |
| `FORAGE_HUB_TOKEN` | bearer token (overrides auth store) |
| `FORAGE_SECRET_<NAME>` | resolve `$secret.<name>` in recipes |
| `RUST_LOG=forage=debug` | per-target log levels |
