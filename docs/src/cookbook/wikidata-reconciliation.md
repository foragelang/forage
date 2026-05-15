# Wikidata reconciliation — enrich aligned records without scraping

Once a hub type carries a `wikidata/<Q…>` alignment and one of its
fields is a Wikidata Q-ID, you can fill in the rest from
`Special:EntityData` instead of scraping the source again. The
`wikidataEntity(qid)` built-in is the bridge.

## The transform

`wikidataEntity(qid)` issues `GET
https://www.wikidata.org/wiki/Special:EntityData/<qid>.json` through
the engine's transport, parses the response, and returns a flattened
record:

```text
{
    _qid:          "Q24851740",
    _label:        "Stripe",
    _description:  "American payments company",
    P112:          "Q5111731",            // founder
    P159:          "Q62",                 // headquarters location
    P571:          "+2010-01-01T00:00:00Z",  // inception
    P1128:         "+8100",               // employees (quantity amount)
    // …one field per named claim on the entity…
    _raw:          { /* untouched entity payload */ }
}
```

Each claim is collapsed to its first value's string form:

- `wikibase-item` / `wikibase-property` → the bare Q-ID or P-ID string.
- `time` → ISO-8601 string from `time`.
- `quantity` → signed `amount` string (e.g. `"+8100"`).
- `string` / `external-id` / `url` / `commonsMedia` / `math` → the
  value as-is.
- `monolingualtext` → just the `text`.
- `globe-coordinate` → `"<lat>,<lon>"`.
- A claim asserted as `novalue` collapses to `null`.

Recipes that need the deeply-nested structure read it from `_raw`.

## The adapter recipe pattern

Combine `wikidataEntity` with the type-extension shape: declare a
child type that adds the fields reconciliation can populate, then
write a `input [Parent] -> output Child` recipe that fills those
fields from the Wikidata entity.

```forage
recipe "wikidata-reconcile"
engine http
emits EnrichedCompany

share type Company
    aligns wikidata/Q4830453
{
    id:         String
    name:       String aligns schema.org/name
    wikidataId: String aligns wikidata/identifier
}

share type EnrichedCompany extends Company@v1
    aligns wikidata/Q4830453
{
    founder:               String? aligns wikidata/P112
    headquartersLocation:  String? aligns wikidata/P159
    inception:             String? aligns wikidata/P571
    employees:             String? aligns wikidata/P1128
}

input companies: [Company]

for $c in $input.companies[*] {
    emit EnrichedCompany {
        id                    ← $c.id
        name                  ← $c.name
        wikidataId            ← $c.wikidataId
        founder               ← wikidataEntity($c.wikidataId) | getField("P112")
        headquartersLocation  ← wikidataEntity($c.wikidataId) | getField("P159")
        inception             ← wikidataEntity($c.wikidataId) | getField("P571")
        employees             ← wikidataEntity($c.wikidataId) | getField("P1128")
    }
}
```

The grammar reserves post-fix `.field` access on a call result for
indexing (`call(args)[expr]`), so claim extraction goes through the
`| getField("P…")` pipe — the syntactic shape stays uniform whether
you're walking a JSON response or projecting a claim.

## Composing with an upstream scraper

A scraping recipe that emits `Company` records can be lifted onto
`EnrichedCompany` by composing with the reconciler:

```forage
recipe "scrape-and-enrich"
engine http
emits EnrichedCompany

compose "scrape-companies" | "wikidata-reconcile"
```

`scrape-companies` produces `Company` records with `wikidataId` set
from the source. `wikidata-reconcile` is the typed function
`[Company] -> EnrichedCompany` from above. Composition validates
because the output / input types line up.

## Replay

`wikidataEntity` issues its fetch through the engine's transport, so
`forage record` captures the wikidata responses alongside whatever
other HTTP traffic the run produced, and `forage run --replay
<fixtures>` replays them. No special-casing for transport-aware
transforms — they ride the same pipe as every other GET.

## What's out of scope

- **Cross-ontology translation.** Reconciliation enriches an
  already-aligned record from its target ontology's authoritative
  source. Translating a `schema:Restaurant` record into a Wikidata
  entity shape is a research problem and stays out.
- **SPARQL.** This bridges the `Special:EntityData` REST endpoint
  only. Full Wikidata query is out of scope.
- **Claim label translation.** The transform returns claim values
  keyed by P-ID. Recipe authors handle the P-ID → human label mapping
  themselves (typically by declaring the field with a meaningful name
  and an `aligns wikidata/P…` clause).
