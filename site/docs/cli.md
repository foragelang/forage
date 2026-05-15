# CLI reference

The `forage` binary is the entry point for everything you do at the
shell. The subcommands you reach for most often:

| Command | What it does |
|---|---|
| [`forage run`](#forage-run) | Parse a recipe, run it, print the snapshot. |
| [`forage record`](#forage-record) | Run a recipe live and write `_fixtures/<recipe>.jsonl`. |
| [`forage test`](#forage-test) | Run a recipe against fixtures, diff against `_snapshots/<recipe>.json`. |
| [`forage new`](#forage-new) | Scaffold `<workspace>/<name>.forage` with a recipe header. |
| [`forage init`](#forage-init) | Drop a `forage.toml` skeleton so a directory becomes a workspace. |
| [`forage publish`](#forage-publish) | Push a recipe to the Forage hub by header name. |
| [`forage migrate`](#forage-migrate) | One-shot restructure of a legacy-shape workspace. |

Run `forage --help` or `forage help <subcommand>` for the full argument
surface. This page is the prose tour.

## Recipe addressing

Every recipe-scoped subcommand takes a **recipe header name** — the
string in the `recipe "<name>"` header inside the file. The resolver
walks ancestor directories looking for `forage.toml`, parses every
`.forage` file in the discovered workspace, and matches `<name>` against
each header. A path to a `.forage` file is accepted as a fallback for
recipes that aren't inside a workspace.

```sh
cd ~/Library/Forage/Recipes
forage run hello                   # resolves "hello" to hello.forage's header
forage run ./hello.forage          # path fallback works too
```

The file's basename is incidental — a file named `foo.forage` can
declare `recipe "bar"` and be run as `forage run bar`.

## `forage run`

Run a `.forage` recipe end-to-end and print the snapshot as JSON.

```sh
forage run <recipe> [--replay] [--inputs <path>] [--format pretty|json|jsonld]
```

### Options

- `--inputs <path>` — path to a JSON object of input bindings. Omit for
  recipes without inputs.
- `--replay` — replay against `<workspace>/_fixtures/<recipe>.jsonl`
  instead of hitting the network.
- `--format <pretty|json|jsonld>` — output format. Default `pretty`.
  `jsonld` emits a JSON-LD document driven by the recipe's type
  alignments (`@context` / `@graph`).

### Output

- **stdout**: pretty-printed `Snapshot` JSON.
- **stderr**: validation warnings (if any), `stallReason` line,
  diagnostic report sections.

Exit codes: `0` on success, `1` on runtime failure, `2` on parse /
validation failure, `3` when one or more `expect { … }` blocks are
unmet.

### Examples

```sh
# Run a Wikipedia recipe with an inputs file
echo '{"topic":"Foraging"}' > /tmp/inputs.json
forage run wikipedia --inputs /tmp/inputs.json

# Browser-engine recipe in replay mode
forage run jane --replay --inputs /tmp/jane.json
```

`--mfa` / `--no-mfa` semantics for recipes whose `auth.session.<...>`
block declares `requiresMFA: true` are covered in
[Authenticated sessions](/docs/auth-sessions).

## `forage record`

Run an HTTP-engine recipe live against the network and write every
exchange to `<workspace>/_fixtures/<recipe>.jsonl`. The recorded JSONL
is the same format `--replay` consumes — round-trip without network on
subsequent runs.

```sh
forage record <recipe> [--inputs <path>]
```

Browser-engine recipes need a real WebView for live capture; use Forage
Studio for those.

## `forage test`

Run a recipe in replay mode (against `_fixtures/<recipe>.jsonl`) and
diff the produced snapshot against `_snapshots/<recipe>.json`.

```sh
forage test <recipe> [--inputs <path>] [--update]
```

### Workspace layout

```
<workspace>/
├── forage.toml
├── <recipe>.forage
├── _fixtures/
│   └── <recipe>.jsonl       # replayable capture stream
└── _snapshots/
    └── <recipe>.json        # golden snapshot, written by --update
```

The test command:

1. Resolves `<recipe>` to its header-bearing file.
2. Reads `_fixtures/<recipe>.jsonl`, replays it through the recipe.
3. If `_snapshots/<recipe>.json` exists, compares produced records to
   the golden.
4. Surfaces any `unmetExpectations` from the diagnostic report.

### Options

- `--inputs <path>` — JSON inputs map. Required for recipes that
  declare `input`s without defaults.
- `--update` — write the produced snapshot to
  `_snapshots/<recipe>.json`. Typical first run: `forage test <recipe>
  --update`; subsequent runs without `--update` are the regression gate.

### Exit codes

- `0` — snapshot matched (or no golden file exists yet) AND no unmet
  expectations.
- `1` — snapshot mismatch OR unmet expectations.
- `2` — setup error: missing recipe / bad parse / validation failure /
  run error.

### Examples

```sh
forage test sweed --update   # pin the current behavior
forage test sweed            # later — see what changed
```

## `forage new`

Scaffold `<workspace>/<name>.forage` at the workspace root with a
`recipe "<name>" engine http` header.

```sh
forage new <name> [--engine http|browser] [--workspace <dir>]
```

The resulting file holds a minimal skeleton — recipe header, one step,
one emit. Edit from there.

## `forage init`

Drop an empty `forage.toml` at the given directory (or cwd) so the
surrounding tree becomes a Forage workspace.

```sh
forage init [dir]
```

After this, `forage new`, `forage run`, etc. all key off the new
workspace.

## `forage publish`

Push a recipe to the Forage hub at `https://api.foragelang.com`.

```sh
forage publish <recipe> [--hub <url>] [--publish] [--token <jwt>]
```

The hub-side slug is the recipe's header name. `forage.toml` declares
the author segment via `name = "<author>/<anything>"`; the slug portion
after the slash is decorative — the recipe's header name wins.

Dry-run is the default — it prints the envelope without hitting the
network. Pass `--publish` to actually POST.

```toml
# forage.toml
name = "alice/anything"
```

```sh
forage publish hello                # dry-run
forage publish hello --publish      # POST → @alice/hello on the hub
```

Auth source precedence: `--token` flag → `FORAGE_HUB_TOKEN` env →
the local auth store (populated by `forage auth login`).

## `forage migrate`

Restructure a workspace from the pre-1.0 legacy layout
(`<slug>/recipe.forage` + `<slug>/fixtures/` + `<slug>/snapshot.json`)
into the current flat layout (`<recipe>.forage` + `_fixtures/<recipe>.jsonl`
+ `_snapshots/<recipe>.json`).

```sh
forage migrate [dir] [--apply]
```

Dry-run by default — prints the planned moves without touching the
filesystem. Pass `--apply` to mutate. The migration is idempotent: a
re-run on an already-flat workspace is a no-op.

## Other subcommands

- `forage auth login` / `logout` / `whoami` — GitHub OAuth device-code
  flow against `api.foragelang.com`. See [Hub: publish & import](/docs/hub).
- `forage sync <@author/slug>` / `forage fork <@author/slug>` — clone a
  published recipe into the current workspace.
- `forage update` — resolve `[deps]` in `forage.toml` against the hub
  and write `forage.lock`.
- `forage lsp` — start the Forage Language Server on stdio.
- `forage scaffold <captures.jsonl>` — heuristic-generate a starter
  recipe from a captures JSONL file.
