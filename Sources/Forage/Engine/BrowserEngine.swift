#if canImport(WebKit)
import Foundation
import AppKit
import WebKit

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
@MainActor
public final class BrowserEngine: NSObject, WKNavigationDelegate, WKScriptMessageHandler, BrowserPaginateHost {
    public let recipe: Recipe
    public let inputs: [String: JSONValue]
    public let evaluator: ExtractionEvaluator
    public let visible: Bool
    public let settleSeconds: TimeInterval
    public let hardTimeoutSeconds: TimeInterval
    public private(set) var webView: WKWebView!

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
        replayer: BrowserReplayer? = nil
    ) {
        precondition(recipe.engineKind == .browser, "BrowserEngine requires browser-engine Recipe")
        precondition(recipe.browser != nil, "browser-engine Recipe must have a `browser { … }` block")
        self.recipe = recipe
        self.inputs = inputs
        self.evaluator = evaluator
        self.visible = visible
        self.settleSeconds = settleSeconds
        self.hardTimeoutSeconds = hardTimeoutSeconds
        self.replayer = replayer
        self.scope = Scope(inputs: inputs, frames: [[:]], current: nil)
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
    /// reasons (settled / hard-timeout / nav-fail) come back through
    /// `report.stallReason` instead.
    public func run() async throws -> RunResult {
        try await withCheckedThrowingContinuation { (cont: CheckedContinuation<RunResult, any Error>) in
            self.continuation = cont
            do {
                try start()
            } catch {
                progress.setPhase(.failed("\(error)"))
                cont.resume(throwing: error)
            }
        }
    }

    private func start() throws {
        if replayer != nil {
            try startReplay()
            return
        }

        let bcfg = recipe.browser!
        let url = try TemplateRenderer.render(bcfg.initialURL, in: scope)
        guard let urlValue = URL(string: url) else {
            throw BrowserEngineError.invalidInitialURL(url)
        }

        if visible {
            let win = NSWindow(
                contentRect: NSRect(x: 60, y: 60, width: 1280, height: 900),
                styleMask: [.titled, .closable, .resizable],
                backing: .buffered,
                defer: false
            )
            win.title = "Forage — \(recipe.name)"
            win.contentView = webView
            win.makeKeyAndOrderFront(nil)
            self.window = win
        }

        progress.setPhase(.loading)
        progress.setCurrentURL(url)
        webView.load(URLRequest(url: urlValue))

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
            applyMatchingRules(to: capture)
        }
        progress.setPhase(.settling)
        finish(reason: "settled")
    }

    private func resetSettleTimer() {
        settleTimer?.invalidate()
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
        let result = buildRunResult(stallReason: stallReason)
        cont.resume(returning: result)
    }

    private func buildRunResult(stallReason: String) -> RunResult {
        let snapshot = Snapshot(records: collector.records, observedAt: Date())
        let report = DiagnosticReport(
            stallReason: stallReason,
            unmatchedCaptures: projectedUnmatchedCaptures(),
            unfiredRules: unfiredRulePatterns(),
            unmetExpectations: ExpectationEvaluator.evaluate(recipe.expectations, against: snapshot),
            unhandledAffordances: []
        )
        return RunResult(snapshot: snapshot, report: report)
    }

    private func stallReasonString(reason: String, navFailURL: String?) -> String {
        switch reason {
        case "settled", "hard-timeout":
            return reason
        case "nav-fail":
            return "navigation-failed: \(navFailURL ?? "")"
        default:
            return reason
        }
    }

    /// Browser runs on a long-lived SPA can produce thousands of captures.
    /// 50 is a soft cap that keeps the report bounded for logging / JSON
    /// dumps while still surfacing the *most recent* endpoints the recipe
    /// didn't cover — those are the actionable ones for the recipe author.
    private static let unmatchedCaptureCap = 50

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
                let listValue = try PathResolver.resolve(coll, in: scope)
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
        let body = message.body
        MainActor.assumeIsolated {
            guard let dict = body as? [String: Any],
                  let cap = Capture(jsBridgePayload: dict) else { return }
            captures.append(cap)
            progress.noteCapture(responseURL: cap.responseUrl)
            applyMatchingRules(to: cap)
            resetSettleTimer()
            paginate?.handleCapture(cap)
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
