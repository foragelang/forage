import SwiftUI
import AppKit
import Forage

/// Source-tab editor. `NSTextView` wrapped in `NSViewRepresentable` with
/// debounced syntax highlighting + inline validation panel below.
struct SourceTab: View {
    @Binding var source: String
    let onSave: () -> Void

    @State private var debouncedSource: String = ""
    @State private var validationIssues: [ValidationIssue] = []
    @State private var parseError: String?

    var body: some View {
        VSplitView {
            ForageTextEditor(text: $source)
                .frame(minHeight: 200)

            validationPanel
                .frame(minHeight: 80, idealHeight: 140, maxHeight: 280)
        }
        .onChange(of: source) { _, _ in
            scheduleValidation()
        }
        .task {
            debouncedSource = source
            runValidation()
        }
        .onReceive(NotificationCenter.default.publisher(for: .toolkitSave)) { _ in
            onSave()
        }
    }

    private var validationPanel: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 6) {
                Text("Validation")
                    .font(.headline)
                Spacer()
                Button("Save") { onSave() }
                    .keyboardShortcut("s", modifiers: [.command])
                    .help("Save the current source to disk")
            }
            .padding(.horizontal, 8)
            .padding(.top, 6)

            ScrollView {
                LazyVStack(alignment: .leading, spacing: 3) {
                    if let parseError {
                        IssueRow(severity: .error, message: parseError, location: "parse")
                    }
                    ForEach(Array(validationIssues.enumerated()), id: \.offset) { _, issue in
                        IssueRow(
                            severity: issue.severity == .error ? .error : .warning,
                            message: issue.message,
                            location: issue.location
                        )
                    }
                    if parseError == nil && validationIssues.isEmpty {
                        Text("No issues.")
                            .foregroundStyle(.secondary)
                            .font(.system(size: 11))
                            .padding(8)
                    }
                }
                .padding(.horizontal, 8)
                .padding(.bottom, 6)
            }
        }
        .background(
            RoundedRectangle(cornerRadius: 4)
                .fill(Color(nsColor: .textBackgroundColor))
                .opacity(0.4)
        )
    }

    private func scheduleValidation() {
        let snapshot = source
        Task {
            try? await Task.sleep(nanoseconds: 500_000_000)
            await MainActor.run {
                if snapshot == source && snapshot != debouncedSource {
                    debouncedSource = snapshot
                    runValidation()
                }
            }
        }
    }

    private func runValidation() {
        let src = debouncedSource
        if src.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            parseError = nil
            validationIssues = []
            return
        }
        do {
            let recipe = try Parser.parse(source: src)
            parseError = nil
            validationIssues = Validator.validate(recipe)
        } catch {
            parseError = "\(error)"
            validationIssues = []
        }
    }
}

private struct IssueRow: View {
    enum Severity { case error, warning }
    let severity: Severity
    let message: String
    let location: String

    var body: some View {
        HStack(alignment: .top, spacing: 6) {
            Image(systemName: severity == .error
                  ? "xmark.octagon.fill"
                  : "exclamationmark.triangle.fill")
                .foregroundStyle(severity == .error ? Color.red : Color.orange)
                .font(.system(size: 11))
                .padding(.top, 2)
            VStack(alignment: .leading, spacing: 1) {
                Text(message)
                    .font(.system(size: 12))
                    .textSelection(.enabled)
                Text(location)
                    .font(.system(size: 10, design: .monospaced))
                    .foregroundStyle(.secondary)
            }
            Spacer()
        }
        .padding(.vertical, 2)
    }
}
