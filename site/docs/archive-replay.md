# Archive & replay

Every recipe run can be persisted to disk and replayed later. Captures
freeze a particular HTTP / browser trace; snapshots freeze the records
the recipe extracted from it. Together they're the unit of "what this
recipe produced against this site on this day," and the basis for
iterating the recipe's extraction logic without re-hitting the network.

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
    └── <recipe>.json        # produced records
```

`_fixtures/<recipe>.jsonl` is the JSONL capture stream the replay
transport consumes. `_snapshots/<recipe>.json` is the golden snapshot
`forage test` diffs against. The filename matches the recipe's header
name; multiple scenarios per recipe land as
`_fixtures/<recipe>/<scenario>.jsonl` subdirs when the need arises.

## Capture shape

Each line in `_fixtures/<recipe>.jsonl` is one `forage_replay::Capture`:

```jsonl
{"kind":"http","url":"https://api.example.com/items?page=1","method":"GET","status":200,"response_headers":{},"body":"…"}
{"kind":"browser","subkind":"match","url":"https://api.iheartjane.com/v2/smartpage?page=1","method":"GET","status":200,"body":"…"}
{"kind":"browser","subkind":"document","url":"https://letterboxd.com/films/popular/","html":"<html>…</html>"}
```

HTTP captures match requests by exact-path + sorted-query-parameter
comparison, so a fixture recorded as `?page=1&size=50` still matches a
request issued as `?size=50&page=1`. Browser `captures.match` patterns
match by the recipe's `urlPattern` regex.

## Recording

- **HTTP-engine recipes** — `forage record <recipe>` runs the recipe
  live against the network and writes the exchange stream to
  `_fixtures/<recipe>.jsonl`. The same stream is what `forage run
  --replay` and `forage test` consume on subsequent runs.
- **Browser-engine recipes** — open the recipe in Forage Studio and
  click **Capture**; the visible WebView records every fetch / XHR and
  the post-settle document. Save on close.

## Replaying

`forage run <recipe> --replay` reads `_fixtures/<recipe>.jsonl` and
feeds the captures through the same evaluator a live run would. The
HTTP transport is swapped from live `reqwest` to the replay transport;
browser-engine recipes skip navigation, age-gate dismissal, warmup,
pagination, settle timer, and hard timeout — they just feed each
capture through `captures.match` / `captures.document` as a live run
would.

`forage test <recipe>` is the regression gate: it runs in replay mode
and diffs the produced snapshot against `_snapshots/<recipe>.json`,
exiting non-zero on divergence. `--update` overwrites the snapshot —
the typical first-run flow on a new recipe.

```sh
forage record sweed                   # capture once, live
forage test sweed --update            # pin the current behavior as golden
forage run sweed --replay             # iterate against the frozen captures
forage test sweed                     # later, after editing the recipe — diff
```

::: tip Replay is the loop, not the test
Replay isn't a substitute for end-to-end live runs — it gives you a
fast iteration cycle. Re-record captures whenever the site shape
changes, or your replay results will silently diverge from production.
:::
