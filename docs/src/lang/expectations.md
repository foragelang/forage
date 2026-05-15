# Expectations

A recipe's `expect { … }` blocks are postconditions the runtime
evaluates against the final snapshot. They don't fail the recipe (the
emit records still land in the snapshot) — they populate
`diagnostic.unmet_expectations` for the host to display.

```forage
expect { records.where(typeName == "Product").count >= 100 }
expect { records.where(typeName == "PriceObservation").count > 0 }
```

## Syntax

The current grammar accepts one shape:

```text
records.where(typeName == "<Type>").count <op> <integer>
```

`<op>` is one of `>=`, `>`, `<=`, `<`, `==`, `!=`.

Multiple `expect` blocks coexist; each evaluates independently. The
order doesn't matter.

## Why this matters

The diagnostic is the recipe's running yardstick. A Jane recipe
expecting `≥ 500 Products` immediately tells you something's off when
a run produces 12 — a wrong selector, a CAPTCHA gate, or an API change.

The CLI's `forage run` returns non-zero exit code 3 when any expectation
is unmet (snapshot still printed). The Studio UI lights the Diagnostic
tab and shows each unmet expectation in red.

`forage test <recipe>` is the canonical CI gate: it runs the recipe in
replay mode, diffs the snapshot against `_snapshots/<recipe>.json`, and
also surfaces unmet expectations.

## Future shapes

Right now expectations only talk about record counts. The roadmap has
slots for:

- `records.where(typeName == "X" && $.field == "Y").count`
- `records.where(typeName == "X" && all(field nonzero)).count`
- Diagnostic-level expectations (`expect { diagnostic.unmatched_captures.empty }`)

These will layer onto the existing grammar without breaking earlier
recipes.
