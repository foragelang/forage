# The typed hub

How the hub, the recipe language, and Studio fit together once types are
first-class citizens — not just recipe source.

Pairs with `hub-roadmap.md` (the existing program that turns the hub into
a social distribution platform) and `typed-refs-and-emit-binding.md` (in
`plans/`, which lands typed references between records inside one
recipe). This document is the next layer: types as a shareable,
ontology-aligned namespace that recipes are typed functions over.

## Citizens of the hub

The hub stores three kinds of citizen, each independently versioned:

- **Types** — record shapes (`type Product { name: String; sku: String;
  price: Money; … }`), optionally carrying *alignments* to external
  ontologies (schema.org, wikidata, dublin-core, …).
- **Recipes** — typed functions over those types: declared input type,
  declared output type, side effects (HTTP, browser) reified. A recipe
  whose input is `()` is the special case we currently call a "scraper."
- **Pipelines** — saved compositions of recipes joined by type
  alignment.

Hub types are the primary namespace (`@alice/Product`,
`@cannabis-coop/MenuItem`). External ontologies are coordinate systems
any hub type can declare alignments onto. Alignments compose: a hub type
can align to multiple external ontologies at once, and to other hub
types.

## Type alignment

A type-level alignment is a declaration of correspondence between a hub
type and an external term:

```forage
type Product
    aligns schema.org/Product
    aligns wikidata/Q2424752
{
    name:        String   aligns schema.org/name
    sku:         String   aligns schema.org/gtin
    price:       Money    aligns schema.org/offers.price
    description: String?  aligns schema.org/description
    // …
}
```

The surface form: one alignment URI per `aligns` clause, stacked
between the type name and the opening brace at the type level;
optional and one-per-field at the field level. Grammar and validator
rules live in `notes/grammar.md` and `docs/src/lang/types.md`.

Alignments declare correspondence at two granularities:

- **Type-level:** "records of this hub type *are* instances of
  schema.org/Product" — full or partial. Partial is the common case:
  some hub fields have no schema counterpart; some schema fields aren't
  populated.
- **Field-level:** "this field maps to that ontology term." May map to
  multiple terms across ontologies.

