#if canImport(WebKit)
import Foundation
import AppKit
import WebKit
import SwiftSoup

/// Runs a `Recipe` whose `engineKind` is `.browser`. Hosts a `WKWebView`,
/// drives navigation / age-gate fill / dismissal / warmup-clicks /
/// pagination per `BrowserConfig`, captures fetch/XHR responses through the
/// existing `InjectedScripts.captureWrapper`, and applies the recipe's
/// `captures.match` rules to produce a `Snapshot`.
///
/// Requires an active `NSApplication` event loop — the consumer (the macOS
/// app, `forage-probe`, etc.) is responsible for starting that. The
/// `run(recipe:inputs:)` method spawns a window, drives the SPA to
/// completion, and `NSApp.terminate(nil)` to exit when the recipe settles
/// (or hits its timeout). The runner returns the accumulated Snapshot.
///
/// Live progress is exposed via `progress` (a `BrowserProgress`) so consumers
/// can render phase / counters without polling. Rule application happens
/// incrementally — each capture is dispatched through its matching
/// `captures.match` rule on arrival and emits records into a long-lived
/// `EmissionCollector` — so `progress.recordsEmitted` is meaningful while
/// the run is in flight, not just after it completes.
///
/// **Replay mode.** Pass a `BrowserReplayer` (typically built from a
/// `captures.jsonl` an earlier `Archive.write(...)` produced) and the
/// engine skips `WKWebView.load(...)`, the age-gate, dismissals, warmup,
/// pagination, and the settle / hard-timeout timers. It feeds each
/// captured exchange through the same `captures.match` pipeline and
/// returns the resulting `RunResult` without ever hitting the network.
/// M10 interactive-bootstrap policy. Honored only when the recipe
/// declares `browser.interactive`.
public enum InteractiveBootstrapMode: Sendable {
    /// Reuse a cached session if one exists and isn't expired; otherwise
    /// open the visible window for human handshake.
    case auto
    /// Force a fresh bootstrap regardless of cache (the user passed
    /// `--interactive` on the CLI, or "Re-bootstrap session" in the
    /// Toolkit).
    case forceBootstrap
    /// Skip bootstrap entirely — headless only. Falls back to running
    /// without seeded cookies when no cache exists; useful for CI hosts
    /// that have a session file copied in from a workstation.
    case skipBootstrap
}

@MainActor
public final class BrowserEngine: NSObject, WKNavigationDelegate, WKScriptMessageHandler, BrowserPaginateHost {
    public let recipe: Recipe
    public let inputs: [String: JSONValue]
    public let evaluator: ExtractionEvaluator
    public let visible: Bool
    public let settleSeconds: TimeInterval
    public let hardTimeoutSeconds: TimeInterval
    public private(set) var webView: WKWebView!

    /// Pre-resolved session cookies to seed into the WKWebView before
    /// navigation. Populated by the host before calling `run()` — typically
    /// the host runs an HTTPEngine login flow first, captures the resulting
    /// `SessionState.payload == .cookies(...)`, and hands them in here.
    public var seedCookies: [SessionCookie] = []

    /// M10 interactive bootstrap mode (honored only when the recipe's
    /// `browser.interactive` is set).
    public let interactiveBootstrapMode: InteractiveBootstrapMode
    /// Root for persisted interactive sessions. Defaults to
    /// `InteractiveSessionStore.defaultRoot()`. Tests override.
    public let interactiveSessionRoot: URL?

    /// Computed once at init: did we resolve to do a fresh bootstrap on
    /// this run, or are we reusing a cached session?
    private let isInteractiveBootstrap: Bool
    /// Cached session being reused (when `isInteractiveBootstrap == false`
    /// and the recipe declares interactive). Nil otherwise.
    private let cachedInteractiveSession: InteractiveSession?

    public let progress = BrowserProgress()

    private var window: NSWindow?
    private var settleTimer: Timer?
    private var hardTimer: Timer?
    private var paginate: BrowserPaginate?
    public private(set) var captures: [Capture] = []
    private var collector = EmissionCollector()
    private var unmatchedCaptures: [Capture] = []
    /// Per-rule match counter, keyed by `CaptureRule.urlPattern`. Rules that
    /// never see a matching capture surface as `unfiredRules` in the report.
    private var ruleMatchCounts: [String: Int] = [:]
    private let scope: Scope
    private var didFireWarmup = false
    private var dismissAttempts = 0
    private var didFinishNav = false

    private var continuation: CheckedContinuation<RunResult, any Error>?
    private let replayer: BrowserReplayer?

