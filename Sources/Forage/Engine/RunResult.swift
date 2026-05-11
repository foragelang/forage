import Foundation

/// The full output of a recipe run. Carries the extracted `Snapshot` plus a
/// `DiagnosticReport` explaining how the run terminated and what (if
/// anything) the engine couldn't account for. Engines return `RunResult`
/// instead of `Snapshot` directly so the consumer never has to guess why a
/// run came back smaller than expected — every short run carries its own
/// receipts.
public struct RunResult: Sendable, Hashable {
    public let snapshot: Snapshot
    public let report: DiagnosticReport

    public init(snapshot: Snapshot, report: DiagnosticReport) {
        self.snapshot = snapshot
        self.report = report
    }
}
