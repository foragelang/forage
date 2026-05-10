# Forage execution plans

Concrete plan for each roadmap phase. Files, types, methods, tests, acceptance criteria. Anti-patterns to avoid (greenfield: delete don't migrate; no shimming).

The work happens in a single forage worktree branched off `main` as `runtime`. Phases A–F land there; weed-prices integration (G–H) happens in a separate branch in the consumer repo. Each phase = one commit on the branch.

---

## Phase A — Syntax design by example

**Goal:** Hand-write `.forage` files for Sweed / Leafbridge / Jane that demonstrate every syntactic construct the parser will need to handle. No code; this is design-on-paper.

**Files**
- `recipes/examples/sweed-zen-leaf.forage`
- `recipes/examples/leafbridge-remedy.forage`
- `recipes/examples/jane-trilogy.forage`

**Constructs each must demonstrate**
- `recipe "<name>" { … }` block, top-level keywords
- `engine http` vs `engine browser`
- `type Foo { name: String; brand: String?; variants: [Variant] }` with `String|Int|Double|Bool|<Type>|[<Type>]?` field types
- `input storeId: String` declarations (consumer-supplied)
- `enum MenuType { RECREATIONAL, MEDICAL }` declarations (recipe-level enums)
- HTTP graph: `step <name> { method "POST"; url "..."; headers { "X": "Y" }; body.json { … } }` and `body.form { … }`
- Auth: `auth.staticHeader { name: "storeId", value: $input.storeId }` and `auth.htmlPrime { step: prime; extract: regex(...) }`
- Pagination: `paginate pageWithTotal { items: $.list; total: $.total; pageParam: "page"; pageSize: 200 }` and `paginate untilEmpty { items: $.data.products_list; pageParam: "prods_pageNumber" }`
- Iteration: `for $cat in categories { … }` (collection-iteration) and `for $menu in [RECREATIONAL, MEDICAL] { … }` (literal-iteration)
- Extraction expressions: `$.id`, `$.brand?.name`, `$.images[*]`, `$.variants[*]`, `$cat.id`, `$input.storeId`
- Transform pipeline: `$.option | parseSize | normalizeToGrams`
- Map-into-record: `$.variants[*] | map(variant)` where `variant` is another emit block
- Conditional construction: `case $menu of { MEDICAL → $.priceMed; RECREATIONAL → $.priceRec }`
- Coalesce: `coalesce($.specialPriceMed, $.priceMed)` (function-call form)
- Browser config: `browser { initialURL: $input.menuPageURL; ageGate.autoFill { dob: 1990-01-01 }; warmupClicks: ["All products"]; observe: "iheartjane.com/v2/smartpage"; paginate browserPaginate.scroll { until: noProgressFor(3) } }`
- `emit Product { name ← $.name; brand ← $.brand?.name | titleCase; variants ← $.variants[*] | map(variant) }`
- Comments: `//` line and `/* … */` block

**Acceptance:** all three files read like prose to a recipe author who's seen the design plan. Constructs nest cleanly; no ambiguity in precedence between `|`, `←`, `?.`, `[*]`. The grammar to implement in Phase C is a direct reading of these files.

**Anti-patterns**
- Don't invent constructs we don't need yet. Don't add module imports, function definitions, generic types, or anything that isn't load-bearing for the four platforms we have.
- Don't pre-design for "future flexibility" — keep every keyword tied to a real use.

---

## Phase B — `Recipe` value type + `HTTPEngine` runtime

**Goal:** Build the runtime that takes a Recipe value and produces a Snapshot. Construct Recipe by hand (no parser yet). Validate against captured Sweed and Leafbridge fixtures.

**Files**
- `Sources/Forage/Recipe/Recipe.swift` — top-level value
- `Sources/Forage/Recipe/RecipeType.swift` — recipe-declared types + enums
- `Sources/Forage/Recipe/HTTPGraph.swift` — request graph value
- `Sources/Forage/Recipe/Pagination.swift` — strategies
- `Sources/Forage/Recipe/Auth.swift` — auth strategies
- `Sources/Forage/Recipe/Iteration.swift` — iteration spec
- `Sources/Forage/Recipe/PathExpr.swift` — `$.foo.bar[*]` model
- `Sources/Forage/Recipe/Template.swift` — string interpolation (`"prefix-{$.x}-suffix"`)
- `Sources/Forage/Recipe/Extraction.swift` — emit block + bindings + ExtractionExpr
- `Sources/Forage/Recipe/Transform.swift` — transform call + vocabulary registry
- `Sources/Forage/Engine/HTTPClient.swift` — polite HTTP (rate limit, retry, honest UA)
- `Sources/Forage/Engine/HTTPEngine.swift` — orchestrator
- `Sources/Forage/Engine/RecipeRunner.swift` — top-level entry point
- `Sources/Forage/Engine/Scope.swift` — variable resolution
- `Sources/Forage/Engine/PathResolver.swift` — evaluate PathExpr against TypedValue / JSON
- `Sources/Forage/Engine/TransformImpls.swift` — built-in transforms
- `Sources/Forage/Engine/JSONValue.swift` — internal JSON-shaped value (decoded from response data, not the same as TypedValue which is the recipe's emit-output type)
- `Tests/ForageTests/HTTPEngineTests.swift`
- `Tests/ForageTests/Fixtures/sweed/categories.json`
- `Tests/ForageTests/Fixtures/sweed/products-flower-rec.json`
- `Tests/ForageTests/Fixtures/sweed/products-vapes-rec.json`
- `Tests/ForageTests/Fixtures/leafbridge/prime.html`
- `Tests/ForageTests/Fixtures/leafbridge/products-rec-page1.json`
- `Tests/ForageTests/Fixtures/leafbridge/products-rec-page2.json`

**Recipe types (Swift)** — see PLANS.md commit history for the full type tree; key shapes:

```swift
public struct Recipe: Sendable {
    public let name: String
    public let engineKind: EngineKind     // .http | .browser
    public let types: [RecipeType]
    public let enums: [RecipeEnum]
    public let inputs: [InputDecl]
    public let httpGraph: HTTPGraph?
    public let browser: BrowserConfig?
    public let emissions: [Emission]
}

public struct RecipeType: Sendable {
    public let name: String
    public let fields: [RecipeField]
}
public struct RecipeField: Sendable {
    public let name: String
    public let type: FieldType
    public let optional: Bool
}
public indirect enum FieldType: Sendable {
    case string, int, double, bool
    case array(FieldType)
    case record(String)   // typeName
    case enumRef(String)  // enum name
}

public struct HTTPGraph: Sendable {
    public let auth: AuthStrategy?
    public let steps: [HTTPStep]
}
public struct HTTPStep: Sendable {
    public let name: String
    public let request: HTTPRequest
    public let pagination: Pagination?
    public let iteration: Iteration?
}
public struct HTTPRequest: Sendable {
    public let method: String
    public let url: Template
    public let headers: [(String, Template)]
    public let body: HTTPBody?
}
public enum HTTPBody: Sendable {
    case jsonObject([(String, BodyValue)])
    case form([(String, Template)])
    case raw(Template)
}
public indirect enum BodyValue: Sendable {
    case template(Template)
    case object([(String, BodyValue)])
    case array([BodyValue])
    case literal(JSONValue)
}

public enum AuthStrategy: Sendable {
    case staticHeader(name: String, value: Template)
    case htmlPrime(stepName: String, extractCookie: Bool, extractToken: PathExpr)  // PathExpr over the HTML response (regex-shaped)
}

public enum Pagination: Sendable {
    case pageWithTotal(itemsPath: PathExpr, totalPath: PathExpr, pageParam: String, pageSize: Int)
    case untilEmpty(itemsPath: PathExpr, pageParam: String)
    case cursor(itemsPath: PathExpr, cursorPath: PathExpr, cursorParam: String)
}

public struct Iteration: Sendable {
    public let variable: String
    public let kind: IterationKind
}
public enum IterationKind: Sendable {
    case overCollection(PathExpr)            // for $cat in categories
    case overLiteralList([JSONValue])         // for $menu in [RECREATIONAL, MEDICAL]
}

public struct Template: Sendable {
    public let parts: [TemplatePart]
}
public enum TemplatePart: Sendable {
    case literal(String)
    case interp(PathExpr)
}

public indirect enum PathExpr: Sendable {
    case current                              // $
    case input(String)                        // $input.storeId
    case variable(String)                     // $cat
    case dot(PathExpr, String)
    case optChain(PathExpr, String)
    case index(PathExpr, Int)
    case wildcard(PathExpr)                   // [*]
}

public struct Emission: Sendable {
    public let typeName: String
    public let bindings: [FieldBinding]
}
public struct FieldBinding: Sendable {
    public let fieldName: String
    public let expr: ExtractionExpr
}
public indirect enum ExtractionExpr: Sendable {
    case path(PathExpr)
    case pipe(ExtractionExpr, [TransformCall])
    case caseOf(scrutinee: PathExpr, branches: [(label: String, expr: ExtractionExpr)])
    case mapTo(PathExpr, emission: Emission)
    case literal(JSONValue)
    case call(name: String, args: [ExtractionExpr])      // e.g. coalesce(a, b)
}
public struct TransformCall: Sendable {
    public let name: String
    public let args: [ExtractionExpr]
}
```

**HTTPEngine algorithm**
1. Validate inputs against `Recipe.inputs` (name + type match).
2. Run auth strategy (e.g. `htmlPrime` step).
3. Walk `httpGraph.steps` in declared order. For each step:
   - Build a request from `request.url`, `request.headers`, `request.body` rendered via the current scope (inputs, prior step results, iteration variables).
   - If `step.iteration` set: enumerate the collection / literal list, run the step body once per value with the iteration variable bound.
   - If `step.pagination` set: loop until termination, collecting items into a list under `$<stepName>`.
   - Else: run once, store response under `$<stepName>`.
4. Walk `recipe.emissions`. Each emission may be inside an `Iteration` (recipe author can declare `for $cat in categories.* { emit Category {...} }` style — TBD in Phase A how this is expressed).
5. For each emit:
   - For each binding: evaluate `expr` against the current scope. Apply transforms in the pipeline. Convert to `TypedValue`.
   - Construct `ScrapedRecord(typeName: emission.typeName, fields: [...])`.
   - Validate field types against the declared `RecipeType`. (Or defer to Phase D's validator.)
   - Append to snapshot's records.
6. Return `Snapshot(records: …, observedAt: Date())`.

**HTTPClient**
- `URLSession.shared`-backed
- Honest UA: `forage/<version> (+see DESIGN.md)`
- 1 req/sec rate limit (per host)
- Exponential backoff on 429/5xx, max 3 retries
- JSON request body encoding
- Form-encoded body encoding (URL-encode keys + values, including bracketed keys like `wizard_data[retailer_id]`)

**Transform vocabulary** (initial):
- `parseFloat`, `parseInt`, `parseBool`
- `regexExtract(pattern, group?)` — returns capture
- `parseSize` — `"3.5g"` → `(3.5, "G")`, `"100mg"` → `(100, "MG")`, `"1oz"` → `(28, "G")` (cannabis-standard ounce → grams)
- `normalizeToGrams(unit)` — pass-through for non-OZ; OZ × 28 → G
- `prevalenceNormalize` — `"INDICA"` / `"Indica"` → `"Indica"`, `"NOT_APPLICABLE"` → null
- `coalesce(a, b, ...)` — first non-null
- `default(value)` — substitute when null
- `lower`, `upper`, `capitalize`, `titleCase`, `trim`
- `length` — for arrays
- `dedup` — for arrays of strings

**Tests**
- Hand-construct a `Recipe` for Sweed (Zen Leaf Elkridge style: storeId 577, two categories) in Swift.
- Set up an in-process `HTTPClient` mock that serves `Tests/ForageTests/Fixtures/sweed/*.json`.
- Run `RecipeRunner.run(recipe:)`.
- Assert the resulting `Snapshot.records(of: "Product")` has the right count and rich fields.

Same for Leafbridge: hand-construct Recipe with `htmlPrime` auth + form-encoded body + `untilEmpty` pagination; verify against fixture.

**Acceptance:** `swift test` passes, both Sweed and Leafbridge recipes produce expected snapshots from fixtures alone (no live network).

**Anti-patterns**
- No partial-step retry logic that hides real errors. If a step fails, the run fails.
- No "default" recipe behavior when fields are missing — emit nil if optional, fail if required.

---

## Phase C — Lexer + parser

**Goal:** `.forage` text → `Recipe` value.

**Files**
- `Sources/Forage/Parser/Token.swift`
- `Sources/Forage/Parser/Lexer.swift`
- `Sources/Forage/Parser/AST.swift` — explicit AST types parallel to but distinct from Recipe types (so the parser can fail readably without partial-recipe weirdness)
- `Sources/Forage/Parser/Parser.swift` — recursive-descent
- `Sources/Forage/Parser/AST2Recipe.swift` — translator
- `Sources/Forage/Parser/ParseError.swift` — line/column + suggestion
- `Tests/ForageTests/LexerTests.swift`
- `Tests/ForageTests/ParserTests.swift`

**Lexer** produces tokens with `Source.Location { line, column }`. Token kinds:
- Keywords: `recipe, engine, http, browser, type, enum, input, step, method, url, headers, body, json, form, raw, auth, staticHeader, htmlPrime, paginate, pageWithTotal, untilEmpty, cursor, for, in, emit, case, of, let, true, false, null, observe, browserPaginate, scroll, replay, ageGate, autoFill, warmupClicks, navigate, until, noProgressFor, maxIterations`
- Identifiers, type names (capitalized identifiers), enum-variant identifiers
- Literals: string (`"..."` with `\` escapes; supports `{$path}` interpolation tokens), int, double, bool (`true`/`false`), `null`, date (`1990-01-01`)
- Operators: `←` (binding), `|` (pipe), `?.` (opt chain), `[*]` (wildcard), `→` (case branch), `..`, `:`, `;`, `,`, `(`, `)`, `{`, `}`, `[`, `]`, `.`, `?`
- Special: `$` prefix for paths/variables, `$input` reserved variable
- Comments: `//` line, `/* */` block (skipped, not tokens)

**Parser** entry `parseRecipe(source:) -> AST.Recipe throws`. Hand-rolled recursive descent. Expression precedence: `|` (lowest, left-assoc) > `?.` > `.` / `[*]` / `[N]`. Inside emit bindings, `←` is a separator not a binary op.

**AST → Recipe translator** does:
- Resolve type-name references (validate type exists in `recipe.types`)
- Resolve transform names (just stringly here; vocabulary check happens in the validator in Phase D)
- Lowercase enum variant references to canonical form
- Convert AST.Template → Recipe.Template (parts only)
- Convert AST.PathExpr → Recipe.PathExpr

**Tests**
- Parse the three Phase A example files; compare resulting `Recipe` to a hand-constructed equivalent (or just assert structural shape: `recipe.types.count == N`, `httpGraph?.steps.count == M`, `emissions.count == K`).
- Parse intentionally malformed inputs; assert the right `ParseError` with line/column.

**Acceptance:** `forage/recipes/examples/*.forage` parse cleanly. `swift test` green.

**Anti-patterns**
- No "best-effort" recovery from parse errors that produces bogus partial recipes. Fail fast, report the error.
- No silent acceptance of unknown keywords — explicit list, anything else is a parse error.

---

## Phase D — Validator + diagnostics + fixture harness

**Goal:** Static recipe validation; structured failure reports; offline test loop with fixtures + snapshots.

**Files**
- `Sources/Forage/Validation/Validator.swift`
- `Sources/Forage/Validation/ValidationError.swift`
- `Sources/Forage/Diagnostics/DiagnosticReport.swift`
- `Sources/Forage/Diagnostics/Expectation.swift` — recipe declares `expect { records.count >= 100 }`
- `Sources/Forage/Fixtures/FixtureStore.swift` — directory layout
- `Sources/Forage/Fixtures/HTTPRecorder.swift` — captures live req/resp into fixtures
- `Sources/Forage/Fixtures/HTTPReplayer.swift` — serves saved responses to HTTPClient
- `Sources/Forage/Fixtures/SnapshotSerializer.swift` — Codable for Snapshot/ScrapedRecord/TypedValue (via custom encoder/decoder)
- `Sources/forage-cli/main.swift` — `forage test <dir>`, `forage test <dir> --refresh`, `forage validate <dir>`
- `Tests/ForageTests/ValidatorTests.swift`
- `Tests/ForageTests/SnapshotSerializerTests.swift`

**Validator checks**
1. Every `RecipeType.fields[i].type.record(name)` references a declared `RecipeType`.
2. Every `RecipeType.fields[i].type.enumRef(name)` references a declared enum.
3. No reference cycles in type declarations (BFS).
4. Every `inputs[i].type` is well-formed (no unknown record/enum refs).
5. Every step name is unique.
6. Every iteration variable name is unique within its scope; no shadowing input names.
7. Every emit's `typeName` is a declared `RecipeType`.
8. Every binding's `fieldName` exists on the typename's `RecipeType`.
9. All declared required fields are bound (warn on missing optional bindings).
10. Every transform name in any pipeline exists in `TransformImpls.registry`.
11. Every PathExpr's `variable(name)` is bound in scope at use site.
12. Every PathExpr's `input(name)` matches a declared input.
13. Every pagination strategy's required params are present.
14. AuthStrategy `htmlPrime`'s `stepName` references a declared step that fires before users.

**DiagnosticReport** value type:
```swift
public struct DiagnosticReport: Sendable {
    public let recipe: String
    public let runMode: RunMode    // .replay | .record | .live
    public let started: Date
    public let finished: Date
    public let outcome: Outcome    // .ok | .stalled(reason) | .failed(error)
    public let captures: CaptureSummary
    public let pagination: PaginationSummary?
    public let unhandledAffordances: AffordancesDump?
    public let snapshot: SnapshotSummary
    public let expectationGaps: [ExpectationGap]   // declared expects vs actual
}
```

Emitted as JSON to `<recipe-dir>/last-run.json` after every run.

**Expectations** — recipes can declare:
```
expect { records.where(typeName == "Product").count >= 100 }
expect { records.where(typeName == "Variant").count > 0 }
```
Validator checks expressions are well-formed; runner checks actual records against them.

**FixtureStore** layout:
```
recipe-dir/
  recipe.forage
  fixtures/
    01-prime.html
    02-categories.json
    03-products-flower-rec-page1.json
    03-products-flower-rec-page2.json
    ...
  snapshot.yaml          // canonical output
  last-run.json          // most recent DiagnosticReport (gitignored)
```

Fixture filenames: `<step-order>-<step-name>-<iteration-key>-<page>.<ext>`.

**SnapshotSerializer** — custom Codable for `Snapshot` / `ScrapedRecord` / `TypedValue` so they round-trip to YAML. YAML for snapshots (readable in PRs); JSON for fixtures (raw response bodies as captured).

**`forage` CLI**
- `forage validate <dir>` — load + parse + validate; exit 0/1.
- `forage test <dir>` — replay mode against `fixtures/`, run, diff records against `snapshot.yaml`, write `last-run.json`. Exit 0 if records == snapshot, 1 otherwise.
- `forage test <dir> --refresh` — record mode (live HTTP), update fixtures, regenerate `snapshot.yaml`, write `last-run.json`.

**Tests**
- ValidatorTests covering each check above (positive and negative case per check).
- SnapshotSerializerTests: round-trip a known Snapshot through YAML and assert equality.

**Acceptance:** `forage test <dir>` works for the Sweed example recipe (with hand-captured fixtures + initial snapshot generated by `--refresh`).

**Anti-patterns**
- No "fix it for the user" auto-correction in the validator. Just report.
- No silent fixture omission — if a step's fixture is missing in replay mode, fail with a clear error.

---

## Phase E — Browser engine

**Goal:** `BrowserEngine` runs a `Recipe` whose `engine` is `browser`. Wraps existing capture + BrowserPaginate machinery, drives a WKWebView from recipe-declared config.

**Files**
- `Sources/Forage/Recipe/BrowserConfig.swift`
- `Sources/Forage/Engine/BrowserEngine.swift`
- `Sources/Forage/Engine/BrowserSession.swift` — WKWebView host (extracted from `forage-probe/main.swift`)
- `Sources/Forage/Engine/BrowserDispatch.swift` — recipe-declared dismissal/warmup steps
- `Sources/Forage/Fixtures/WKURLSchemeReplayer.swift` — serves fixtures to WKWebView via `WKURLSchemeHandler`
- `Tests/ForageTests/BrowserEngineTests.swift` (limited — needs macOS UI session)

**BrowserConfig**
```swift
public struct BrowserConfig: Sendable {
    public let initialURL: Template
    public let ageGate: AgeGateConfig?
    public let dismissals: DismissalConfig?
    public let warmupClicks: [String]
    public let observe: String
    public let pagination: BrowserPaginationConfig
    public let captures: [CaptureExtractionRule]   // URL pattern + emission spec
}
public struct AgeGateConfig: Sendable {
    public let dob: DateComponents      // DD/MM/YYYY
    public let reloadAfter: Bool
}
public struct DismissalConfig: Sendable {
    public let maxAttempts: Int         // default 8
    public let labels: [String]?        // override default dismissal vocabulary
}
public struct BrowserPaginationConfig: Sendable {
    public let mode: BrowserPaginate.Mode
    public let seedFilter: String?
    public let replayOverride: [String: TypedValue]
    public let until: BrowserPaginateUntil
    public let maxIterations: Int
    public let iterationDelay: TimeInterval
}
public enum BrowserPaginateUntil: Sendable {
    case noProgressFor(Int)
    case captureCount(matching: String, atLeast: Int)
}
public struct CaptureExtractionRule: Sendable {
    public let urlPattern: String
    public let pathTo: PathExpr           // path within the captured response body to the items array
    public let emit: Emission             // how to construct a record per item
}
```

**BrowserEngine**
1. Spawn `BrowserSession` (NSApp + WKWebView).
2. Render `initialURL` via input substitution; load.
3. Run `ageGate` (if set) — `InjectedScripts.ageGateFill`, then reload.
4. Run dismissals — `InjectedScripts.dismissModal` loop until no match.
5. Run `warmupClicks` — `InjectedScripts.clickButtonByText` for each.
6. Start `BrowserPaginate` with `observe`, mode, `until`.
7. As captures arrive (via existing wrapper), apply `captures[*]` rules: any matching URL pattern → parse body as JSON → walk `pathTo` → for each item, construct `ScrapedRecord` per `emit`.
8. After paginate finishes, dump affordances + last-run diagnostic, return Snapshot.

**Replay support** — `WKURLSchemeReplayer` registers a custom scheme that serves fixtures. Browser recipes can be replayed offline by remapping the recipe's URLs through the scheme handler.

**Tests** — limited because BrowserEngineTests need an NSApplication runloop. Smoke test: load a static-content URL via the WKURLSchemeReplayer, assert that captures fire and Snapshot has expected records. Real validation comes in Phase F.

**Acceptance:** `forage test` works for a Jane recipe with browser fixtures.

**Anti-patterns**
- Don't overload `BrowserPaginate` with new modes for site-specific behavior. New gestures (e.g. hover-to-load) become new explicit recipe primitives, not implicit fallbacks.
- Don't auto-retry navigations. A failed navigation is a failed run.

---

## Phase F — Port the four real platforms

**Goal:** four canonical recipe directories, each with `recipe.forage` + `fixtures/` + `snapshot.yaml` + green `forage test`.

**Recipes**
- `recipes/sweed-zen-leaf-elkridge/` — http engine, two categories (flower, vapes), pageWithTotal, both rec + med menus.
- `recipes/leafbridge-remedy-columbia/` — http engine, prime step, untilEmpty pagination, both menus.
- `recipes/leafbridge-remedy-baltimore/` — same shape, different retailer UUID.
- `recipes/jane-trilogy-rec/` — browser engine, age-gate + reload, navigate to `/menu/all`, scroll-with-click-load-more, observe `/v2/smartpage`.
- `recipes/jane-trilogy-med/` — same, different storeId / menu URL.
- `recipes/dutchie-liberty-baltimore/` — browser engine. Probe first via `forage-probe`, identify the resolver endpoint, write recipe.
- `recipes/dutchie-liberty-oxon-hill/`
- `recipes/dutchie-liberty-rockville/`

**For each recipe**
1. Capture fixtures (live, with `forage test --refresh` or directly via `forage-probe`).
2. Write `recipe.forage`.
3. Run `forage test <dir>` until green.
4. Commit recipe + fixtures + snapshot.

**Acceptance:** all eight recipes pass `forage test`; their snapshots round-trip.

**Anti-patterns**
- Don't share recipes via includes / imports yet — too early. Two recipes for two Remedy stores duplicate the structure; that's fine for now.
- Don't inline platform-specific transforms into recipes if they generalize (e.g. `parseJaneSize`). Add to `TransformImpls.registry` as named transforms first.

---

## Phase G — Wire Forage into weed-prices

**Goal:** weed-prices loads recipes via Forage and persists Snapshots into SQLite.

**Files**
- `weed-prices/app/project.yml` — add Forage as Swift package dep (`packages: Forage: { path: "../forage" }`)
- `weed-prices/app/Sources/WeedPrices/Scraping/SnapshotPersister.swift` — Snapshot → SQLite
- `weed-prices/app/Sources/WeedPrices/Scraping/RecipeRegistry.swift` — load recipes from bundle + user-config dir
- `weed-prices/app/Sources/WeedPrices/Scraping/ScrapeService.swift` — rework to use Forage runtime
- `weed-prices/app/Sources/WeedPrices/Scraping/Dispensaries.swift` — per-store recipe-input config
- `weed-prices/app/Resources/recipes/` — bundle of canonical recipes (copied/symlinked from forage submodule's `recipes/`)
- `weed-prices/app/Sources/WeedPrices/SettingsView.swift` — "Test recipe" button

**SnapshotPersister**
- Input: `Snapshot` from Forage.
- For each `ScrapedRecord` typed `Dispensary`, upsert into `dispensary` table (by `slug`).
- For each `Category` record, upsert into `category` (by `dispensary_id, external_id`).
- For each `Product` record, upsert into `product` (by `dispensary_id, external_id`); resolve `categoryExternalId` to `category_id`.
- For each `Variant` record (or embedded variants in Product records — recipe author chooses), upsert into `variant`.
- For each `PriceObservation`, insert into `price_observation` with the right `snapshot_id`.
- Snapshot lifecycle: `startSnapshot()` returns ID; `finishSnapshotOk(...)` closes it.

The persister knows the weed-prices schema directly. If a recipe emits a record with a typeName the persister doesn't recognize, it errors with a clear message ("Recipe declared type 'Foo' which weed-prices doesn't know how to persist; supported: Dispensary, Category, Product, Variant, PriceObservation").

**RecipeRegistry**
- Loads `*.forage` files from `Bundle.main.resourceURL/recipes/` and `~/Library/Application Support/weed-prices/recipes/`.
- Parses + validates via Forage; surfaces errors in the LogStore.
- Exposes a registry: `recipes: [Recipe]` keyed by name.

**ScrapeService rework**
- For each dispensary in `Dispensaries.all`: look up the recipe by name (`dispensary.recipeName`), supply inputs from `DispensaryConfig`, run via `RecipeRunner`, persist via `SnapshotPersister`.
- All log output flows through the existing `LogStore`.

**Dispensaries.swift** — per-store config keys recipe inputs:
```swift
DispensaryConfig(
  slug: "zen-leaf-elkridge",
  recipeName: "sweed-zen-leaf-elkridge",
  inputs: [
    "storeId": .string("577"),
    "priceCategoryIds": .array([.int(5687), .int(5685), …]),
  ]
)
```

**SettingsView** — "Test recipes" section listing each recipe with a "Test" button that runs `forage test` programmatically and surfaces the DiagnosticReport.

**Tests** — live runs against real dispensaries (slow; not unit tests). Use the existing Update flow as a smoke test.

**Acceptance:**
- `xcodegen` regenerates clean.
- `xcodebuild` succeeds.
- Clicking "Update All" in the app drives all configured dispensaries through recipes and writes to the DB.

**Anti-patterns**
- Don't dual-path scrapers: weed-prices uses Forage exclusively. The bespoke Sweed/Leafbridge scrapers are the next phase to delete.
- Don't add fallback "if the recipe is missing, use the legacy scraper" — greenfield. Recipe missing = error visible in the log.

---

## Phase H — Migration cleanup

**Goal:** delete legacy bespoke scraper code now that recipes own all scraping.

**Files to delete**
- `weed-prices/app/Sources/WeedPrices/Scraping/SweedScraper.swift`
- `weed-prices/app/Sources/WeedPrices/Scraping/Sweed.swift`
- `weed-prices/app/Sources/WeedPrices/Scraping/LeafbridgeScraper.swift`
- `weed-prices/app/Sources/WeedPrices/Scraping/Leafbridge.swift`

**Files to update**
- `weed-prices/notes/scraping-dsl.md` — strike-through the engine-status section (now historical)
- `weed-prices/notes/jane-platform.md` — point at `recipes/jane-trilogy-*` instead of "future"
- `weed-prices/notes/README.md` — table reflects recipe-driven status
- `weed-prices/notes/zen-leaf-elkridge.md`, `remedy-maryland.md`, etc. — annotate that these are historical research; the runtime path is the recipe

**Acceptance:** `xcodebuild` succeeds with the bespoke scraper code deleted. Update All still works through recipes only.

**Anti-patterns**
- Don't keep legacy code "in case the recipe path breaks." Git history is the archive.
- Don't half-migrate (e.g. keep one bespoke scraper because its recipe is harder). All-or-nothing per phase boundary.

---

## Cross-cutting principles (apply to every phase)

- **No migrations.** When schemas change, edit + wipe. No `IF NOT EXISTS`, no version flags, no compat shims.
- **Delete don't comment out.** Old code goes; git is the archive.
- **One commit per phase.** Big enough to land an architecturally complete unit; small enough to revert atomically.
- **Tests are not gating; they're confirming.** Don't write speculative tests. Write tests that confirm the recipe + engine behavior we actually need.
- **Real-world validation = live runs against the four platforms.** Phases B-E culminate in F (port real recipes); F is where you discover what's broken.

---

## Worktree

All Phase A-F work happens in a forage worktree on the `runtime` branch:

```sh
cd /Users/dima/dev/forage
git worktree add ../forage-runtime runtime
cd ../forage-runtime
```

After F, merge the branch back into main forage. Then bump the submodule pointer in weed-prices and continue with Phases G-H on a `forage-integration` branch in weed-prices.
