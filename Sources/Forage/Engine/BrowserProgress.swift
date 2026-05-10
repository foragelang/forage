import Foundation

/// Live progress signal a `BrowserEngine` exposes for the duration of a run.
///
/// The engine owns one of these and drives all mutations as it walks through
/// loading → age-gate dismissal → warmup → pagination → settle → done.
/// Consumers (a SwiftUI status panel, a CLI log line, a test) read it.
/// The class is `@Observable` so SwiftUI tracking sees per-field changes
/// without a manual `@Published` story.
///
/// Mutators are `internal` — only the engine in the same module drives
/// transitions. External code is read-only.
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

    public init() {}

    internal func setPhase(_ phase: Phase) {
        self.phase = phase
    }

    internal func noteCapture(responseURL: String) {
        capturesObserved += 1
        lastObservedURL = responseURL
    }

    internal func setRecordsEmitted(_ count: Int) {
        recordsEmitted = count
    }

    internal func setCurrentURL(_ url: String?) {
        currentURL = url
    }
}
