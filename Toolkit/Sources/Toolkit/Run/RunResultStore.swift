import Foundation
import SwiftUI
import Forage

/// Per-slug last-run result. Keyed by slug so navigating between recipes
/// preserves each one's most recent snapshot + diagnostic.
@MainActor
@Observable
final class RunResultStore {
    private(set) var resultsBySlug: [String: RunResult] = [:]

    func setLatest(_ result: RunResult, for slug: String) {
        resultsBySlug[slug] = result
    }

    func snapshot(for slug: String) -> Snapshot? {
        resultsBySlug[slug]?.snapshot
    }

    func report(for slug: String) -> DiagnosticReport? {
        resultsBySlug[slug]?.report
    }
}
