# Composition

A recipe's body is one of two kinds: a scraping body (the historical
shape — `step`, `for`, `emit`) or a **composition body** — a chain of
recipe references joined by `|`. The runtime walks the chain, feeding
each stage's emitted records to the next.

A composed recipe **is itself a recipe**. It declares the same
`recipe "<name>"` header, can carry the same optional `emits`
clause, and publishes under `/v1/recipes/` like any scraping recipe.
There is no separate "pipeline" citizen: one citizen (Recipe), two
body kinds.

```forage
recipe "enriched-products"
engine http
emits Product

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

- A must emit exactly one type. The validator resolves the type from
  A's declared `emits T` when the source supplies one, otherwise from
  the body's `emit X { … }` statements. Either way, A's resolved
  output set must have exactly one element — multi-type composition
  is a future extension.
- B must declare exactly one input slot whose type matches A's
  resolved output:
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

emits Product

for $p in $input.prior {
    emit Product { id ← $p.id }
}
```

## Composition is closed under composition

A composition is itself a recipe with a resolvable output type, so it
can appear as a stage in another composition. Nested composition is
the common case once recipe authors start sharing pipelines:

```forage
recipe "ab-c"
engine http
emits Product

compose "ab" | "c"
```

where `"ab"` is itself a composed recipe `compose "a" | "b"`. The
linker walks the chain at validate time, freezing every reachable
stage into the deployed module's closure; the runtime then traverses
that closure without consulting other deployments.

## Resolution happens at validate / link time

Composition stage references are resolved when validation runs (the
linker walks the workspace, pulls each stage's parsed recipe, and
folds the full closure into a `LinkedModule`). The runtime consumes
the linked module and never does name resolution at run time. As a
result, redeploying a downstream stage after a composition recipe was
itself deployed does **not** change the composition's behavior — the
composition's deployed module already pinned every transitively
referenced stage at the version that was current at composition-
deploy time.

## Validation

The validator's composition rules:

- **`UnknownComposeStage`** — a bare-name stage isn't a recipe in
  the local workspace.
- **`HubDepStageUnsupported`** — the stage uses the
  `@author/name` hub-dep form; the runtime can't resolve those today,
  so the validator rejects them. Sync the upstream into your workspace
  and reference it by bare name.
- **`EmptyComposeStage`** — a stage emits no types: it neither
  declares `emits` nor carries any `emit X { … }` statements in its
  body, so there's nothing for the downstream input to bind to.
- **`MultiTypeComposeStage`** — a stage emits more than one type
  (declared `emits T | U | …` or multiple distinct body emits);
  composition requires exactly one concrete output per stage.
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
store keyed under the composition's name. The output schema comes
from the composition's terminal stage emits, already resolved in the
deployed `LinkedModule`.