    public init(
        recipe: Recipe,
        inputs: [String: JSONValue],
        evaluator: ExtractionEvaluator = ExtractionEvaluator(),
        visible: Bool = true,
        settleSeconds: TimeInterval = 8,
        hardTimeoutSeconds: TimeInterval = 240,
        replayer: BrowserReplayer? = nil,
        interactiveBootstrapMode: InteractiveBootstrapMode = .auto,
        interactiveSessionRoot: URL? = nil
    ) {
        precondition(recipe.engineKind == .browser, "BrowserEngine requires browser-engine Recipe")
        precondition(recipe.browser != nil, "browser-engine Recipe must have a `browser { … }` block")
        self.recipe = recipe
        self.inputs = inputs
        self.evaluator = evaluator
        self.settleSeconds = settleSeconds
        self.hardTimeoutSeconds = hardTimeoutSeconds
        self.replayer = replayer
        self.interactiveBootstrapMode = interactiveBootstrapMode
        self.interactiveSessionRoot = interactiveSessionRoot
        self.scope = Scope(inputs: inputs, frames: [[:]], current: nil)

        // M10: decide bootstrap-vs-reuse before the WKWebView is built so
        // we can scope the message-handler set + visible flag accordingly.
        let interactiveCfg = recipe.browser?.interactive
        let cachedSession: InteractiveSession? = {
            guard interactiveCfg != nil else { return nil }
            let url = InteractiveSessionStore.file(for: recipe.name, root: interactiveSessionRoot)
            guard let s = InteractiveSessionStore.read(at: url), !s.isExpired() else { return nil }
            return s
        }()
        let needsBootstrap: Bool = {
            guard interactiveCfg != nil else { return false }
            switch interactiveBootstrapMode {
            case .forceBootstrap: return true
            case .auto:           return cachedSession == nil
            case .skipBootstrap:  return false
            }
        }()
        self.isInteractiveBootstrap = needsBootstrap
        self.cachedInteractiveSession = needsBootstrap ? nil : cachedSession

        // Bootstrap mode forces a visible window so the human can interact;
        // reuse mode forces headless. Recipes that don't declare interactive
        // honor whatever `visible` value the caller passed.
        if interactiveCfg != nil {
            self.visible = needsBootstrap
        } else {
            self.visible = visible
        }

        super.init()

        let config = WKWebViewConfiguration()
        config.websiteDataStore = .default()
        let ucc = WKUserContentController()
        let captureScript = WKUserScript(
            source: InjectedScripts.captureWrapper,
            injectionTime: .atDocumentStart,
            forMainFrameOnly: false
        )
        ucc.addUserScript(captureScript)
        ucc.add(self, name: "captureNetwork")
        if interactiveCfg != nil && needsBootstrap {
            // Only register the bootstrap message handler when we'll
            // actually inject the overlay — saves the WKWebView from
            // routing a name we'll never post to.
            ucc.add(self, name: "forageInteractiveDone")
        }
        config.userContentController = ucc

        self.webView = WKWebView(
            frame: NSRect(x: 0, y: 0, width: 1280, height: 900),
            configuration: config
        )
        webView.navigationDelegate = self

        // Construct paginate from recipe config.
        let bcfg = recipe.browser!
        let pmode: BrowserPaginate.Mode = (bcfg.pagination.mode == .scroll) ? .scroll : .replay
        let untilLimit: Int = {
            if case .noProgressFor(let n) = bcfg.pagination.until { return n }
            return 3
        }()
        self.paginate = BrowserPaginate(
            observe: bcfg.observe,
            mode: pmode,
            replayOverride: [:],
            seedFilter: bcfg.pagination.seedFilter,
            maxIterations: bcfg.pagination.maxIterations,
            noProgressLimit: untilLimit,
            iterationDelay: bcfg.pagination.iterationDelay
        )
        self.paginate?.host = self
    }

