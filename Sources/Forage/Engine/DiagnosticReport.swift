import Foundation

/// Post-run forensics. Populated by each engine before it returns a
/// `RunResult`. Consumers read this to surface "why did the run stop?" and
/// "what didn't we extract?" without poking at engine internals.
///
/// Field semantics:
/// - `stallReason`: short tag describing how the run terminated.
///   Browser engine: `"settled"`, `"hard-timeout"`, `"navigation-failed: <url>"`.
///   HTTP engine: `"completed"` for a normal run, `"failed: <error>"` for an
///   error that interrupted the walker.
/// - `unmatchedCaptures`: browser-engine only. Captures whose response URL
///   matched none of the recipe's `captures.match` rules. Capped at the most
///   recent 50 entries to keep the report bounded on runaway SPAs.
/// - `unfiredRules`: browser-engine only. URL patterns of `captures.match`
///   rules that never matched any capture during the run. Highlights stale
///   patterns or wrong endpoints.
/// - `unmetExpectations`: filled by Phase 4's `ExpectationEvaluator`. Empty
///   for now.
/// - `unhandledAffordances`: left empty; future expansion may populate based
///   on engine-observed UI affordances.
public struct DiagnosticReport: Sendable, Hashable, Codable {
    public let stallReason: String
    public let unmatchedCaptures: [UnmatchedCapture]
    public let unfiredRules: [String]
    public let unmetExpectations: [String]
    public let unhandledAffordances: [String]

    public init(
        stallReason: String,
        unmatchedCaptures: [UnmatchedCapture] = [],
        unfiredRules: [String] = [],
        unmetExpectations: [String] = [],
        unhandledAffordances: [String] = []
    ) {
        self.stallReason = stallReason
        self.unmatchedCaptures = unmatchedCaptures
        self.unfiredRules = unfiredRules
        self.unmetExpectations = unmetExpectations
        self.unhandledAffordances = unhandledAffordances
    }
}

/// One capture the browser engine recorded but no `captures.match` rule
/// claimed. Stored as a thin projection of `Capture` — just enough for a
/// consumer to recognize the endpoint and decide whether the recipe should
/// grow a rule for it. Body itself is dropped to keep diagnostic reports
/// small and Sendable; only the byte count survives.
public struct UnmatchedCapture: Sendable, Hashable, Codable {
    public let url: String
    public let method: String
    public let status: Int
    public let bodyBytes: Int

    public init(url: String, method: String, status: Int, bodyBytes: Int) {
        self.url = url
        self.method = method
        self.status = status
        self.bodyBytes = bodyBytes
    }
}
