# Jane / browser-engine pipeline — forage-side work

The goal: ship the runtime support that lets a forage consumer drive Jane
(or any other browser-engine recipe) to completion with live progress,
diagnostic reports, expectation evaluation, archived artifacts, hot-
reload, and offline replay.

Consumer apps (weed-prices etc.) just embed the runtime + their recipes
and call `BrowserEngine` / `RecipeRunner`. They never host a `WKWebView`,
never know about captures, never see the page. Everything the consumer
surfaces (status text, counters, log lines) it reads off forage's public
progress/diagnostic types.

Worktree: `/Users/dima/dev/forage-jane/`, branch `jane-browser-pipeline`.
Each phase is one or more commits on this branch; once a phase lands, the
weed-prices worktree at `/Users/dima/dev/weed-prices-jane/` bumps its
submodule pointer to pick it up.

---

## Phase 1 — `BrowserProgress`

**Goal:** an `@Observable` progress signal that `BrowserEngine` drives
during a run so consumers can read phase / capture-count / record-count
live and so the engine can apply capture rules incrementally instead of
only at `buildSnapshot()` time.

**Files:**

- `Sources/Forage/Engine/BrowserProgress.swift` *(new)*.
- `Sources/Forage/Engine/BrowserEngine.swift` *(modify)*.
- `Sources/Forage/Forage.swift` *(modify — header comment lists the new public type)*.

**API:**

```swift
@MainActor
@Observable
public final class BrowserProgress {
    public enum Phase: Sendable, Hashable {
        case starting
        case loading
        case ageGate
        case dismissing
        case warmupClicks
        case paginating(iteration: Int, maxIterations: Int)
        case settling
        case done
        case failed(String)
    }
    public private(set) var phase: Phase = .starting
    public private(set) var capturesObserved: Int = 0
    public private(set) var recordsEmitted: Int = 0
    public private(set) var currentURL: String? = nil
    public private(set) var lastObservedURL: String? = nil
}
```

Mutators are `internal` so only the engine drives them.

**Engine integration:**

- `BrowserEngine` owns `public let progress = BrowserProgress()`.
- Transitions: `.loading` at `start()`; `.ageGate` once initial nav
  finishes and age-gate runs; `.dismissing`; `.warmupClicks` (skip if
  none configured); `.paginating(i, max)` updated per iteration from the
  `BrowserPaginate` callback; `.settling` post-pagination; `.done` (or
  `.failed`) on finish.
- **Capture handling refactor:** today the engine accumulates `captures:
  [Capture]` and runs `applyRule(...)` once at `buildSnapshot()`. Move
  the apply step inline: in
  `userContentController(_:didReceive:)`, each capture is dispatched
  through `applyRule` *as it arrives* into a long-lived
  `EmissionCollector` property. `progress.recordsEmitted` is
  `collector.records.count`. `buildSnapshot()` just snapshots the
  running collector.
- While refactoring, stash a `private var unmatchedCaptures: [Capture]`
  for Phase 3 (captures that matched no rule, which feed
  `DiagnosticReport.unmatchedCaptures`). Internal for now.

**Acceptance:**
- `swift test` — 27 existing tests still pass, plus a new test
  exercising `BrowserProgress` mutator semantics.
- Header comment in `Forage.swift` mentions `BrowserProgress`.

---

## Phase 2 — `HTTPProgress`

**Goal:** parity with `BrowserProgress` for HTTP-engine recipes so
consumers can show live counters for Sweed/Leafbridge scrapes too.

**Files:**

- `Sources/Forage/Engine/HTTPProgress.swift` *(new)*.
- `Sources/Forage/Engine/HTTPEngine.swift` *(modify)* — drive transitions.
- `Sources/Forage/Engine/RecipeRunner.swift` *(modify)* — expose
  `progress` so the consumer can read it without holding the engine.

**API:**

```swift
@MainActor
@Observable
public final class HTTPProgress {
    public enum Phase: Sendable, Hashable {
        case starting
        case priming                                   // auth.htmlPrime in flight
        case stepping(name: String)                    // running a `step` block
        case paginating(name: String, page: Int)       // inside a paginate clause
        case done
        case failed(String)
    }
    public private(set) var phase: Phase = .starting
    public private(set) var requestsSent: Int = 0
    public private(set) var recordsEmitted: Int = 0
    public private(set) var currentURL: String? = nil
}
```

