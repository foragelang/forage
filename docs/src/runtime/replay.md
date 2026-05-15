# Replay and fixtures

Every run produces captures: HTTP request/response pairs for HTTP
recipes, fetch/XHR + document snapshots for browser recipes. Saving
them under `_fixtures/<recipe>.jsonl` at the workspace root lets future
runs reproduce the same output without network.

## Workspace layout

Captures and snapshots live alongside source at the workspace root,
keyed by recipe header name:

```
<workspace>/
├── forage.toml
├── <recipe>.forage
├── _fixtures/
│   └── <recipe>.jsonl       # capture stream
└── _snapshots/
    └── <recipe>.json        # golden snapshot, written by `forage test --update`
```

## Recording captures

- **HTTP recipes**: `forage record <recipe>` runs the recipe live and
  writes every exchange to `_fixtures/<recipe>.jsonl`. The same JSONL
  is what `--replay` consumes.
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
forage test hacker-news            # diff produced vs golden
forage test hacker-news --update   # rewrite the golden
```

Workflow:

1. Author the recipe, record captures, validate the snapshot manually.
2. Run `forage test <recipe> --update` once to write
   `_snapshots/<recipe>.json`.
3. Commit `<recipe>.forage`, `_fixtures/<recipe>.jsonl`, and
   `_snapshots/<recipe>.json`.
4. CI runs `forage test`; any divergence prints a unified diff and
   exits non-zero.

When a real-world response changes, re-record captures, re-run
`--update`, eyeball the diff, commit.

## Why this matters

- **Reproducibility.** A recipe + its `_fixtures/` + its `_snapshots/`
  is a self-contained reviewable unit. Reviewers can verify the recipe
  extracts what its snapshot claims without running anything.
- **No live-API flakiness in CI.** Every recipe in the repo runs offline.
- **Snapshot diffs over wire diffs.** When `forage test` fails, the
  diff is in the recipe's emit vocabulary, not raw HTTP bytes.
