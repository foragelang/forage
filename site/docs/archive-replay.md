# Archive & replay

Every recipe run can be persisted to disk and replayed later. The archive is the unit of "what this recipe produced against this site on this day"; the replayer is how you iterate the recipe's extraction logic against a frozen response set without re-hitting the network.

## Archive layout

`Archive.write(...)` writes one run, atomically, under a caller-supplied root:

```
<root>/<slug>/<ISO8601-Z>/
    snapshot.json
    diagnostic.json
    captures.jsonl     # browser-engine only; omitted if empty or nil
    meta.json
```

- `<slug>` is the recipe / scraper identifier you supplied — typically `recipe.name`.
- `<ISO8601-Z>` is the run's `observedAt` rendered as a filesystem-safe timestamp (colons → dashes, e.g. `2026-05-10T15-22-03Z`). The substitution preserves lexical ordering — `list(...)` sorts directory names directly and returns newest-first.
- `snapshot.json` is the produced records, pretty-printed for diffing.
- `diagnostic.json` is the run's `DiagnosticReport`. See [diagnostics](/docs/diagnostics).
- `captures.jsonl` is one `Capture` per line, sorted-key JSON. Browser engine only — pass it back through `BrowserReplayer` to re-run.
- `meta.json` is `ArchiveMeta`: `recipeName`, `inputs`, `runtimeSeconds`, `observedAt`.

Writes are atomic. The contents are staged into a sibling `<dir>.writing/` directory and renamed onto the final path only after every file lands. A crash mid-write leaves a stale `.writing/` staging dir, never a half-populated final dir.

## Writing a run

```swift
let result = try await runner.run(recipe: recipe, inputs: inputs)

let handle = try Archive.write(
    root:     archiveRoot,
    slug:     recipe.name,
    snapshot: result.snapshot,
    report:   result.report,
    captures: browserEngine?.captures,        // nil for HTTP runs
    meta: ArchiveMeta(
        recipeName:     recipe.name,
        inputs:         inputs,
        runtimeSeconds: elapsed,
        observedAt:     Date()
    )
)
print("archived: \(handle.directory.path)")
```

## Listing and reading

```swift
let runs = Archive.list(root: archiveRoot, slug: "jane")
// newest first

if let latest = runs.first {
    let (snapshot, report, captures, meta) = try Archive.read(latest)
    // ...
}
```

`Archive.list` skips `.writing/` staging dirs and any entry whose name doesn't parse as a timestamp.

## Replaying a browser run

`BrowserReplayer` drives a `BrowserEngine` run from captures instead of from a live `WKWebView`. The engine skips navigation, age-gate dismissal, warmup, pagination, the settle timer, and the hard timeout — it just feeds each capture through the same `captures.match` pipeline a live run would. Useful when:

- You're iterating on extraction logic and don't want to re-hit a rate-limited site every change.
- You're diffing against a known-good run after editing the recipe.
- You're testing offline.

Construct the replayer from a captures file:

```swift
let replayer = try BrowserReplayer(capturesFile: handle.directory.appendingPathComponent("captures.jsonl"))
```

Or directly from an in-memory list:

```swift
let replayer = BrowserReplayer(captures: someCaptures)
```

Then pass it into the engine:

```swift
let engine = BrowserEngine(
    recipe:   recipe,
    inputs:   inputs,
    replayer: replayer
)
let result = try await engine.run()
```

The returned `RunResult` reflects what the *current* recipe extracts from the *frozen* captures — perfect for "did my edit change the snapshot?" diffs.

::: tip Replay is the loop, not the test
Replay isn't a substitute for end-to-end live runs — it just gives you a fast iteration cycle. Re-record captures whenever the site shape changes, or your replay results will silently diverge from production.
:::
