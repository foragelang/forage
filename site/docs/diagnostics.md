# Diagnostics

Every engine run returns a `DiagnosticReport` alongside the snapshot. Read the report when a run came back smaller than expected — it tells you *why* without poking at engine internals.

## DiagnosticReport

```rust
pub struct DiagnosticReport {
    pub stall_reason:           Option<StallReason>,
    pub unmatched_captures:     Vec<UnmatchedCapture>,    // browser engine only
    pub unfired_capture_rules:  Vec<String>,              // browser engine only
    pub unmet_expectations:     Vec<ExpectationFailure>,
    pub unhandled_affordances:  Vec<String>,              // browser engine only
}
```

### stall_reason

A short tag describing how the run terminated.

| Engine  | Value                          | Meaning                                                                          |
| ------- | ------------------------------ | -------------------------------------------------------------------------------- |
| HTTP    | `"completed"`                  | The walker finished every statement in the recipe body without throwing.         |
| HTTP    | `"failed: <description>"`      | An error interrupted the walker. The snapshot has whatever was emitted before.   |
| HTTP    | `"cancelled"`                  | The task was cancelled mid-run. Snapshot reflects work completed up to that point. |
| Browser | `"settled"`                    | The page went quiet (no new captures within `settleSeconds`).                    |
| Browser | `"hard-timeout"`               | `hardTimeoutSeconds` elapsed before the page settled.                            |
| Browser | `"navigation-failed: <url>"`   | The initial WebView navigation itself failed.                                    |
| Browser | `"cancelled"`                  | The task was cancelled mid-run.                                                  |

### unmatched_captures

Browser engine only. Captures whose response URL matched none of the recipe's `captures.match` rules — bounded at the most recent 50 entries so a runaway SPA can't blow up memory.

```rust
pub struct UnmatchedCapture {
    pub url:        String,
    pub method:     String,
    pub status:     u16,
    pub body_bytes: usize,    // body itself is dropped to keep the report small
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

Browser engine only. At settle / hard-timeout / nav-fail the engine dumps every visible button / link / `role=button` on the page, keeps those whose label matches a pagination idiom — `view more`, `load more`, `next page`, `show more`, `see more`, `more results`, `older`, `next ›`, `›`, `→` — then subtracts (a) the labels the built-in load-more clicker drives, and (b) labels the recipe declared in `warmupClicks`. What's left is pagination-shaped UI the recipe didn't drive.

```
View more products (button.load-more-btn)
Older posts (a.archive-link)
```

This is the "the engine saw a pagination button but didn't click it" signal — directly actionable for recipe authors. Capped at 50 entries.

## Reading a report

`forage run` prints the report's non-empty sections after the snapshot. The three lists are independent signals:

- **Unmatched captures** with **unfired rules** of similar shape → the rule's URL pattern is wrong; the SPA *is* fetching the data, just at a URL the rule didn't expect.
- **Unmet expectations** alone, with all rules fired and no unmatched captures → the recipe extracts everything the SPA exposed, but the SPA exposed less than expected (a filter, an empty category, an upstream change in coverage).
- **Stall reason** `"hard-timeout"` with non-empty captures → the page kept loading; raise `hardTimeoutSeconds` or refine the pagination `until` clause.

In Studio, the Diagnostic tab renders each section as a list — click a row to jump to the relevant span in the editor.

## Persisting reports

The diagnostic is part of the `Snapshot` that `forage test` diffs against `_snapshots/<recipe>.json`. Re-reading a snapshot lets you diff today's `unfired_capture_rules` against last week's without rerunning the engine.