    /// Run the recipe. Returns a `RunResult` carrying the accumulated
    /// snapshot plus a `DiagnosticReport` describing how the run terminated
    /// and what wasn't accounted for. Throws only for setup-time errors
    /// (e.g. an unparseable `initialURL` in the recipe); runtime termination
    /// reasons (settled / hard-timeout / nav-fail / cancelled) come back
    /// through `report.stallReason` instead.
    ///
    /// Honors `Task.cancel()`: the cancellation handler hops to the main
    /// actor (the engine is `@MainActor`-isolated, but the handler runs
    /// synchronously on whatever queue called `cancel()`) and triggers
    /// `finish(reason: "cancelled")`, which resolves the continuation with
    /// `stallReason: "cancelled"` and whatever snapshot has accumulated. If
    /// the run had already finished, `finish` no-ops via its
    /// `guard let cont = continuation` idempotency check.
    public func run() async throws -> RunResult {
        try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { (cont: CheckedContinuation<RunResult, any Error>) in
                self.continuation = cont
                do {
                    try start()
                } catch {
                    self.continuation = nil
                    progress.setPhase(.failed("\(error)"))
                    cont.resume(throwing: error)
                }
            }
        } onCancel: { [weak self] in
            Task { @MainActor [weak self] in
                self?.finish(reason: "cancelled")
            }
        }
    }

    private func start() throws {
        if replayer != nil {
            try startReplay()
            return
        }

        let bcfg = recipe.browser!
        // M10: interactive bootstrap mode loads `bootstrapURL` (if the
        // recipe declared one); reuse mode loads `initialURL` normally.
        let urlTemplate: Template = {
            if isInteractiveBootstrap, let bs = bcfg.interactive?.bootstrapURL {
                return bs
            }
            return bcfg.initialURL
        }()
        let url = try TemplateRenderer.render(urlTemplate, in: scope)
        guard let urlValue = URL(string: url) else {
            throw BrowserEngineError.invalidInitialURL(url)
        }

        // M10: reuse mode seeds the cached cookies before the WKWebView's
        // first request goes out. Bootstrap mode never has cached cookies
        // by construction.
        if let cached = cachedInteractiveSession {
            seedCookies.append(contentsOf: cached.cookies)
        }

        if visible {
            let win = NSWindow(
                contentRect: NSRect(x: 60, y: 60, width: 1280, height: 900),
                styleMask: [.titled, .closable, .resizable],
                backing: .buffered,
                defer: false
            )
            let titlePrefix = isInteractiveBootstrap
                ? "Forage — \(recipe.name) [sign in / handle gate, then click ✓ Scrape this page]"
                : "Forage — \(recipe.name)"
            win.title = titlePrefix
            win.contentView = webView
            win.makeKeyAndOrderFront(nil)
            self.window = win
        }

        progress.setPhase(.loading)
        progress.setCurrentURL(url)

        // If the host (or session-auth bootstrap) supplied seed cookies,
        // install them in both HTTPCookieStorage.shared (so URLSession-backed
        // fetches inherit) and the webView's data store (so the SPA's own
        // fetch/XHR see them too). We have to seed before `load(...)` so the
        // first navigation request goes out with cookies attached.
        installSeedCookies(forURL: urlValue) { [weak self] in
            guard let self else { return }
            self.webView.load(URLRequest(url: urlValue))
        }

        hardTimer = Timer.scheduledTimer(withTimeInterval: hardTimeoutSeconds, repeats: false) { [weak self] _ in
            self?.finish(reason: "hard-timeout")
        }
        resetSettleTimer()
    }

    /// Replay-mode entry: skip `WKWebView.load`, the age-gate, dismissals,
    /// warmup, paginate, and the settle / hard-timeout timers; feed each
    /// captured exchange through the same `applyMatchingRules` path the live
    /// JS bridge uses, then finish. The WKWebView is still constructed so
    /// consumer code that touches `engine.webView` doesn't crash — it just
    /// never navigates.
    ///
    /// Phase ordering during replay: `.loading` → `.paginating(0, 0)` →
    /// `.settling` → `.done`. Skipped: `.ageGate`, `.dismissing`,
    /// `.warmupClicks`. The transitions are mostly cosmetic — a Phase F
    /// status strip can render them — and `.done` is the only one that
    /// matters for completion.
    private func startReplay() throws {
        guard let replayer else { return }
        progress.setPhase(.loading)
        progress.setPhase(.paginating(iteration: 0, maxIterations: 0))
        for capture in replayer.captures {
            captures.append(capture)
            progress.noteCapture(responseURL: capture.responseUrl)
            if capture.kind == .document {
                // M9: route synthesized document captures to the
                // documentCapture rule rather than the XHR pipeline.
                if let docRule = recipe.browser?.documentCapture {
                    applyDocumentRule(docRule, capture: capture)
                }
            } else if capture.kind != .diagnostic {
                applyMatchingRules(to: capture)
            }
        }
        progress.setPhase(.settling)
        finish(reason: "settled")
    }

    /// Seed `seedCookies` into both `HTTPCookieStorage.shared` and the
    /// WKWebView's data store, then invoke `then`. WKHTTPCookieStore writes
    /// are async (callback-based), so we chain them and call back on
    /// completion. If `seedCookies` is empty, calls back synchronously.
    private func installSeedCookies(forURL url: URL, then: @escaping () -> Void) {
        if seedCookies.isEmpty { then(); return }
        let host = url.host ?? ""
        let httpCookies: [HTTPCookie] = seedCookies.compactMap { sc in
            var props: [HTTPCookiePropertyKey: Any] = [
                .name: sc.name,
                .value: sc.value,
                .path: sc.path ?? "/",
            ]
            // Cookie domain: prefer the recipe's; else fall back to the
            // target host so the cookie is at least scoped to this navigation.
            props[.domain] = sc.domain ?? host
            return HTTPCookie(properties: props)
        }
        for c in httpCookies { HTTPCookieStorage.shared.setCookie(c) }
        let store = webView.configuration.websiteDataStore.httpCookieStore
        var remaining = httpCookies
        func step() {
            guard !remaining.isEmpty else { then(); return }
            let next = remaining.removeFirst()
            store.setCookie(next) { step() }
        }
        step()
    }

    private func resetSettleTimer() {
        settleTimer?.invalidate()
        // M10: bootstrap-mode runs wait for the human to click the
        // overlay, not for the page to stop fetching. The hard timer
        // remains as a safety net.
        if isInteractiveBootstrap {
            settleTimer = nil
            return
        }
        settleTimer = Timer.scheduledTimer(withTimeInterval: settleSeconds, repeats: false) { [weak self] _ in
            self?.finish(reason: "settled")
        }
    }

    private func finish(reason: String, navFailURL: String? = nil) {
        guard let cont = continuation else { return }
        continuation = nil
        let stallReason = stallReasonString(reason: reason, navFailURL: navFailURL)
        if reason == "settled" {
            progress.setPhase(.done)
        } else {
            progress.setPhase(.failed(stallReason))
        }
        settleTimer?.invalidate(); settleTimer = nil
        hardTimer?.invalidate(); hardTimer = nil

        if reason == "cancelled" {
            cont.resume(returning: buildRunResult(stallReason: stallReason, unhandledAffordances: []))
            return
        }

        // M9: if the recipe declared a `captures.document` rule, evaluate
        // `document.documentElement.outerHTML` and feed it through the
        // rule before finalizing. The rule's emissions accumulate into
        // the same `collector` the XHR rules write to.
        let runDocumentCapture: () -> Void = { [weak self] in
            guard let self else { return }
            self.webView.evaluateJavaScript(InjectedScripts.dumpAffordances) { [weak self] result, _ in
                MainActor.assumeIsolated {
                    guard let self else { return }
                    let unhandled = Self.parseUnhandledAffordances(
                        jsResult: result,
                        additionalHandledLabels: self.recipe.browser?.warmupClicks ?? []
                    )
                    cont.resume(returning: self.buildRunResult(
                        stallReason: stallReason,
                        unhandledAffordances: unhandled
                    ))
                }
            }
        }

        // In replay mode the document capture (if any) is already in the
        // replayer's captures list and was processed during startReplay.
        // Don't re-evaluate JS against an unloaded WebView. M10 interactive
        // bootstrap also runs the document rule manually before reaching
        // `finish`, so skip the second pass.
        if let docRule = recipe.browser?.documentCapture,
           replayer == nil,
           !isInteractiveBootstrap {
            captureDocumentBody(rule: docRule, then: runDocumentCapture)
        } else {
            runDocumentCapture()
        }
    }

    /// Evaluate `document.documentElement.outerHTML`, synthesize a
    /// `Capture` for it (kind = `.document`), and run the recipe's
    /// `captures.document { … }` rule against it. The synthetic capture
    /// is also appended to `captures` so it shows up in the archived
    /// captures.jsonl alongside fetch/XHR exchanges — that lets a later
    /// `BrowserReplayer` reconstruct the same extraction without a
    /// fresh navigation.
    private func captureDocumentBody(rule: DocumentCaptureRule, then continuation: @escaping () -> Void) {
        let currentURL = webView.url?.absoluteString ?? ""
        webView.evaluateJavaScript("document.documentElement.outerHTML") { [weak self] result, _ in
            MainActor.assumeIsolated {
                guard let self else { continuation(); return }
                let body = (result as? String) ?? ""
                let synthetic = Capture(
                    timestamp: Date(),
                    kind: .document,
                    method: "GET",
                    requestUrl: currentURL,
                    responseUrl: currentURL,
                    requestBody: "",
                    status: 200,
                    bodyLength: body.utf8.count,
                    body: body
                )
                self.captures.append(synthetic)
                self.applyDocumentRule(rule, capture: synthetic)
                self.progress.setRecordsEmitted(self.collector.records.count)
                continuation()
            }
        }
    }

    /// Apply a `DocumentCaptureRule` to a `.document`-kind capture. The
    /// capture body is parsed as HTML once and bound as `$.` so the
    /// recipe can walk it with `select(...)` directly (no need for an
    /// inner `parseHtml`).
    fileprivate func applyDocumentRule(_ rule: DocumentCaptureRule, capture: Capture) {
        let parsed: JSONValue
        do {
            let doc = try SwiftSoup.parse(capture.body)
            parsed = .node(HTMLNode(doc))
        } catch {
            parsed = .string(capture.body)
        }
        let captureScope = scope.withCurrent(parsed)
        do {
            try runCaptureBody(rule.body, scope: captureScope)
        } catch {
            // Document-rule errors don't terminate the run; the
            // diagnostic report surfaces partial output.
        }
    }

    private func buildRunResult(stallReason: String, unhandledAffordances: [String]) -> RunResult {
        let snapshot = Snapshot(records: collector.records, observedAt: Date())
        let report = DiagnosticReport(
            stallReason: stallReason,
            unmatchedCaptures: projectedUnmatchedCaptures(),
            unfiredRules: unfiredRulePatterns(),
            unmetExpectations: ExpectationEvaluator.evaluate(recipe.expectations, against: snapshot),
            unhandledAffordances: unhandledAffordances
        )
        return RunResult(snapshot: snapshot, report: report)
    }

    private func stallReasonString(reason: String, navFailURL: String?) -> String {
        switch reason {
        case "settled", "hard-timeout", "cancelled":
            return reason
        case "nav-fail":
            return "navigation-failed: \(navFailURL ?? "")"
        case "session-expired":
            return "session-expired: re-run with --interactive to refresh"
        default:
            return reason
        }
    }

    /// Browser runs on a long-lived SPA can produce thousands of captures.
    /// 50 is a soft cap that keeps the report bounded for logging / JSON
    /// dumps while still surfacing the *most recent* endpoints the recipe
    /// didn't cover — those are the actionable ones for the recipe author.
    private static let unmatchedCaptureCap = 50

    /// Cap on `unhandledAffordances` for the same reason — a stalled run on
    /// an SPA can have hundreds of visible buttons; 50 is plenty to surface
    /// the missed pagination targets without blowing up the report.
    nonisolated static let unhandledAffordanceCap = 50

    /// Case-insensitive substrings that mark a button/link as
    /// pagination-shaped. Matches the page's *visible label*; e.g. a button
    /// whose textContent is "Show more results" matches "show more".
    nonisolated static let paginationKeywords: [String] = [
        "view more", "load more", "next page", "show more", "see more",
        "more results", "older", "next ›", "›", "→"
    ]

    /// Labels the engine's built-in `scrollAndClickLoadMore` JS clicks on
    /// every pagination iteration. A match here is considered *handled* —
    /// the engine actively drove the affordance. Keep in sync with the
    /// `loadMoreLabels` array in `InjectedScripts.scrollAndClickLoadMore`.
    nonisolated static let engineClickedLabels: [String] = [
        "shop all products", "show more", "view more", "load more", "see more", "view all"
    ]

    private func projectedUnmatchedCaptures() -> [UnmatchedCapture] {
        unmatchedCaptures.map {
            UnmatchedCapture(
                url: $0.responseUrl,
                method: $0.method,
                status: $0.status,
                bodyBytes: $0.body.utf8.count
            )
        }
    }

    private func unfiredRulePatterns() -> [String] {
        guard let bcfg = recipe.browser else { return [] }
        return bcfg.captures
            .map(\.urlPattern)
            .filter { (ruleMatchCounts[$0] ?? 0) == 0 }
    }

    /// Parse the JSON string `InjectedScripts.dumpAffordances` returns, run
    /// it through `unhandledAffordances(items:additionalHandledLabels:)`, and
    /// fall back to `[]` on any parse / shape error. `jsResult` is whatever
    /// `WKWebView.evaluateJavaScript` handed back — typically a `String`,
    /// but also `nil` (empty page) or an unexpected `NSError`-bearing
    /// callback. All non-string paths return `[]`.
    nonisolated static func parseUnhandledAffordances(
        jsResult: Any?,
        additionalHandledLabels: [String]
    ) -> [String] {
        guard let s = jsResult as? String, let data = s.data(using: .utf8) else { return [] }
        guard let dump = try? JSONDecoder().decode(AffordanceDump.self, from: data) else { return [] }
        let items = dump.buttons + dump.links + dump.roleButtons
        return unhandledAffordances(items: items, additionalHandledLabels: additionalHandledLabels)
    }

    /// Pure filter / dedup / cap pipeline — given a flat list of dumped
    /// affordances and any extra labels the recipe declared as handled,
    /// returns the formatted strings to surface in `DiagnosticReport`.
    /// Tested directly without a WKWebView.
    nonisolated static func unhandledAffordances(
        items: [AffordanceItem],
        additionalHandledLabels: [String]
    ) -> [String] {
        let handled = Set(
            (engineClickedLabels + additionalHandledLabels).map { $0.lowercased() }
        )
        var out: [String] = []
        var seen = Set<String>()
        for item in items {
            let text = item.text.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !text.isEmpty else { continue }
            let lower = text.lowercased()
            guard paginationKeywords.contains(where: { lower.contains($0) }) else { continue }
            if handled.contains(lower) { continue }
            let formatted: String = {
                if let sel = item.selector, !sel.isEmpty { return "\(text) (\(sel))" }
                return text
            }()
            if seen.insert(formatted).inserted {
                out.append(formatted)
                if out.count >= unhandledAffordanceCap { break }
            }
        }
        return out
    }

    private func applyMatchingRules(to capture: Capture) {
        let bcfg = recipe.browser!
        var matched = false
        for rule in bcfg.captures where capture.responseUrl.contains(rule.urlPattern) {
            matched = true
            ruleMatchCounts[rule.urlPattern, default: 0] += 1
            applyRule(rule, capture: capture)
        }
        if !matched {
            unmatchedCaptures.append(capture)
            if unmatchedCaptures.count > Self.unmatchedCaptureCap {
                unmatchedCaptures.removeFirst()
            }
        }
        progress.setRecordsEmitted(collector.records.count)
    }

    private func applyRule(_ rule: CaptureRule, capture: Capture) {
        guard let bodyData = capture.body.data(using: .utf8),
              let json = try? JSONValue.decode(bodyData) else { return }
        let captureScope = scope.withCurrent(json)
        do {
            try runCaptureBody(rule.body, scope: captureScope)
        } catch {
            // Capture-rule errors are surfaced via DiagnosticReport in
            // a later phase; for now, swallow so a single bad capture
            // doesn't terminate the whole run.
        }
    }

    private func runCaptureBody(_ stmts: [Statement], scope: Scope) throws {
        for stmt in stmts {
            switch stmt {
            case .emit(let em):
                let record = try evaluator.emit(em, in: scope)
                collector.append(record)
            case .forLoop(let varName, let coll, let body):
                let listValue = try evaluator.evaluateToJSON(coll, in: scope)
                let items: [JSONValue]
                switch listValue {
                case .array(let xs): items = xs
                case .null: items = []
                default: items = [listValue]
                }
                for item in items {
                    let inner = scope.with(varName, item).withCurrent(item)
                    try runCaptureBody(body, scope: inner)
                }
            case .step:
                continue
            }
        }
    }

    // MARK: - Navigation delegate

    public func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        didFinishNav = true
        progress.setCurrentURL(webView.url?.absoluteString)
        guard let bcfg = recipe.browser else { return }

        // M10 bootstrap mode: inject the overlay button and stop here.
        // The settle timer is cancelled — the human's "Scrape this page"
        // click is the trigger for completion, not idle settle.
        if isInteractiveBootstrap {
            settleTimer?.invalidate(); settleTimer = nil
            webView.evaluateJavaScript(InjectedScripts.interactiveOverlay) { _, _ in }
            return
        }

        // M10 reuse mode: restore cached localStorage for the current
        // origin, then check whether the document body matches the
        // recipe's gate pattern — if so, the session is expired and we
        // need a human re-bootstrap.
        if let cached = cachedInteractiveSession,
           let origin = webView.url.flatMap(Self.origin(of:)),
           let storage = cached.localStorage[origin],
           !storage.isEmpty,
           let json = try? String(data: JSONEncoder().encode(storage), encoding: .utf8) {
            webView.evaluateJavaScript(InjectedScripts.restoreLocalStorage(json)) { _, _ in }
        }
        if let pattern = bcfg.interactive?.sessionExpiredPattern {
            webView.evaluateJavaScript("document.documentElement.outerHTML") { [weak self] result, _ in
                MainActor.assumeIsolated {
                    guard let self else { return }
                    let body = (result as? String) ?? ""
                    if body.contains(pattern) {
                        // Session is no longer valid — drop the cache so
                        // the next run bootstraps fresh.
                        InteractiveSessionStore.evict(
                            at: InteractiveSessionStore.file(
                                for: self.recipe.name,
                                root: self.interactiveSessionRoot
                            )
                        )
                        self.finish(reason: "session-expired")
                        return
                    }
                    self.afterNavigationContinue(bcfg: bcfg)
                }
            }
            return
        }

        afterNavigationContinue(bcfg: bcfg)
    }

    /// Shared post-navigation flow that branches into the dismissal/
    /// warmup/pagination dance. Extracted so the M10 reuse-mode gate
    /// check can call it as a continuation after the document HTML is
    /// confirmed gate-free.
    private func afterNavigationContinue(bcfg: BrowserConfig) {
        if isBeforeDismissals(progress.phase) {
            if bcfg.ageGate != nil {
                progress.setPhase(.ageGate)
            } else if bcfg.dismissals != nil {
                progress.setPhase(.dismissing)
            }
        }
        if bcfg.ageGate != nil || bcfg.dismissals != nil {
            DispatchQueue.main.asyncAfter(deadline: .now() + 2.0) { [weak self] in
                self?.attemptDismissals()
            }
        } else {
            DispatchQueue.main.asyncAfter(deadline: .now() + 1.0) { [weak self] in
                self?.runWarmupAndPaginate()
            }
        }
    }

    /// `<scheme>://<host>` for the URL — the origin key under which we
    /// stash and restore `localStorage` snapshots.
    nonisolated static func origin(of url: URL) -> String? {
        guard let scheme = url.scheme, let host = url.host else { return nil }
        return "\(scheme)://\(host)"
    }

    private func isBeforeDismissals(_ phase: BrowserProgress.Phase) -> Bool {
        switch phase {
        case .starting, .loading, .ageGate, .dismissing: return true
        default: return false
        }
    }

    public func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: any Error) {
        finish(reason: "nav-fail", navFailURL: webView.url?.absoluteString)
    }

    public func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: any Error) {
        finish(reason: "nav-fail", navFailURL: webView.url?.absoluteString)
    }

    // MARK: - Dismissal orchestration

    private func attemptDismissals() {
        let bcfg = recipe.browser!
        let maxAttempts = bcfg.dismissals?.maxAttempts ?? 8
        guard dismissAttempts < maxAttempts else {
            runWarmupAndPaginate()
            return
        }
        dismissAttempts += 1

        // Pass 1: age gate
        if bcfg.ageGate != nil {
            webView.evaluateJavaScript(InjectedScripts.ageGateFill) { [weak self] result, _ in
                guard let self else { return }
                if let s = result as? String, !s.isEmpty {
                    DispatchQueue.main.asyncAfter(deadline: .now() + 2.0) {
                        if bcfg.ageGate?.reloadAfter == true { self.webView.reload() }
                    }
                    DispatchQueue.main.asyncAfter(deadline: .now() + 4.0) { [weak self] in
                        self?.dismissAttempts = 0
                        self?.attemptDismissals()
                    }
                    return
                }
                self.attemptModalDismiss()
            }
            return
        }
        attemptModalDismiss()
    }

    private func attemptModalDismiss() {
        if case .ageGate = progress.phase {
            progress.setPhase(.dismissing)
        }
        webView.evaluateJavaScript(InjectedScripts.dismissModal) { [weak self] result, _ in
            guard let self else { return }
            if let s = result as? String, !s.isEmpty {
                DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) { [weak self] in
                    self?.attemptDismissals()
                }
            } else {
                self.runWarmupAndPaginate()
            }
        }
    }

    // MARK: - Warmup + paginate

    private func runWarmupAndPaginate() {
        guard !didFireWarmup else { return }
        didFireWarmup = true
        let labels = recipe.browser?.warmupClicks ?? []
        if !labels.isEmpty {
            progress.setPhase(.warmupClicks)
        }
        clickWarmup(labels: labels) { [weak self] in
            DispatchQueue.main.asyncAfter(deadline: .now() + 2.0) { [weak self] in
                self?.paginate?.start()
            }
        }
    }

    private func clickWarmup(labels: [String], completion: @escaping () -> Void) {
        var remaining = labels
        func step() {
            guard !remaining.isEmpty else { completion(); return }
            let label = remaining.removeFirst()
            webView.evaluateJavaScript(InjectedScripts.clickButtonByText(label)) { _, _ in
                DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) { step() }
            }
        }
        step()
    }

    // MARK: - WKScriptMessageHandler

    public nonisolated func userContentController(_ ucc: WKUserContentController, didReceive message: WKScriptMessage) {
        let name = message.name
        let body = message.body
        MainActor.assumeIsolated {
            // M10: the human clicked "Scrape this page" in the overlay.
            // Snapshot cookies + localStorage, persist the session, run
            // the documentCapture rule against the supplied HTML, finish.
            if name == "forageInteractiveDone" {
                guard let dict = body as? [String: Any] else { return }
                let urlStr = (dict["url"] as? String) ?? ""
                let html = (dict["html"] as? String) ?? ""
                self.completeInteractiveBootstrap(currentURL: urlStr, documentHTML: html)
                return
            }
            // Default path: capture-network bridge from injected JS.
            guard let dict = body as? [String: Any],
                  let cap = Capture(jsBridgePayload: dict) else { return }
            captures.append(cap)
            progress.noteCapture(responseURL: cap.responseUrl)
            applyMatchingRules(to: cap)
            resetSettleTimer()
            paginate?.handleCapture(cap)
        }
    }

    /// Snapshot cookies + localStorage, persist as `InteractiveSession`,
    /// synthesize a `.document` Capture, route through `documentCapture`
    /// if the recipe declared one, and finish the run.
    private func completeInteractiveBootstrap(currentURL: String, documentHTML: String) {
        let interactive = recipe.browser?.interactive
        let cookieDomains = interactive?.cookieDomains ?? []
        let slug = recipe.name

        webView.evaluateJavaScript(InjectedScripts.dumpLocalStorage) { [weak self] localStorageJSON, _ in
            MainActor.assumeIsolated {
                guard let self else { return }
                let origin = URL(string: currentURL).flatMap(Self.origin(of:)) ?? ""
                let storageMap: [String: String] = {
                    if let s = localStorageJSON as? String,
                       let d = s.data(using: .utf8),
                       let parsed = try? JSONSerialization.jsonObject(with: d) as? [String: String] {
                        return parsed
                    }
                    return [:]
                }()
                self.webView.configuration.websiteDataStore.httpCookieStore.getAllCookies { httpCookies in
                    MainActor.assumeIsolated {
                        let filtered = httpCookies.filter { c in
                            cookieDomains.isEmpty || cookieDomains.contains { c.domain.contains($0) }
                        }
                        let sessionCookies = filtered.map { c in
                            SessionCookie(name: c.name, value: c.value, domain: c.domain, path: c.path)
                        }
                        // Best-effort expiry hint: nearest cookie expiry
                        // within the kept set.
                        let earliest = filtered
                            .compactMap { $0.expiresDate }
                            .min()
                        let session = InteractiveSession(
                            recipeSlug: slug,
                            bootstrappedAt: Date(),
                            expiresAt: earliest,
                            cookies: sessionCookies,
                            localStorage: storageMap.isEmpty ? [:] : [origin: storageMap]
                        )
                        let dest = InteractiveSessionStore.file(for: slug, root: self.interactiveSessionRoot)
                        try? InteractiveSessionStore.write(session, to: dest)

                        // Synthesize a .document capture for the rule
                        // pipeline so the recipe extracts records the
                        // same way it would on a headless reuse run.
                        let synth = Capture(
                            timestamp: Date(),
                            kind: .document,
                            method: "GET",
                            requestUrl: currentURL,
                            responseUrl: currentURL,
                            requestBody: "",
                            status: 200,
                            bodyLength: documentHTML.utf8.count,
                            body: documentHTML
                        )
                        self.captures.append(synth)
                        if let docRule = self.recipe.browser?.documentCapture {
                            self.applyDocumentRule(docRule, capture: synth)
                        }
                        self.progress.setRecordsEmitted(self.collector.records.count)
                        self.finish(reason: "settled")
                    }
                }
            }
        }
    }

    // MARK: - BrowserPaginateHost

    public func paginateLog(_ message: String) {
        // No-op by default; consumer subclasses can override / hook a logger.
    }

    public func paginateEvalJS(_ js: String, completion: @MainActor @Sendable @escaping (Any?, Error?) -> Void) {
        webView.evaluateJavaScript(js) { result, err in
            MainActor.assumeIsolated { completion(result, err) }
        }
    }

    public func paginateCountCaptures(matching pattern: String) -> Int {
        captures.reduce(0) { acc, c in
            c.responseUrl.contains(pattern) ? acc + 1 : acc
        }
    }

    public func paginateIterationStarted(iteration: Int, maxIterations: Int) {
        progress.setPhase(.paginating(iteration: iteration, maxIterations: maxIterations))
    }

    public func paginateDidFinish() {
        progress.setPhase(.settling)
    }
}

