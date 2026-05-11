# Engines & pagination

Pick the right engine for the site, then pick the right pagination strategy for the endpoint.

## Two engines

Every recipe declares one engine at the top:

```forage
engine http      // for documented APIs
engine browser   // for JS-rendered SPAs
```

### HTTP engine

Drives the recipe over `URLSession`. Cheap, fast, deterministic. Use it when the site has documented JSON endpoints or returns server-rendered HTML that you can parse directly. The HTTP engine implements:

- Implicit cookie jar shared across all steps in a run.
- Polite defaults: rate-limited (~1 req/sec by default), exponential backoff on 429 and 5xx, honest User-Agent.
- Templated URLs, headers, JSON and form-encoded bodies.
- `auth.staticHeader` and `auth.htmlPrime` strategies.
- Two HTTP pagination strategies, below.
- Transient-error retry only — `URLError.timedOut`, `.notConnectedToInternet`, etc. 404s and parse errors fail fast instead of retrying.

### Browser engine

Drives the recipe through a real `WKWebView` on macOS. The host application is responsible for running an `NSApplication` event loop; the engine runs on the main actor. Use it when the data sits behind:

- A JavaScript single-page app that constructs requests with session tokens or signatures the page mints itself.
- Generic bot-management gates (e.g. Cloudflare) on otherwise-public pages — a real browser clears these.
- Per-session cookies, CSP, or Origin checks that a plain `URLSession` client can't satisfy.

The browser engine doesn't construct the page's API requests itself. It loads the page, observes the in-flight requests the SPA fires, and either lets the page paginate naturally (scroll mode) or replays a captured seed request with overridden parameters (replay mode).

::: warning Recipes don't bypass access controls
Neither engine logs in for you, solves real CAPTCHAs, or works against pages that require a paid account. Generic bot-management gates on otherwise-public pages are a different category and are cleared by the browser engine.
:::

## What an engine returns

Both engines return a `RunResult`:

```swift
public struct RunResult: Sendable, Hashable {
    public let snapshot: Snapshot           // emitted records
    public let report:   DiagnosticReport   // how the run terminated
}
```

The snapshot is the produced records; the report is the post-run forensics. A successful HTTP run reports `stallReason == "completed"`. A successful browser run reports `stallReason == "settled"`. Cancelled runs report `stallReason == "cancelled"`. See [Diagnostics](/docs/diagnostics) for the full set of report fields.

## Live progress

Each engine exposes a `progress` value the consumer reads concurrently with the run. Both are `@MainActor @Observable` so SwiftUI sees per-field updates without `@Published` or polling.

```swift
public final class HTTPProgress {
    public enum Phase {
        case starting, priming
        case stepping(name: String)
        case paginating(name: String, page: Int)
        case done
        case failed(String)
    }
    public var phase:           Phase
    public var requestsSent:    Int
    public var recordsEmitted:  Int
    public var currentURL:      String?
}

public final class BrowserProgress {
    public enum Phase {
        case starting, loading, ageGate, dismissing, warmupClicks
        case paginating(iteration: Int, maxIterations: Int)
        case settling, done
        case failed(String)
    }
    public var phase:             Phase
    public var capturesObserved:  Int
    public var recordsEmitted:    Int
    public var currentURL:        String?
    public var lastObservedURL:   String?
}
```

Wire `progress.recordsEmitted` straight into a counter view; the browser engine applies capture rules incrementally, so the count is meaningful *during* the run, not just after it settles.

## Cancellation

Both engines honor `Task.cancel()`. The in-flight `URLSession.data(...)` or pagination loop unwinds, the run terminates, and the diagnostic report carries `stallReason: "cancelled"` with the corresponding `progress.phase: .failed("cancelled")`. The snapshot reflects whatever records the engine had emitted before the cancellation arrived.

## Pagination

The DSL exposes a small, named set of pagination strategies. The engine handles the loop; the recipe declares which strategy and points at the relevant response paths. New strategies are added to the engine in Swift as real platforms surface them.

### pageWithTotal

For endpoints that return a page of items plus a total count. The engine bumps the page parameter until accumulated items meet or exceed the total.

```forage
step products {
    method "POST"
    url    "https://api.example.com/products"
    body.json { page: 1, pageSize: 200 }
    paginate pageWithTotal {
        items:     $.list
        total:     $.total
        pageParam: "page"
        pageSize:  200
    }
}
```

### untilEmpty

For endpoints that return a page of items but no total. The engine bumps the page parameter until a response comes back empty or shorter than the page size.

```forage
paginate untilEmpty {
    items:     $.data.products
    pageParam: "page"
    pageSize:  60
}
```

### browserPaginate

Only available on the browser engine. The recipe doesn't construct paginated requests itself — instead it tells the engine which request URL pattern signals a "page arrived" and how to trigger the next one.

#### scroll mode

After the first capture, the engine drives the SPA forward by scrolling the page (including nested shadow-DOM scrollables) and clicking the bottom-most visible button labeled `"View more"`, `"Load more"`, `"Show more"`, etc. The page fires its own next-page request with its own auth; the engine captures it.

```forage
paginate browserPaginate {
    observe: "iheartjane.com/v2/smartpage"
    mode:    "scroll"
    until:   { no_progress_for: 3 }
}
```

#### replay mode

After the first capture, the engine takes its request body/headers as a template, applies per-iteration overrides, and re-fires via the page's own `fetch` (so Origin, cookies, CSP, and session tokens all match). Faster than scroll mode when it works; requires knowing which request param controls pagination.

```forage
paginate browserPaginate {
    observe:  "dmerch.iheartjane.com/v2/multi"
    mode:     "replay"
    override: { page: $i }
    until:    { count >= $.placements[*].nb_hits }
}
```

Both browser-pagination modes share `observe` (which request URL signals a page) and `until` (termination — `no_progress_for: N`, `max_iterations: N`, or a count comparison).

::: warning Anti-pattern
Don't bypass pagination by demanding an oversized batch in a single request (e.g. raising a natural `pageSize: 60` to `2000`). Drive the site's natural pagination instead — it's politer, less likely to trip rate limits, and survives shape changes.
:::

## Diagnostic affordances

When a browser-engine run finishes — success or stall — it writes a sibling JSON file enumerating every visible button, link, `role=button`, and scrollable on the page, with text, selector, and viewport position. When a recipe doesn't reach expected coverage, the recipe author (human or LLM) reads this file to see what UI affordances the engine didn't interact with.

The vocabulary in the dump matches the vocabulary the recipe uses (button text, selector, position) so the corrective edit is a direct reading of the dump.
