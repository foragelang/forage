# Scraping DSL — design plan

We're replacing the bespoke per-platform Swift scrapers (`SweedScraper`, `LeafbridgeScraper`) with a declarative DSL that describes "how to scrape a site into our typed output." Recipes load at runtime from a directory; primary authoring path is "show the DSL spec + reference recipes to Claude, give it the dispensary URL, get back a recipe." Hand-authoring is supported. Designed so recipes can be shared and reviewed on a public hub.

## Why a DSL (vs. a Swift `Scraper` protocol or scripted plugins)

- **Swift protocol**: every new platform needs a recompile. Doesn't fit "user adds a store the app doesn't yet support."
- **Scripted plugins (JS via JavaScriptCore, etc.)**: full expressiveness, but runs untrusted model output verbatim with HTTP access; failure modes are silent ("ran but produced garbage"); LLM gets creative when it should stay inside well-trodden patterns; every scraper is a snowflake with its own bespoke pagination/extraction code.
- **Declarative DSL + bounded engine**: the engine is the only thing that runs HTTP, parses responses, writes the DB. Recipes are pure data describing what to do. This bounds blast radius (auditable), keeps the LLM inside known patterns, makes failures schema-checkable, and gives us a single place to fix bugs that affect every recipe.

When a recipe needs something the DSL can't express, the answer is to extend the DSL — add a new pagination kind, auth strategy, or transform to the engine in Swift. Vocabulary grows by deliberate increments, not by per-recipe improvisation.

## Authoring model

- **Custom syntax, not YAML/JSON.** Modern LLMs handle a published grammar + 2–3 worked examples + a validator-in-the-loop fine. Strictly-typed-for-our-domain wins on review experience: comments inline, multiline strings without escaping, parser errors that speak our domain, no general-format footguns to defend against.
- Hand-rolled PEG parser in Swift. Domain-specific error messages. Tree-sitter grammar is a follow-on if/when we want GitHub syntax highlighting on the hub.
- Reference recipes (Sweed, Leafbridge) ship in-tree as the canonical exemplars.

## Output type catalog (fixed by the engine)

Recipes don't declare types. They target a fixed catalog matching `Schema.swift`. This is project-specific, not a generic scraping framework — explicit non-goal. If we later want generic, we refactor.

```
Dispensary       { slug, name, platform, storeId?, address?, latitude?, longitude?,
                   phone?, website?, timezone? }
Category         { externalId, name }
Product          { externalId, name, description?, brand?, strain?, strainPrevalence?,
                   productType?, category, subcategoryExternalId?, subcategoryName?,
                   terpenes: [String], images: [String], variants: [Variant] }
Variant          { externalId, name?, sku?, sizeValue?, sizeUnit? }
PriceObservation { variant, menuType, price?, promoPrice?, availableQty?,
                   thcPct?, cbdPct?, terpenePct?, stockType?, availability?, promoCount? }
```

The DSL's type system is just enough for this: structs-with-optionals, lists, scalars, plus a tagged union `MenuType = RECREATIONAL | MEDICAL`. No generics.

## Recipe shape

A recipe has three sections (semantic shape; syntax is a follow-on design exercise).

### 1. HTTP graph

A small DAG of HTTP steps with named outputs.

- Each step: method, URL (templated), headers, body (JSON, form-encoded, or none), output binding name.
- Steps can reference earlier-step outputs in templates / bodies / headers via `{{name.path}}`.
- Cookie session is implicit and shared across steps.
- Per-recipe rate-limit + retry policy. Defaults match `notes/legal.md`: ~1 req/sec, exponential backoff on 429/5xx, honest UA.

The two existing platforms exercise the full graph capability:

- **Sweed**: `auth = static_header(storeId)` → `categories = POST .../GetProductCategoryList` → for each `(category, menu)` pair: `POST .../GetProductList { paginate: pageWithTotal }`.
- **Leafbridge**: `prime = GET menuPageURL` (extract nonce + acquire `__cf_bm` cookie via regex on response body) → for each `menu`: `POST .../admin-ajax.php { paginate: untilEmpty }`.

Both natural cross-products (categories × menus, single-axis menu iteration) need to fall out of the iteration model.

### 2. Pagination strategies

Three logical categories, two engine primitives. The third strategy is a `mode` flag of the second since both depend on observing a natural in-flight request from the page and only differ in *how* they trigger subsequent batches.

**`httpPaginate`** — recipe knows the request shape up front; engine bumps a param and stops on a termination condition. Variants (Swift-side enum, recipe picks one):

