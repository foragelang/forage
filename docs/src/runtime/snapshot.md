# Snapshot and records

A run produces a `Snapshot`: every emitted record in emission order,
plus a `DiagnosticReport`.

```json
{
    "records": [
        { "typeName": "Story", "fields": { "title": "Hardware Attestation as Monopoly Enabler", "points": 2095, "url": "..." } },
        { "typeName": "Story", "fields": { "title": "Postmortem: TanStack...", "points": 624 } }
    ],
    "diagnostic": {
        "stall_reason": null,
        "unmet_expectations": [],
        "unfired_capture_rules": [],
        "unmatched_captures": [],
        "unhandled_affordances": []
    }
}
```

This is the canonical wire format — what the CLI prints with
`--output json`, what `forage test` diffs against
`expected.snapshot.json`, what hub-published snapshots use.

## Record shape

Each record carries:

- `typeName` — the recipe-declared type.
- `fields` — an object keyed by field name. Values are the result of
  the binding expression converted to JSON (`String`, `Int`, `Double`,
  `Bool`, `Array`, `Object`, `null`). HTML nodes flatten to outer-HTML
  strings at the emit boundary, so the on-disk snapshot is pure data.

Records appear in **emission order**. A loop that emits `Product`
followed by `Variant` for each item produces an interleaved list:
`[Product#1, Variant#1a, Variant#1b, Product#2, Variant#2a, ...]`. The
order is deterministic given the input + capture stream.

## DiagnosticReport

The diagnostic envelope tells the host how a run terminated and which
checks failed. Fields:

- **`stall_reason`** — set by engines when a run terminates other than
  "ran to completion." Example values: `"session-expired: re-run with --interactive"`,
  `"auth-mfa-cancelled"`, `"max-iterations exceeded"`.
- **`unmet_expectations`** — one string per `expect { … }` block whose
  predicate didn't hold against the produced records.
- **`unfired_capture_rules`** (browser engine) — `captures.match`
  patterns that never saw a matching capture during the run. Usually
  means the pattern is wrong or the endpoint changed.
- **`unmatched_captures`** (browser engine) — captures the recipe
  didn't claim. If the recipe is missing a `captures.match` for an
  endpoint you care about, look here.
- **`unhandled_affordances`** (browser engine) — pagination-shaped
  buttons/links the engine saw but didn't drive. If the page has a
  "View more" button and the recipe stopped before clicking it, you'll
  see it called out here.

`has_content()` is true when any of these are populated — the CLI uses
it to color its output and to set a non-zero exit code.

## Equality + diffing

Two snapshots compare equal iff they have the same records in the same
order with the same field values *and* equivalent diagnostic content.
`forage test` exploits this for golden-file workflows: capture once
with `--update`, then any run that diverges produces a `similar`-style
unified diff.
