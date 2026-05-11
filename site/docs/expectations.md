# Expectations

`expect { … }` clauses declare invariants the snapshot should satisfy. The engine evaluates them at the end of every run and surfaces unmet ones in the [diagnostic report](/docs/diagnostics#unmetexpectations) — so a thin snapshot never reaches the consumer without a structured explanation.

## Syntax

Expectations live at the top level of a recipe block, alongside `type`, `step`, and `emit`.

```forage
recipe "example" {
    engine http

    type Product { externalId: String; name: String }
    type Variant { sku: String; price: Double }

    // ... steps and emits ...

    expect { records.where(typeName == "Product").count >= 50 }
    expect { records.where(typeName == "Variant").count > 0 }
}
```

The supported shape today is:

```
records.where(typeName == "<TypeName>").count <op> <integer>
```

Where `<op>` is one of `>=`, `>`, `<=`, `<`, `==`, `!=`.

The validator checks that `<TypeName>` is a declared type. The engine counts records of that type in the produced snapshot and applies the comparison.

## Failure rendering

A failing expectation appears in `report.unmetExpectations` as a string that mirrors the source with the actual count appended:

```
records.where(typeName == "Product").count >= 500 (got 247)
```

A run with failing expectations still returns its snapshot — expectations are diagnostics, not aborts. Consumers decide whether to surface, retry, or archive based on report contents.

## When to use them

Use expectations when a thin snapshot is a *recipe bug*, not a *site reality*. Good fits:

- "This category page always has at least 50 products" — if it returns 3, the recipe missed a pagination round.
- "Every product has at least one variant" — if 30% are bare, the variant extraction broke.
- "Promotions endpoint emits non-zero placements" — if it's empty, the cookie/auth handshake didn't take.

Skip expectations for cases where the count is genuinely variable (search results, user-filtered lists, sparse categories). A noisy unmet expectation that fires constantly trains the consumer to ignore the report — defeating the point.

## Roadmap

The parsed AST is structural: today it only carries `recordCount(typeName, op, value)`. Future forms — predicates on field values, boolean combinators, comparisons across types — will render as `unsupported: <description>` until the evaluator learns them, rather than crash.