- `pageWithTotal`: send page-number param, response carries `items` + `total`, stop when accumulated ≥ total. Sweed.
- `untilEmpty`: send page-number param, response carries `items`, stop when items shorter than page-size or empty. Leafbridge.
- `cursor` (future): server returns a continuation token in each response that gets sent back on the next request. Add when we encounter one.

Each variant declares which response path holds `items` / `total` / `cursor`, and which request param encodes the page.

**`browserPaginate`** — recipe doesn't construct paginated requests itself; the engine observes a natural in-flight request from the page (via the same fetch/XHR wrapper used for first-page capture) and drives more from there. The first matching capture is the seed; subsequent batches come from one of two modes:

- `mode: "scroll"` — after the seed, dispatch scroll events on the rendered page on intervals. The SPA fires its own next-page request using its own auth tokens, and the engine's wrapper captures it. Cheap to author (no per-platform request shape required). Slowest per scrape.
- `mode: "replay"` — after the seed, take its request body / headers as a template, apply per-iteration `override` params (`{ page: $i }` etc.), and re-fire via the page's own fetch (so Origin, cookies, CSP, session tokens all match). Faster per scrape. Requires knowing which request param controls pagination, but inherits all auth from the page so we never construct credentials ourselves.

Both `browserPaginate` modes share:

- `observe: <url-pattern>` — which request matters (e.g. `dmerch.iheartjane.com/v2/multi`)
- `until: <condition>` — termination: `count >= nb_hits`, `no_progress_for: N`, `max_iterations: N`

Conceptually:

```
browserPaginate {
  observe:  "dmerch.iheartjane.com/v2/multi"
  mode:     "scroll"                // or "replay"
  override: { page: $i }            // replay-mode only
  until:    { count_full_products >= placements[*].nb_hits }
}
```

**Anti-pattern (forbidden)**: overriding the natural batch-size param (e.g. `page_size: 60 → 2000`) to bypass pagination by demanding a huge response in one shot. See `CLAUDE.md` *"Don't ask servers for things real clients wouldn't."* Drive the natural pagination instead.

New strategies and modes are Swift-side additions. The DSL doesn't grow primitives ad-hoc; recipes pick from the named set, and we extend the set when a real platform demands it.

#### Engine status (validated against Trilogy/Jane, 93% coverage)

`scripts/probe.swift` carries a working `BrowserPaginate` implementation. Both modes built and tested:

- **`scroll` mode** is "drive the SPA forward" — each iteration scrolls window + every nested shadow-DOM scrollable to the bottom AND clicks the bottom-most visible button labeled `"View more"` / `"Show more"` / `"Shop all products"` / `"Load more"` / `"View all"` / `"See more"` (case-insensitive exact match, position-disambiguated by highest viewport `top`). Click-driven sites use this; scroll-driven sites also use this; both are cheap to do together.
- **`replay` mode** forks a captured seed request via the page's `window.fetch`, applies dotted-path overrides with `$i` substitution. Mechanically wired; CORS-blocked on Jane (CSP `connect-src` likely refuses `evaluateJavaScript`-injected fetches even with a faithful seed body and real session IDs). Worth revisiting if a future platform requires it as the only path.

**Validated on Trilogy** (Jane platform, ~1068 products):

| Aspect | Result |
|---|---|
| Cover the full menu | **1000 / 1068 = 93%** in one run with `scroll` mode |
| Shape of the right loop | URL = `/shop/<menu-slug>/menu/all` (recipe input); gesture = click bottom-most "View more"; `observe = iheartjane.com/v2/smartpage` (the resolver endpoint that returns rich data per click) |
| Termination | 17 iterations: 16 with progress, 3 idle. `noProgressLimit: 3` was the right default |

Three concrete lessons that feed back into DSL design:

1. **`observe` is platform-specific.** A recipe watching the wrong endpoint will see `no_progress` on every iteration even though clicks are firing successfully. The recipe author needs to know which endpoint signals "next page arrived" — for Jane that's `/v2/smartpage` (resolver), not `/v2/multi` (initial). The diagnostic loop is: run, see `no_progress`, inspect the captured URL list, identify the right pattern.
2. **The recipe needs to land on the right view.** Trilogy's `/shop/adult-use-menu/` is a curated homepage; `/menu/all` is the full paginatable list. Recipes need a `navigateTo` or `clickButton` step before paginate to put the SPA in the right state. Hardcoded direct URL works for Trilogy; other Jane sites likely have the same `/menu/all` convention.
3. **Click disambiguation matters.** Multiple buttons can share the same label (banner "View more" vs pagination "View more"). The pagination one is below the content it paginates → bottom-most match is the right disambiguator. Bake into the engine; recipe authors don't need to think about it unless a site puts the load-more button somewhere weird.

