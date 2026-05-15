# Replay and fixtures

Every run produces captures: HTTP request/response pairs for HTTP
recipes, fetch/XHR + document snapshots for browser recipes. Saving
them under `fixtures/captures.jsonl` next to the recipe lets future
runs reproduce the same output without network.

## Directory layout

Inside your workspace (`~/Library/Forage/Recipes/` on macOS by
convention) each recipe is one directory:

```
<slug>/
├── recipe.forage
├── expected.snapshot.json           # written by `forage test --update`
└── fixtures/
    ├── inputs.json
    └── captures.jsonl
```

## Recording captures

- **HTTP recipes**: `forage capture <url>` (CLI) — runs a one-shot
  request, writes the response to `captures.jsonl`. Or hand-write the
  file from any HAR / curl output.
- **Browser recipes**: open the recipe in Forage Studio, click
  **Capture**, the visible WebView records every fetch/XHR + the
  post-settle document. Save on close.

## JSONL shape

One capture per line. The schema is `forage_replay::Capture`:

```jsonl
{"kind":"http","url":"https://api.example.com/items?page=1","method":"GET","status":200,"response_headers":{},"body":"..."}
{"kind":"browser","subkind":"match","url":"...","method":"GET","status":200,"body":"..."}
{"kind":"browser","subkind":"document","url":"...","html":"<html>..."}
```

URL matching for HTTP captures is exact-path + sorted-query-parameter
comparison, so a fixture recorded with `?page=1&size=50` still matches
a request issued as `?size=50&page=1`. For browser `captures.match`,
the recipe's `urlPattern` regex is the matcher.

## `forage test`

```sh
forage test ~/Library/Forage/Recipes/hacker-news            # diff produced vs expected
forage test ~/Library/Forage/Recipes/hacker-news --update   # rewrite expected
```

Workflow:

1. Author the recipe, record captures, validate the snapshot manually.
2. Run `forage test --update` once to write `expected.snapshot.json`.
3. Commit `recipe.forage`, `fixtures/`, and `expected.snapshot.json`.
4. CI runs `forage test`; any divergence prints a unified diff and
   exits non-zero.

When a real-world response changes, re-record captures, re-run
`--update`, eyeball the diff, commit.

## Why this matters

- **Reproducibility.** A recipe + its fixtures + its expected snapshot
  is a self-contained reviewable unit. Reviewers can verify the recipe
  extracts what its snapshot claims without running anything.
- **No live-API flakiness in CI.** Every recipe in the repo runs offline.
- **Snapshot diffs over wire diffs.** When `forage test` fails, the
  diff is in the recipe's emit vocabulary, not raw HTTP bytes.
