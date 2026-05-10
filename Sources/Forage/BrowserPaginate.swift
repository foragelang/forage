import Foundation

/// `BrowserPaginate` is the engine primitive behind the
/// `browserPaginate { mode: scroll | replay }` recipe construct documented in
/// `DESIGN.md` § "Pagination strategies." One engine, two modes:
///
/// - **`scroll`** — drive the SPA forward each iteration by scrolling +
///   clicking the bottom-most load-more-style button. The SPA's own SDK
///   fires the next-page request using its own auth tokens; the host's
///   capture wrapper records it. Cheap to author (no per-platform request
///   shape required).
/// - **`replay`** — fork the captured seed request body, apply dotted-path
///   overrides with `$i` substitution, fire via the page's own
///   `window.fetch` so Origin / cookies / CSP all match. Faster per scrape
///   when the platform allows it, but some sites (Jane / Trilogy) reject
///   `evaluateJavaScript`-injected fetches via CSP `connect-src`.
///
/// The class is host-agnostic: it talks to a `BrowserPaginateHost` to evaluate
/// JavaScript, count captures matching its `observe` pattern, and log
/// progress. The host is implemented by whichever shell is hosting WKWebView
/// (the in-app `BrowserProbe`, the standalone `forage-probe` CLI, or the
/// future production `BrowserEngine`).
@MainActor
public protocol BrowserPaginateHost: AnyObject {
    func paginateLog(_ message: String)
    func paginateEvalJS(_ js: String, completion: @MainActor @Sendable @escaping (Any?, Error?) -> Void)
    func paginateCountCaptures(matching pattern: String) -> Int
    /// Called once per pagination iteration, just after the iteration counter
    /// is bumped and before the JS dispatch. Lets the engine surface live
    /// progress (`BrowserProgress.phase = .paginating(...)`) without coupling
    /// the paginator to the progress type. Default implementation is a no-op
    /// so hosts that don't care can ignore it.
    func paginateIterationStarted(iteration: Int, maxIterations: Int)
    /// Called when pagination terminates (max iterations reached, no-progress
    /// limit hit, or `.off` mode immediately). Lets the engine flip into a
    /// `.settling` phase before the settle timer fires.
    func paginateDidFinish()
}

extension BrowserPaginateHost {
    public func paginateIterationStarted(iteration: Int, maxIterations: Int) {}
    public func paginateDidFinish() {}
}

@MainActor
public final class BrowserPaginate {
    public enum Mode: String, Sendable {
        case off
        case scroll
        case replay
    }

    public let observe: String
    public let mode: Mode
    public let replayOverride: [String: Any]
    public let seedFilter: String?
    public let maxIterations: Int
    public let noProgressLimit: Int
    public let iterationDelay: TimeInterval

    public weak var host: BrowserPaginateHost?

    public private(set) var isFinished = false
    private var iteration = 0
    private var noProgressCount = 0
    private var seedRequestUrl: String?
    private var seedRequestBody: String?
    private var captureCountAtIterStart = 0
    private var didStart = false

    public init(observe: String,
                mode: Mode,
                replayOverride: [String: Any] = [:],
                seedFilter: String? = nil,
                maxIterations: Int = 30,
                noProgressLimit: Int = 3,
                iterationDelay: TimeInterval = 1.8) {
        self.observe = observe
        self.mode = mode
        self.replayOverride = replayOverride
        self.seedFilter = seedFilter
        self.maxIterations = maxIterations
        self.noProgressLimit = noProgressLimit
        self.iterationDelay = iterationDelay
    }

    /// Called for every capture by the host. Records the seed when the first
    /// matching capture arrives (only relevant for `replay` mode but cheap
    /// for `scroll`). Once a seed is captured we never replace it — so even
    /// if our own replay response loops through here, it's ignored.
    public func handleCapture(_ capture: Capture) {
        guard !isFinished else { return }
        guard capture.responseUrl.contains(observe) else { return }
        if seedRequestBody != nil { return }
        if let filter = seedFilter, !filter.isEmpty, !capture.requestBody.contains(filter) { return }
        seedRequestUrl = capture.requestUrl.isEmpty ? capture.responseUrl : capture.requestUrl
        seedRequestBody = capture.requestBody
        let path = URL(string: capture.responseUrl)?.path ?? capture.responseUrl
        host?.paginateLog("paginate: seed captured (\(path))" + (seedFilter.map { " [filter=\($0)]" } ?? ""))
    }

    /// Called by the host once dismissals are settled and we should begin
    /// driving pagination.
    public func start() {
        guard !didStart, !isFinished, mode != .off else { return }
        didStart = true
        host?.paginateLog("paginate[\(mode.rawValue)]: starting (observe=\(observe))")
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.0) { [weak self] in
            self?.tickOnce()
        }
    }

    private func tickOnce() {
        guard !isFinished, let host else { return }
        if iteration >= maxIterations {
            host.paginateLog("paginate[\(mode.rawValue)]: max iterations reached (\(maxIterations))")
            finish()
            return
        }
        iteration += 1
        host.paginateIterationStarted(iteration: iteration, maxIterations: maxIterations)
        captureCountAtIterStart = host.paginateCountCaptures(matching: observe)

        let js: String
        switch mode {
        case .off:
            finish(); return
        case .scroll:
            js = InjectedScripts.scrollAndClickLoadMore
        case .replay:
            guard let url = seedRequestUrl, let body = seedRequestBody else {
                host.paginateLog("paginate[replay]: no seed captured yet — waiting one more tick")
                DispatchQueue.main.asyncAfter(deadline: .now() + iterationDelay) { [weak self] in
                    self?.checkProgress()
                }
                return
            }
            js = InjectedScripts.replayFork(url: url, seedBody: body, override: replayOverride, iter: iteration)
        }

        host.paginateLog("paginate[\(mode.rawValue)]: iter \(iteration)/\(maxIterations) (matches so far: \(captureCountAtIterStart))")
        host.paginateEvalJS(js) { [weak self] result, err in
            guard let self else { return }
            if let err { self.host?.paginateLog("paginate iter eval error: \(err)") }
            if let s = result as? String, !s.isEmpty {
                self.host?.paginateLog("paginate[\(self.mode.rawValue)]: \(s)")
            }
        }

        DispatchQueue.main.asyncAfter(deadline: .now() + iterationDelay) { [weak self] in
            self?.checkProgress()
        }
    }

    private func checkProgress() {
        guard !isFinished, let host else { return }
        let nowCount = host.paginateCountCaptures(matching: observe)
        if nowCount > captureCountAtIterStart {
            noProgressCount = 0
            host.paginateLog("paginate[\(mode.rawValue)]: progress (\(captureCountAtIterStart) → \(nowCount))")
        } else {
            noProgressCount += 1
            host.paginateLog("paginate[\(mode.rawValue)]: no progress (\(noProgressCount)/\(noProgressLimit))")
            if noProgressCount >= noProgressLimit {
                host.paginateLog("paginate[\(mode.rawValue)]: done — \(noProgressLimit) idle iterations in a row")
                finish()
                return
            }
        }
        tickOnce()
    }

    private func finish() {
        guard !isFinished else { return }
        isFinished = true
        host?.paginateDidFinish()
    }
}
