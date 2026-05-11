import Foundation

/// Browser-engine recipe config — the navigation / dismissal / pagination
/// plan a `BrowserEngine` follows. Fully hydrated from the recipe parser;
/// the runtime feeds it to the browser session in order.
public struct BrowserConfig: Hashable, Sendable {
    public let initialURL: Template
    public let ageGate: AgeGateConfig?
    public let dismissals: DismissalConfig?
    public let warmupClicks: [String]
    public let observe: String
    public let pagination: BrowserPaginationConfig
    /// Capture-extract rules: when a captured fetch/XHR response URL matches
    /// `urlPattern`, walk its body via `iterPath` and run the inner statements
    /// per item. Statements typically emit records using the per-item scope.
    public let captures: [CaptureRule]
    /// Document-extract rule: fires once after the browser has settled
    /// (page loaded, pagination done). The capture body is the rendered
    /// `document.documentElement.outerHTML`. Lets recipes extract from
    /// server-rendered HTML behind bot management — sites where the data
    /// is in the initial DOM, not in XHR responses (eBay, Cloudflare-gated
    /// pages, classic server-rendered listings).
    public let documentCapture: DocumentCaptureRule?

    public init(
        initialURL: Template,
        ageGate: AgeGateConfig? = nil,
        dismissals: DismissalConfig? = nil,
        warmupClicks: [String] = [],
        observe: String,
        pagination: BrowserPaginationConfig,
        captures: [CaptureRule] = [],
        documentCapture: DocumentCaptureRule? = nil
    ) {
        self.initialURL = initialURL
        self.ageGate = ageGate
        self.dismissals = dismissals
        self.warmupClicks = warmupClicks
        self.observe = observe
        self.pagination = pagination
        self.captures = captures
        self.documentCapture = documentCapture
    }
}

public struct AgeGateConfig: Hashable, Sendable {
    /// DOB to fill in. Plain components rather than a Date because some
    /// age-gate forms expect specific year/month/day inputs separately.
    public let year: Int
    public let month: Int
    public let day: Int
    /// True if the form posts via AJAX without navigating; runtime forces a
    /// reload after submitting so the SPA boots fresh on the post-gate page.
    public let reloadAfter: Bool

    public init(year: Int, month: Int, day: Int, reloadAfter: Bool = true) {
        self.year = year; self.month = month; self.day = day
        self.reloadAfter = reloadAfter
    }
}

public struct DismissalConfig: Hashable, Sendable {
    public let maxAttempts: Int
    public let extraLabels: [String]   // additional labels to recognize beyond the runtime defaults

    public init(maxAttempts: Int = 8, extraLabels: [String] = []) {
        self.maxAttempts = maxAttempts
        self.extraLabels = extraLabels
    }
}

public struct BrowserPaginationConfig: Hashable, Sendable {
    public enum Mode: String, Hashable, Sendable {
        case scroll
        case replay
    }
    public let mode: Mode
    public let until: BrowserPaginateUntil
    public let maxIterations: Int
    public let iterationDelay: TimeInterval
    /// Optional substring filter on captured request bodies (replay-mode seed picking).
    public let seedFilter: String?
    /// Replay-mode override: dotted-path → value (with `$i` template substitution).
    public let replayOverride: [String: ExtractionExpr]

    public init(
        mode: Mode,
        until: BrowserPaginateUntil,
        maxIterations: Int = 30,
        iterationDelay: TimeInterval = 1.8,
        seedFilter: String? = nil,
        replayOverride: [String: ExtractionExpr] = [:]
    ) {
        self.mode = mode
        self.until = until
        self.maxIterations = maxIterations
        self.iterationDelay = iterationDelay
        self.seedFilter = seedFilter
        self.replayOverride = replayOverride
    }
}

public enum BrowserPaginateUntil: Hashable, Sendable {
    case noProgressFor(Int)
    case captureCount(matching: String, atLeast: Int)
}

public struct CaptureRule: Hashable, Sendable {
    public let urlPattern: String
    /// Iterate this expression within the matched response; the inner body
    /// runs per item, with `$.` bound to the item. An `ExtractionExpr` so
    /// HTML-bearing captures can pipe through `parseHtml | select(...)`.
    public let iterPath: ExtractionExpr
    public let body: [Statement]

    public init(urlPattern: String, iterPath: ExtractionExpr, body: [Statement]) {
        self.urlPattern = urlPattern
        self.iterPath = iterPath
        self.body = body
    }
}

/// Document-extract rule (M9). Fires once after the browser engine
/// has settled. The capture's "current value" (`$.`) is the parsed
/// document node — recipes walk it with `select(...)` / `text` / `attr`
/// directly, no explicit `parseHtml` needed. The grammar mirrors
/// `captures.match` but with no `urlPattern` (the document body has
/// no URL of its own).
public struct DocumentCaptureRule: Hashable, Sendable {
    public let iterPath: ExtractionExpr
    public let body: [Statement]

    public init(iterPath: ExtractionExpr, body: [Statement]) {
        self.iterPath = iterPath
        self.body = body
    }
}
