import SwiftUI
import Forage

/// Renders `DiagnosticReport` sections for the most recent run. Disclosure
/// groups for each non-empty array; `stallReason` always shown at top.
struct DiagnosticTab: View {
    let slug: String
    @Environment(RunResultStore.self) private var runResults

    var body: some View {
        Group {
            if let report = runResults.report(for: slug) {
                content(report)
            } else {
                ContentUnavailableView(
                    "No diagnostic yet",
                    systemImage: "stethoscope",
                    description: Text("Use \"Run live\" or \"Run replay\" to produce a diagnostic report.")
                )
            }
        }
    }

    private func content(_ report: DiagnosticReport) -> some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                HStack(alignment: .firstTextBaseline) {
                    Text("Stall reason")
                        .font(.headline)
                    Text(report.stallReason)
                        .font(.system(.body, design: .monospaced))
                        .padding(.horizontal, 6)
                        .padding(.vertical, 2)
                        .background(
                            RoundedRectangle(cornerRadius: 4)
                                .fill(Color.secondary.opacity(0.15))
                        )
                    Spacer()
                }

                if !report.unmetExpectations.isEmpty {
                    section("Unmet expectations", count: report.unmetExpectations.count) {
                        ForEach(Array(report.unmetExpectations.enumerated()), id: \.offset) { _, line in
                            Text(line)
                                .font(.system(size: 12, design: .monospaced))
                                .textSelection(.enabled)
                        }
                    }
                }

                if !report.unfiredRules.isEmpty {
                    section("Unfired capture rules", count: report.unfiredRules.count) {
                        ForEach(Array(report.unfiredRules.enumerated()), id: \.offset) { _, rule in
                            Text(rule)
                                .font(.system(size: 12, design: .monospaced))
                                .textSelection(.enabled)
                        }
                    }
                }

                if !report.unmatchedCaptures.isEmpty {
                    section("Unmatched captures", count: report.unmatchedCaptures.count) {
                        ForEach(Array(report.unmatchedCaptures.enumerated()), id: \.offset) { _, cap in
                            HStack(alignment: .firstTextBaseline) {
                                Text(cap.method)
                                    .font(.system(size: 11, weight: .semibold, design: .monospaced))
                                Text("\(cap.status)")
                                    .font(.system(size: 11, design: .monospaced))
                                    .foregroundStyle(.secondary)
                                Text(cap.url)
                                    .font(.system(size: 12, design: .monospaced))
                                    .textSelection(.enabled)
                                    .lineLimit(1)
                                    .truncationMode(.middle)
                                Spacer()
                                Text("\(cap.bodyBytes) B")
                                    .font(.system(size: 10, design: .monospaced))
                                    .foregroundStyle(.tertiary)
                            }
                        }
                    }
                }

                if !report.unhandledAffordances.isEmpty {
                    section("Unhandled affordances", count: report.unhandledAffordances.count) {
                        ForEach(Array(report.unhandledAffordances.enumerated()), id: \.offset) { _, a in
                            Text(a)
                                .font(.system(size: 12, design: .monospaced))
                                .textSelection(.enabled)
                        }
                    }
                }

                if report.unmetExpectations.isEmpty &&
                   report.unfiredRules.isEmpty &&
                   report.unmatchedCaptures.isEmpty &&
                   report.unhandledAffordances.isEmpty {
                    HStack(spacing: 6) {
                        Image(systemName: "checkmark.seal.fill").foregroundStyle(.green)
                        Text("Run settled cleanly — no issues to surface.")
                            .foregroundStyle(.secondary)
                    }
                    .padding(.vertical, 6)
                }
            }
            .padding(14)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    @ViewBuilder
    private func section<Content: View>(_ title: String, count: Int, @ViewBuilder content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 6) {
                Text(title)
                    .font(.headline)
                Text("(\(count))")
                    .foregroundStyle(.secondary)
            }
            VStack(alignment: .leading, spacing: 4) {
                content()
            }
        }
    }
}