Same model as `BrowserProgress`: engine drives mutations, consumer
observes. The HTTP engine already loops over `Recipe.body` statements;
add progress updates as each `step` / `paginate` / `emit` runs.

**Acceptance:** unit test mirroring the BrowserProgress one + a synthetic
recipe run that asserts `progress.phase` transitions through priming →
stepping → paginating → done.

---

## Phase 3 — `DiagnosticReport` + `RunResult`

**Goal:** every recipe run returns `(Snapshot, DiagnosticReport)` so the
consumer can show *why* a run stalled or fell short.

**Files:**

- `Sources/Forage/Engine/RunResult.swift` *(new)*.
- `Sources/Forage/Engine/DiagnosticReport.swift` *(new)*.
- `Sources/Forage/Engine/BrowserEngine.swift` *(modify run signature)*.
- `Sources/Forage/Engine/RecipeRunner.swift` *(modify)*.
- `Sources/Forage/Engine/HTTPEngine.swift` *(modify)*.

**API:**

```swift
public struct RunResult: Sendable, Hashable {
    public let snapshot: Snapshot
    public let report: DiagnosticReport
}

public struct DiagnosticReport: Sendable, Hashable {
    public let stallReason: String       // "settled" / "hard-timeout" / "navigation-failed: …"
    public let unmatchedCaptures: [UnmatchedCapture]
    public let unfiredRules: [String]    // URL patterns of rules that never matched
    public let unmetExpectations: [String]   // filled by Phase 4
    public let unhandledAffordances: [String]
}

public struct UnmatchedCapture: Sendable, Hashable {
    public let url: String
    public let method: String
    public let status: Int
    public let bodyBytes: Int
}
```

Browser engine populates `unmatchedCaptures`, `unfiredRules`,
`unhandledAffordances`, `stallReason`. HTTP engine populates
`stallReason` only (no captures concept); `unmetExpectations` lands in
Phase 4.

`run()` signatures change to `-> RunResult` from `-> Snapshot`.

**Acceptance:** existing tests adjusted to unpack `RunResult.snapshot`;
new tests for diagnostic population.

---

## Phase 4 — Expectation evaluation

**Goal:** evaluate `expect { records.where(typeName == "Product").count
>= 500 }` clauses against the produced snapshot and surface failures in
`DiagnosticReport.unmetExpectations`.

**Files:**

- `Sources/Forage/Engine/ExpectationEvaluator.swift` *(new)*.
- `Sources/Forage/Engine/BrowserEngine.swift` + `HTTPEngine.swift` *(modify)*
  — call the evaluator after the run, before returning `RunResult`.

**Expectation grammar (already parsed in `Recipe.swift::Expectation`):**

```
records.where(typeName == "Product").count >= 500
records.where(typeName == "PriceObservation").count > 0
```

The evaluator interprets these against `Snapshot.records`. On failure,
renders `records.where(typeName == "Product").count >= 500 (got 247)`
into `unmetExpectations`.

**Acceptance:** unit tests covering pass/fail; integration with engines
verifies the report is populated.

---

## Phase 5 — Archive format + reader/writer

**Goal:** durable on-disk record of every run: snapshot, diagnostic
report, raw captures (browser only), meta (recipe name, inputs,
runtime). Forage owns the format and the IO; consumers pass a directory
path and call `Archive.write(...)` / `Archive.list(...)` /
`Archive.read(...)`.

**Files:**

- `Sources/Forage/Fixtures/Archive.swift` *(new)*.
- `Sources/Forage/Engine/BrowserEngine.swift` — expose `captures` as
  public read-only so the archive writer can persist them.

**Layout (caller-supplied root):**

```
<root>/<dispensary-slug>/<ISO8601-Z>/
    snapshot.json
    diagnostic.json
    captures.jsonl        # browser-engine only
    meta.json             { recipe: "jane", inputs: {...}, runtime: 12.3 }
```