**Diagnostic affordances dump.** When `BrowserPaginate` finishes (success or stall), the probe writes `probe-affordances.json` next to the captures — every visible button / link / role=button / scrollable with text + selector + position. This is the platform's debug artifact: when a recipe doesn't reach expected coverage, the recipe author (human or LLM) reads this file to see what UI affordances the engine *didn't* interact with. The vocabulary in the dump matches the vocabulary recipes use (button text, selector, position) so the corrective recipe edit is a direct reading of the dump.

### 3. Type construction

The body of the recipe binds output type fields to extraction expressions over the fetched data. Mental model: Elm decoders / Swift `Codable` / GraphQL — declare the target shape, bind each field to an expression. Optionals make missing data graceful. Lists come from paginated fetches.

Conceptually (shape, not committed syntax):

```
product {
  externalId   ← $.id
  name         ← $.name
  description  ← $.description
  brand        ← $.brand?.name
  strain       ← $.strain?.name
  strainPrevalence ← $.strain?.prevalence?.name | prevalenceNormalize
  productType  ← $.productType?.name
  terpenes     ← $.strain?.terpenes[*].name
  images       ← $.images[*]
  variants     ← $.variants[*] | map(variant)
}

variant {
  externalId   ← $.id
  name         ← $.name
  sku          ← $.sku
  sizeValue    ← $.unitSize.value | normalizeToGrams($.unitSize.unitAbbr)
  sizeUnit     ← $.unitSize.unitAbbr | normalizeUnitToGrams
}

priceObservation(menu = $menu) {
  price         ← $.price
  promoPrice    ← $.promoPrice
  availableQty  ← $.availableQty
  thcPct        ← $.labTests?.thc?.value[0]
  cbdPct        ← $.labTests?.cbd?.value[0]
  terpenePct   ← $.labTests?.terpenes?.value[0]
  stockType     ← $.stockType
  availability  ← $.orderingAvailability?.reason
  promoCount    ← $.promos | length
}
```

#### Transforms (fixed vocabulary)

Initial set — covers everything the existing two scrapers need:

- `parseFloat`, `parseInt`
- `regexExtract(pattern, group?)` — returns capture group or full match
- `parseSize` — parses `"3.5g"`, `"1oz"`, `"100mg"` → `(value, unit)` tuple
- `normalizeToGrams` — `(value, "OZ")` → `(value × 28, "G")`; pass-through otherwise. Cannabis-standard.
- `prevalenceNormalize` — `INDICA` / `Indica` → `Indica`; `NOT_APPLICABLE` → null
- `coalesce(a, b, ...)` — first non-null
- `default(value)` — substitute when null
- `lower`, `upper`, `capitalize`, `trim`
- `length` (for arrays)

New transforms are added to the engine as new platforms surface them. The DSL doesn't grow expression syntax; it picks from named functions.

#### Conditional construction

Some fields branch on dimension values (Leafbridge price field varies by `menuType`). Handled via the iteration variable being available in extraction expressions:

```
priceObservation(menu = $menu) {
  price        ← case $menu of {
                   MEDICAL       → $.priceMed
                   RECREATIONAL  → $.priceRec
                 }
  promoPrice   ← case $menu of {
                   MEDICAL       → coalesce($.specialPriceMed, $.priceMed)
                   RECREATIONAL  → coalesce($.specialPriceRec, $.priceRec)
                 }
}
```

## Test/dev workflow

A recipe is a directory:

```
recipes/
  sweed/
    recipe.scrape         # the DSL file
    fixtures/
      categories-rec.json
      products-flower-rec-1.json
      products-flower-rec-2.json
      ...
    snapshot.yaml          # serialized typed records the recipe produces
```

CLI commands:

- **`weed-prices test recipes/sweed/`** — replay mode. HTTP intercepted, served from `fixtures/`. Engine constructs records, diffs against `snapshot.yaml`. Sub-second feedback.
- **`weed-prices test recipes/sweed/ --update`** — accept current output as the new snapshot. Run after intentional recipe changes.
- **`weed-prices test recipes/sweed/ --refresh`** — record mode. Hits live site, overwrites fixtures with fresh responses, then re-runs in replay mode and shows the record diff. The "site changed, repair the recipe" workflow. See "Fixture refresh" below.