The relationship modeled is **equivalence** by default
(`hub:Product.name ≅ schema:name`), with optional flags for
**subtype** (this hub type is *narrower* than the external term, e.g.
`EnhancedJobPosting` is a subtype of `schema:JobPosting`) and
**partial-overlap** (some instances align, some don't — e.g. a `Place`
hub type that's only sometimes a `schema:Restaurant`).

Alignment is not translation. The hub doesn't synthesize values in the
target ontology that aren't present in the source. Alignment is *index*
data, used for discovery, JSON-LD serialization, and federated query —
not a transformation engine.

## Recipes as typed functions

Today a recipe is "fetch a URL, walk the response, `emit` records."
Going forward, every recipe declares its signature:

```forage
recipe ScrapeOpenTable {
    input  ()
    emits  Restaurant
    // …
}

recipe EnrichWithWikidata {
    input  MusicGroup
    emits  MusicGroup
    // …
}

recipe MergeBy<T, K> {
    input  [T]
    emits  [T]
    key    K
    // …
}
```

- `input ()` is the scraper case — no caller-supplied input.
- `input T` is the enricher / transformer case — operates on records of
  type `T` (locally produced or from another recipe).
- HTTP / browser side effects are reified in the body, not in the
  signature. The signature is the contract; the body is the
  implementation.
- A recipe may emit multiple types; in that case `emits` is a sum
  (`emits Product | Variant | PriceObservation`) and the hub indexes
  it under each component.

The recipe's signature is what the hub indexes. The body is what the
runtime executes.

## The hub is queryable, not just browseable

Discovery's primary verb is "find me a recipe that takes X and produces
Y." Keyword search is a fallback for when type-shaped search returns
too many or too few results.

- `producers_of(T)` — every recipe whose emitted types align with `T`.
- `consumers_of(T)` — every recipe whose `input` aligns with `T`.
- `aligned_with(ontology_term)` — every hub type that declares an
  alignment to the given external term.
- `members_of(T)` — every record (across recipe runs) whose type aligns
  with `T`, if the user has materialized the run.

These four queries also power LLM-agent tool selection: the agent
browses the hub by output type and assembles a pipeline by type
matching.

## What this enables

**Type-aligned joins.** OpenTable as `@me/Restaurant ≅
schema:Restaurant`; NYC health inspections as
`@nyc/InspectionResult ≅ schema:Restaurant`. `forage join opentable
nyc-health on address` is one line — the join condition is a field two
recipes' outputs share via alignment.

**Enrichment recipes as typed functions.** `EnrichWithWikidata` takes
any `MusicGroup` (locally produced or another recipe's output) and
returns the same shape augmented with members, genre, founded date.
Pipelines line up by type: `bandcamp | enrich-wikidata | enrich-lastfm
| csv`.

**Discovery by output shape.** "Every recipe whose output aligns with
schema:Event and carries a `location`" is a one-query browse. Same
query gives an LLM agent its toolbox.

**Cross-source materialized records.** Three retailers' scrapers each
emit `Product` aligned to `schema:Product` with `gtin` as the
schema-mapped identity field. The hub merges on `gtin` into one
materialized record carrying three prices, accreting price history
across runs.

**Wikidata reconciliation for free.** Once a hub type aligns with
schema.org, its records can be reconciled against wikidata using the
already-aligned identity fields. Scraped "Stripe" resolves to
`Q24851740` and inherits founder / HQ / headcount / funding rounds
without a separate scrape.

**Type extension as community contribution.** Anyone can publish
`@alice/EnhancedJobPosting extends schema:JobPosting + {salaryMin,
salaryMax, remoteOk}`. Any recipe emitting plain `JobPosting` can be
lifted by a thin adapter recipe (`input JobPosting → output
EnhancedJobPosting`). The type itself is the contribution.

## Design commitments

These are the structural decisions the vision rests on. Each is an
invariant; violating any of them collapses the model back into
"GitHub-for-scrapers."

**The DSL is a composition language first, a scraper second.** Recipes
are typed functions: declared input type, declared output type, side
effects reified. The current `recipe = scrape this URL` shape is the
degenerate case where `input = ()`. Adding `input T` is not a feature
bolted onto a scraper DSL — it's the general form that the scraper case
specializes.

**The hub is queryable by type, not just browseable by name.**
`producers_of(T)` / `consumers_of(T)` / `aligned_with(term)` are the
primary discovery surface. Keyword + categories are fallbacks. This
shapes the hub-api storage: published packages are indexed by every
type they reference (as input, as output, as aligned-to), not just by
slug.

**Notebook-to-hub is one command.** If playing in Studio *is*
publishing to the hub, any friction kills the flywheel. The pipeline
you assembled in Studio this afternoon lands as a hub recipe (or hub
pipeline) before you close the tab — no separate "publishing workflow,"
no manifest editing as a distinct step, no schema massaging.

**Same recipe, two modes.** Dev (sampled inputs, cached HTTP, top-N
preview) and prod (full, persisted, scheduled) are flags at invocation,
not separate recipes. The moment a user forks "playground recipe" vs
"production recipe" to get different sampling behavior, the duality
collapses back into two systems and the contribution flywheel dies.
Sampling, caching, replay are runtime modes, not language constructs.

**Types version independently of recipes.** A recipe pins the type
versions it consumes and produces; types evolve on their own cadence
(new alignments, tightened fields). Otherwise every refinement of
`@me/Product` is a breaking change to N recipes and nobody touches the
types. Type versions are first-class in the hub-api alongside recipe
versions.

**Alignment is index data, not translation.** The hub knows that
`@alice/Product` aligns with `schema:Product`. It does not synthesize
schema.org field values that aren't present in the source recipe's
output. JSON-LD serialization uses the alignments to write `@context`
and rename fields; it doesn't fabricate. Semantic translation between
ontologies is out of scope — it's a research problem.

**Greenfield, same as the rest of forage.** No alignment-compat shims
when an external ontology evolves; bump the alignment version and
update consumers. No `#[serde(default)]` softening when a type's shape
changes. Pre-1.0 — break and move.

## Out of scope (for this layer)

- **Cross-ontology semantic translation.** "Translate this
  `schema:Restaurant` to this Wikidata entity" is a research problem;
  the hub indexes alignments but does not perform automatic transforms
  across them.
- **Cross-record entity resolution as a hub feature.** Merging
  `Product` records across retailers by `gtin` is fine when the
  identity field is well-defined; fuzzy entity resolution
  ("`Restaurant("Joe's Pizza, Brooklyn")` and `Restaurant("Joe's
  Pizza")` are the same place") is not part of the hub.
- **Hub-side execution.** The hub continues to host *replay* against
  fixtures (per `hub-roadmap.md`); it does not run live HTTP, does not
  schedule recipes for users, does not host a query engine over a
  central data store. Studio + local daemons execute; the hub
  distributes.
- **A centralized typed datastore.** The hub is a registry of types,
  recipes, pipelines, and per-version artifacts (fixtures, snapshots).
  It is not a federated database with a query layer; cross-recipe
  joins materialize *locally* (or in Studio) by pulling the relevant
  recipes' outputs.
- **Streaming / live types.** Type versions are discrete published
  artifacts. No live-edit propagation across hub consumers.
- **Mandatory alignment.** A hub type can ship with no external
  alignment and still be useful as a contribution. Alignment is opt-in;
  the discovery / JSON-LD / reconciliation features only kick in for
  the types that opt in.

## Why this matters (one paragraph)

Recipes stop being one-off scrapers and become composable functions
over a shared typed data plane. The "fun" mode (mashing recipes
together in a notebook) and the production mode (a pipeline that runs
nightly) are the same mechanism — playing in the hub IS using the
system correctly, which is what makes the contribution flywheel turn.
Types are the unit that carries the ecosystem: any user who publishes
a well-aligned type makes every recipe in that domain more discoverable
and more interoperable, with no further work on their part.