/// One row from `InjectedScripts.dumpAffordances`. The JS emits five
/// buckets (buttons, links, role=button, scrollables, inputs); only the
/// first three are pagination-shaped and end up in `AffordanceDump`'s
/// payload after filtering. Other fields the JS emits (`x`, `y`, `href`,
/// `type`, etc.) are decoded as `nil` and dropped — we only care about
/// the label and a CSS-style locator.
public struct AffordanceItem: Sendable, Hashable, Codable {
    public let selector: String?
    public let text: String
}

/// Top-level shape of the JSON `InjectedScripts.dumpAffordances` returns.
/// Mirrors the JS object keys exactly; only the three actionable buckets
/// participate in unhandled-affordance scoring.
struct AffordanceDump: Decodable {
    let buttons: [AffordanceItem]
    let links: [AffordanceItem]
    let roleButtons: [AffordanceItem]
}

public enum BrowserEngineError: Error, CustomStringConvertible {
    case invalidInitialURL(String)
    case nsApplicationNotRunning

    public var description: String {
        switch self {
        case .invalidInitialURL(let s): return "browser engine: initial URL didn't parse: \(s)"
        case .nsApplicationNotRunning: return "browser engine: NSApplication event loop not running"
        }
    }
}

#endif // canImport(WebKit)