**API:**

```swift
public struct ArchiveMeta: Sendable, Hashable, Codable {
    public let recipeName: String
    public let inputs: [String: JSONValue]
    public let runtimeSeconds: Double
    public let observedAt: Date
}

public struct ArchiveRunHandle: Sendable, Hashable {
    public let slug: String
    public let observedAt: Date
    public let directory: URL
}

public enum Archive {
    public static func write(root: URL, slug: String,
                             snapshot: Snapshot, report: DiagnosticReport,
                             captures: [Capture]?, meta: ArchiveMeta) throws -> ArchiveRunHandle
    public static func list(root: URL, slug: String) -> [ArchiveRunHandle]
    public static func read(_ handle: ArchiveRunHandle) throws
        -> (snapshot: Snapshot, report: DiagnosticReport,
            captures: [Capture]?, meta: ArchiveMeta)
}
```

**Acceptance:** round-trip test (write then read = identity); list
returns runs sorted newest-first.

---

## Phase 6 — Recipe registry + hot-reload

**Goal:** forage ships a `RecipeRegistry` that consumers point at a
directory and ask for recipes by name; in dev builds it watches the
directory and reloads on `.forage` changes so the consumer never has to
rebuild to iterate a recipe.

**Files:**

- `Sources/Forage/Recipe/RecipeRegistry.swift` *(new)*.
- `Sources/Forage/Recipe/RecipeWatcher.swift` *(new — internal)*.

**API:**

```swift
@MainActor
@Observable
public final class RecipeRegistry {
    public init(root: URL, watch: Bool = false,
                logger: ((String) -> Void)? = nil)
    public func loadAll() throws
    public func recipe(forName name: String) -> Recipe?
    public var recipes: [String: Recipe] { get }
}
```

`root` is a directory of the form `<root>/<platform>/recipe.forage`.
When `watch: true`, a `DispatchSourceFileSystemObject` (or
`FSEventStream`) on the root detects changes and triggers
parse+validate+swap of the affected recipe; old entry stays loaded if
validation fails.

(weed-prices' current `RecipeRegistry.swift` lives in the consumer; it
becomes a thin call through to this forage type.)

**Acceptance:** synthetic test directory, mutate a recipe file on disk,
verify the registry reflects the new content after the system file
event fires.

---

## Phase 7 — `BrowserReplayer`

**Goal:** drive a `BrowserEngine` run from saved captures (e.g. a Phase
5 `captures.jsonl`) instead of hitting the network. Lets recipe authors
iterate extraction logic offline.

**Files:**

- `Sources/Forage/Fixtures/BrowserReplayer.swift` *(new)*.
- `Sources/Forage/Engine/BrowserEngine.swift` *(modify)* — accept an
  optional `replayer`; when set, skip `WKWebView.load(...)` entirely and
  feed captures into the same handler the JS bridge calls.

**API:**

```swift
public struct BrowserReplayer: Sendable {
    public init(capturesFile: URL) throws
    public init(captures: [Capture])
    public let captures: [Capture]
}
```

`BrowserEngine.init` gains `replayer: BrowserReplayer? = nil`. When
non-nil, the engine still constructs a (now offscreen) WKWebView for
type/host-protocol consistency but never loads the initial URL; instead
its `start()` enqueues each capture into the existing capture handler.
Phase transitions jump directly through loading → paginating(0,0) →
settling → done.

**Acceptance:** test that synthesizes a capture list, runs a recipe
under the replayer, asserts the resulting snapshot matches the expected
extraction.

---

## Phase order

Phases are independent enough that order is mostly about dependencies:

1. `BrowserProgress` (foundation)
2. `HTTPProgress` (independent — can run in parallel with 1 if needed)
3. `DiagnosticReport` + `RunResult` (1 must be done so we can populate
   `unmatchedCaptures` / `unfiredRules`)
4. Expectation evaluation (3 is the carrier)
5. Archive (3 + 4 are inputs)
6. `RecipeRegistry` (independent)
7. `BrowserReplayer` (1 + 3 are dependencies; uses the same capture
   pipeline)

Execute serially in numeric order.