The same engine runs in three modes:

- **Replay** (default during dev): HTTP backed by fixture files.
- **Record** (`--refresh`): HTTP hits live; responses captured to `fixtures/`.
- **Live** (production / app's Update button): HTTP hits live; responses not saved; records written to the SQLite DB via the existing `AppDatabase` write path.

### Fixture refresh — the drift repair flow

Sites change shape over time. Local tests pass against stale fixtures, masking the drift. `--refresh` is the one-command repair:

1. Engine runs the recipe's full HTTP graph against the live site.
2. Each response is saved into `fixtures/`, replacing the prior captures.
3. Engine re-runs in replay mode against the fresh fixtures.
4. Three diffs are surfaced:
   - **Wire diff** (old fixtures vs. new): raw response changes.
   - **Record diff** (snapshot vs. new output): does extraction still produce the right records?
   - **Schema warnings**: any newly-violated type constraints.

Outcomes:

- Wire changed, records identical → cosmetic site change. Commit fixtures, no recipe edit.
- Wire changed, records broken → fix the recipe, iterate in replay mode (no more network calls), run `--update` once correct.
- Wire changed, records semantically different but valid → decide whether to accept with `--update`.

The engine knows the exact HTTP graph the recipe produces. Manual fixture re-capture would require reconstructing every URL, header, body, and pagination loop by hand — first-class `--refresh` collapses that to one command.

## Engine responsibilities

Swift code, single execution path used by the app, the CLI, and the test runner:

- **Parse** recipe DSL files (hand-rolled PEG). Domain-specific error messages with line/column.
- **Validate** against the type catalog: every required field bound, every type produced, no unknown transforms / pagination kinds / auth strategies.
- **Execute** the HTTP graph in replay / record / live modes.
- **Apply** rate limiting, polite retry on 429/5xx, honest UA per `notes/legal.md`.
- **Apply** pagination strategies and iteration.
- **Apply** transforms.
- **Validate** constructed records (required fields present, unions only contain declared variants, types match).
- In **live mode**: open snapshot row, write upserts via the existing schema, finalize with counts. Same DB writes the current scrapers do — none of that logic lives in recipes.

## Recipe loading

- App ships with a bundled set of canonical recipes (Sweed, Leafbridge — ports of the current scrapers).
- App reads additional recipes at startup from `~/Library/Application Support/weed-prices/recipes/`.
- `Dispensaries.swift` config remains: each dispensary names a recipe + supplies per-store params (`storeId`, `menuPageURL`, `priceCategoryIds`, etc.). Per-store config is data; recipe is shared across all stores on that platform.

## Migration of existing scrapers

Greenfield (`CLAUDE.md`): port Sweed and Leafbridge to recipes, delete the bespoke Swift scrapers entirely. No compat shim, no "keep them around for fallback." If a port surfaces something the DSL can't express, that's a signal to extend the DSL — add the missing primitive in Swift, complete the port, ship.

The two existing scrapers are the acid test: if the DSL can't express both naturally, the design is wrong.

## Hub (downstream, not part of this build)

The recipe + fixtures + snapshot directory is self-contained and shareable. A reviewer reads:

- The recipe — what HTTP it'll hit, how extraction maps to types.
- The fixtures — actual saved responses they can verify look plausible.
- The snapshot — the records that come out, structurally diffable in any PR review.

The DSL's review properties (comments, multiline strings, declarative-only, fixed type catalog) are what make hub review meaningful. The hub itself isn't part of this plan, but the design choices above are aimed at supporting it cleanly.

## Out of scope (explicitly)

- **Generic scraping framework.** Output types are fixed to our schema. If we ever want generic, refactor at that time.
- **Substantive access controls.** Login walls, paywalls, real CAPTCHAs that don't auto-clear, account-required pages — recipes don't bypass these per `notes/legal.md` rule 5. Generic bot-management gates on otherwise-public pages (Cloudflare CF-Challenge etc.) are *not* in this category and are cleared by the browser-engine recipe kind via `WKWebView` — see `notes/jane-platform.md` for a worked example.
- **Recipe sharing UI in the app.** Users curate the local recipes directory; a hub is downstream.
- **Backwards compatibility for old recipes.** Greenfield: when the DSL changes, all in-tree recipes get updated.
- **Per-recipe target type extensions.** Recipes target the fixed catalog. New types require an engine + schema change.
