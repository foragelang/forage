# Diagnostics

Every engine run returns a `RunResult` carrying a `DiagnosticReport` alongside the snapshot. Read the report when a run came back smaller than expected — it tells you *why* without poking at engine internals.

## DiagnosticReport

```swift
public struct DiagnosticReport: Sendable, Hashable, Codable {
    public let stallReason:          String
    public let unmatchedCaptures:    [UnmatchedCapture]   // browser engine only
    public let unfiredRules:         [String]             // browser engine only
    public let unmetExpectations:    [String]
    public let unhandledAffordances: [String]
}
```

### stallReason

A short tag describing how the run terminated.

| Engine  | Value                          | Meaning                                                                          |
| ------- | ------------------------------ | -------------------------------------------------------------------------------- |
| HTTP    | `"completed"`                  | The walker finished every statement in the recipe body without throwing.         |
| HTTP    | `"failed: <description>"`      | An error interrupted the walker. The snapshot has whatever was emitted before.   |
| HTTP    | `"cancelled"`                  | `Task.cancel()` arrived mid-run. Snapshot reflects work completed up to that point. |
| Browser | `"settled"`                    | The page went quiet (no new captures within `settleSeconds`).                    |
| Browser | `"hard-timeout"`               | `hardTimeoutSeconds` elapsed before the page settled.                            |
| Browser | `"navigation-failed: <url>"`   | The initial `WKWebView.load(...)` itself failed.                                 |
| Browser | `"cancelled"`                  | `Task.cancel()` arrived mid-run.                                                 |

### unmatchedCaptures

Browser engine only. Captures whose response URL matched none of the recipe's `captures.match` rules — bounded at the most recent 50 entries so a runaway SPA can't blow up memory.

```swift
public struct UnmatchedCapture {
    public let url:       String
    public let method:    String
    public let status:    Int
    public let bodyBytes: Int    // body itself is dropped to keep the report Sendable
}
```

Skim this when the snapshot is empty but the browser obviously fetched data: it tells you which endpoints the SPA hit that no rule claimed.

### unfiredRules

Browser engine only. The `urlPattern` of every `captures.match` rule that never matched any capture during the run. Highlights stale patterns or wrong endpoints — if a rule never fires, either it's pointing at the wrong URL or the SPA didn't reach the code path that fires it.

### unmetExpectations

A pre-rendered string per failing `expect { … }` clause. See the [expectations page](/docs/expectations) for syntax and rendering.

```
records.where(typeName == "Product").count >= 500 (got 247)
```

### unhandledAffordances

Reserved for future use. Currently always empty.

## Reading a report

Typical pattern: run, then triage by report.

```swift
let result = try await runner.run(recipe: recipe, inputs: inputs)

if !result.report.unmetExpectations.isEmpty {
    print("expectation gaps:")
    for s in result.report.unmetExpectations { print("  \(s)") }
}

if !result.report.unfiredRules.isEmpty {
    print("rules that never matched a capture:")
    for s in result.report.unfiredRules { print("  \(s)") }
}

if !result.report.unmatchedCaptures.isEmpty {
    print("captures with no rule:")
    for c in result.report.unmatchedCaptures {
        print("  \(c.method) \(c.url) [\(c.status), \(c.bodyBytes)B]")
    }
}
```

The three lists are independent signals:

- **Unmatched captures** with **unfired rules** of similar shape → the rule's URL pattern is wrong; the SPA *is* fetching the data, just at a URL the rule didn't expect.
- **Unmet expectations** alone, with all rules fired and no unmatched captures → the recipe extracts everything the SPA exposed, but the SPA exposed less than expected (a filter, an empty category, an upstream change in coverage).
- **Stall reason** `"hard-timeout"` with non-empty captures → the page kept loading; raise `hardTimeoutSeconds` or refine the pagination `until` clause.

## Persisting reports

`Archive.write(...)` writes the report to `diagnostic.json` next to the snapshot — see [archive & replay](/docs/archive-replay). Re-reading a run lets you diff today's `unfiredRules` against last week's without rerunning the engine.
