# Composition

A recipe's body is one of two kinds: a scraping body (the historical
shape — `step`, `for`, `emit`) or a **composition body** — a chain of
recipe references joined by `|`. The runtime walks the chain, feeding
each stage's emitted records to the next.

A composed recipe **is itself a recipe**. It declares the same
`recipe "<name>"` header, the same `output` signature, and publishes
under `/v1/recipes/` like any scraping recipe. There is no separate
"pipeline" citizen: one citizen (Recipe), two body kinds.

```forage
recipe "enriched-products"
engine http
output Product

compose "scrape-amazon" | "enrich-wikidata"
```

Each stage reference is a string literal carrying the referenced
recipe's header name. Workspace-local references use a bare name. The
parser accepts hub-dep references like `"@upstream/enrich-wikidata"`,
but the validator rejects them with `HubDepStageUnsupported` — the
runtime can't resolve published recipes referenced this way today.
Sync the upstream into your workspace first and reference it by bare
name.

## Stage signatures

For `compose A | B`:

- A must declare `output T` — exactly one type. Multi-type sum
  outputs in a composition chain are reserved for a future
  extension.
- B must declare exactly one input slot whose type matches the
  upstream:
  - `input <name>: [T]` — batched: B sees the entire stream at once.
    Typical for transformers and enrichers.
  - `input <name>: T` — single-record: B sees one record per
    upstream emission. Restricted to chains where the upstream emits
    a single record.

The downstream recipe accesses the upstream records through the
declared input by name:

```forage
recipe "enrich-wikidata"
engine http

share type Product { id: String }

input prior: [Product]

output Product

for $p in $input.prior {
    emit Product { id ← $p.id }
}
```

## Composition is closed under composition

A composition is itself a recipe with a typed signature, so it can
appear as a stage in another composition. Nested composition is the
common case once recipe authors start sharing pipelines:

```forage
recipe "ab-c"
engine http
output Product

compose "ab" | "c"
```

where `"ab"` is itself a composed recipe `compose "a" | "b"`. The
runtime resolves each stage's deployed version and walks the chain
recursively.

## Validation

The validator's composition rules:

- **`UnknownComposeStage`** — a bare-name stage isn't a recipe in
  the local workspace.
- **`HubDepStageUnsupported`** — the stage uses the
  `@author/name` hub-dep form; the runtime can't resolve those today,
  so the validator rejects them. Sync the upstream into your workspace
  and reference it by bare name.
- **`UnsignedComposeStage`** — a stage has no `output` declaration;
  the validator can't check the next boundary.
- **`MultiOutputComposeStage`** — a stage declares `output T | U`
  (a multi-type sum); composition requires exactly one concrete
  output per stage.
- **`IncompatiblePipeStage`** — stage N+1 has no input slot whose
  type matches stage N's output.
- **`ComposeCycle`** — the chain transitively references the recipe
  being validated. The runtime can't terminate a cycle, so the
  validator rejects it.

## Engine kinds

A composition recipe declares an `engine` kind in its header, but
the value is unused at run time — the inner stages carry their own
engine kinds. Browser-engine stages can run as composition stage 1
(no upstream records to bind) but not as stage 2+ today; the browser
driver doesn't yet accept a pre-seeded record stream.

## Output store

The composition's daemon Run row carries the aggregate emission
counts from the final stage. Inner stage runs are bookkept inside
the composition's run but don't surface as separate `ScheduledRun`
rows. The daemon writes the chain's final snapshot to the output
store keyed under the composition's name.
