# `forage` CLI

The Rust binary at `target/release/forage` (or wherever the installer
dropped it). All subcommands are flat — `forage <subcommand>
[options]`.

```text
forage run        Parse and execute a .forage recipe; print the snapshot
forage record     Run a recipe live and write _fixtures/<recipe>.jsonl
forage test       Run a recipe against fixtures and diff vs _snapshots/<recipe>.json
forage new        Scaffold <workspace>/<name>.forage with a recipe header
forage init       Drop a forage.toml so a directory becomes a workspace
forage update     Resolve forage.toml [deps] against the hub
forage publish    Push a recipe to the Forage hub by header name
forage sync       Clone a published recipe into the current workspace
forage fork       Fork an upstream recipe into your account, then clone
forage migrate    Restructure a legacy-shape workspace to the flat shape
forage auth       Sign in / out / check status against the Forage hub via GitHub
forage lsp        Start the Forage Language Server on stdio
forage scaffold   Build a starter .forage recipe from a captures JSONL file
forage capture    (stubbed — Studio drives live capture)
```

## Recipe addressing

Recipe-scoped subcommands (`run`, `record`, `test`, `publish`) take a
recipe header name. The resolver walks ancestor directories looking
for `forage.toml`, parses every `.forage` file in the workspace, and
matches `<name>` against each header. A path to a `.forage` file is
accepted as a fallback for recipes outside a workspace.

```sh
cd ~/Library/Forage/Recipes
forage run hello             # resolves the recipe by header name
forage run ./hello.forage    # path fallback works too
```

## `forage run <recipe>`

Validates, runs the engine, prints the snapshot.

Flags:
- `--inputs <path>` — path to a JSON object of input bindings.
- `--replay` — replay against `<workspace>/_fixtures/<recipe>.jsonl`
  instead of hitting the network.
- `--replay-from <path>` — replay against an explicit captures file
  (overrides `--replay`'s default fixture lookup).
- `--sample <N>` — cap each top-level `for $x in $arr[*]` iteration at
  N items. Nested loops always run to completion. Useful for a
  top-of-funnel sanity check against a real source.
- `--mode dev|prod` — preset bundle. `dev` is sugar for `--sample 10
  --replay`; `prod` is the empty preset. Explicit per-flag values
  override the preset's defaults.
- `--output {pretty|json}` — output format. Default `pretty`.

`--ephemeral` lives at the daemon / Studio layer where output
persistence is a real choice; `forage run` is already stateless so
it isn't surfaced here.

Exit codes:
- `0` — clean run.
- `1` — runtime / transport error.
- `2` — parse or validation error.
- `3` — one or more `expect { … }` blocks failed.

```sh
forage run hacker-news
forage run hacker-news --output json | jq '.records | length'
forage run letterboxd-popular --replay
forage run hacker-news --sample 5             # top-5 records from live
forage run hacker-news --mode dev             # sampled + replay against fixtures
forage run hacker-news --replay-from cap.jsonl
```

## `forage record <recipe>`

Run an HTTP-engine recipe live against the network and write every
exchange to `<workspace>/_fixtures/<recipe>.jsonl` — the same format
`--replay` consumes.

```sh
forage record hacker-news
forage record hacker-news --inputs ./my-inputs.json
```

Browser-engine recipes need a real WebView for live capture; use
Forage Studio.

## `forage test <recipe>`

Replay-mode run, then diffs the produced snapshot against
`<workspace>/_snapshots/<recipe>.json` using the `similar` crate.

- `--update` (or no snapshot file yet) writes the produced snapshot as
  the new golden.
- `--inputs <path>` — JSON inputs map.

```sh
forage test hacker-news
forage test hacker-news --update
```

## `forage new <name>`

Scaffold `<workspace>/<name>.forage` with a `recipe "<name>" engine
http` header at the workspace root.

```sh
forage new my-recipe
forage new my-browser-recipe --engine browser
```

## `forage init [dir]`

Drop an empty `forage.toml` so the surrounding tree becomes a Forage
workspace.

## `forage update [dir]`

Resolve `[deps]` in `forage.toml` against the hub, fetch each into the
local cache, and write `forage.lock`.

## `forage capture <url>`

Open a WebView, navigate to `<url>`, record every fetch/XHR + the
final document HTML. Today the CLI's capture command points users at
Forage Studio (it needs a tao event loop the standalone CLI doesn't
host). Use Studio's **Capture** button.

## `forage scaffold <captures.jsonl>`

Read a JSONL file of captures and print a starter recipe. Groups by
URL path, emits one `step` per unique request shape, scaffolds a
placeholder `Item` type and emit loop.

```sh
forage scaffold ~/captures.jsonl --name my-new-recipe > ~/Library/Forage/Recipes/my-new-recipe.forage
```

## `forage publish <recipe>`

Push a recipe to the Forage hub. The hub-side slug is the recipe's
header name. `forage.toml` declares the author segment via `name =
"<author>/<anything>"`; the slug portion after the slash is decorative.

```sh
forage publish hacker-news                    # dry-run
forage publish hacker-news --publish          # actually POST
forage publish hacker-news --hub https://...  # alternate hub
forage publish hacker-news --token $TOKEN     # explicit bearer
```

Auth source precedence: `--token` flag → `FORAGE_HUB_TOKEN` env →
`AuthStore` at `~/Library/Forage/Auth/<host>.json`.

## `forage sync @<author>/<slug>` / `forage fork @<author>/<slug>`

Clone a published recipe (or fork it first into your namespace) into
the current workspace.

```sh
forage sync alice/zen-leaf
forage fork alice/zen-leaf --as my-leaf
```

## `forage migrate [dir]`

One-shot restructure of a pre-1.0 workspace from
`<slug>/recipe.forage` + `<slug>/fixtures/` + `<slug>/snapshot.json`
into the current flat shape (`<recipe>.forage` +
`_fixtures/<recipe>.jsonl` + `_snapshots/<recipe>.json`). Dry-run by
default; pass `--apply` to mutate.

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
| `FORAGE_WORKSPACE_ROOT` | Studio's default workspace directory |
| `FORAGE_HUB_URL` | default hub for `publish` / `auth` |
| `FORAGE_HUB_TOKEN` | bearer token (overrides auth store) |
| `FORAGE_SECRET_<NAME>` | resolve `$secret.<name>` in recipes |
| `RUST_LOG=forage=debug` | per-target log levels |
