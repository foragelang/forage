import Foundation

/// Live progress signal an `HTTPEngine` exposes for the duration of a run.
///
/// Sibling of `BrowserProgress`. The engine owns one of these and drives all
/// mutations as it walks through priming (htmlPrime auth, if configured),
/// step execution, pagination, and emission. Consumers (a SwiftUI status
/// panel, a CLI log line, a test) read it.
///
/// The class is `@Observable` so SwiftUI tracking sees per-field changes
/// without a manual `@Published` story. Mutators are `internal` — only the
/// engine in the same module drives transitions. External code is read-only.
@MainActor
@Observable
public final class HTTPProgress {
    public enum Phase: Sendable, Hashable {
        case starting
        case priming
        case stepping(name: String)
        case paginating(name: String, page: Int)
        case done
        case failed(String)
    }

    public private(set) var phase: Phase = .starting
    public private(set) var requestsSent: Int = 0
    public private(set) var recordsEmitted: Int = 0
    public private(set) var currentURL: String? = nil

    /// True once the run has reached a terminal phase. Mutators that race
    /// against `.done` / `.failed` (e.g. a late pagination iteration emitted
    /// after a hard error already fired) check this to avoid rewriting the
    /// terminal state.
    public var isTerminal: Bool {
        switch phase {
        case .done, .failed: return true
        default: return false
        }
    }

    public nonisolated init() {}

    internal func setPhase(_ phase: Phase) {
        self.phase = phase
    }

    internal func noteRequestSent(url: String?) {
        requestsSent += 1
        currentURL = url
    }

    internal func setRecordsEmitted(_ count: Int) {
        recordsEmitted = count
    }

    /// Reset to the initial state. Used by `RecipeRunner` between runs so a
    /// long-lived runner can be observed across multiple invocations without
    /// the consumer needing to grab a new reference each time.
    internal func reset() {
        phase = .starting
        requestsSent = 0
        recordsEmitted = 0
        currentURL = nil
    }
}
