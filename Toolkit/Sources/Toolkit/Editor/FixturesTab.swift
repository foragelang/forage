import SwiftUI
import Forage

/// Reads `fixtures/captures.jsonl` (if present) via `BrowserReplayer` and
/// shows one row per capture. Selecting a row pretty-prints the body.
struct FixturesTab: View {
    let entry: RecipeEntry
    let onCaptureFresh: () -> Void

    @State private var captures: [Capture] = []
    @State private var selectedID: UUID?
    @State private var loadError: String?

    var body: some View {
        HSplitView {
            VStack(spacing: 0) {
                toolbar
                Divider()
                list
            }
            .frame(minWidth: 320)

            inspector
                .frame(minWidth: 360)
        }
        .task(id: entry.slug) { reload() }
        .onReceive(NotificationCenter.default.publisher(for: .toolkitFixturesChanged)) { _ in
            reload()
        }
        .alert("Couldn't read fixtures", isPresented: .constant(loadError != nil)) {
            Button("OK") { loadError = nil }
        } message: {
            Text(loadError ?? "")
        }
    }

    private var toolbar: some View {
        HStack(spacing: 6) {
            Text("\(captures.count) captures")
                .font(.caption)
                .foregroundStyle(.secondary)
            Spacer()
            Button {
                onCaptureFresh()
            } label: {
                Label("Capture fresh", systemImage: "dot.radiowaves.left.and.right")
            }
            .help("Open a WKWebView and capture fresh requests for this recipe")
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
    }

    private var list: some View {
        Group {
            if captures.isEmpty {
                ContentUnavailableView(
                    "No captures",
                    systemImage: "tray",
                    description: Text("Use \"Capture fresh\" to record requests, or place a `captures.jsonl` under \(entry.capturesFile.path).")
                )
            } else {
                List(captures, id: \.id, selection: $selectedID) { c in
                    captureRow(c).tag(c.id)
                }
            }
        }
    }

    private func captureRow(_ c: Capture) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack {
                Text(c.method)
                    .font(.system(size: 11, weight: .semibold, design: .monospaced))
                Text("· \(c.status)")
                    .font(.system(size: 11, design: .monospaced))
                    .foregroundStyle(statusColor(c.status))
                Spacer()
                Text(humanByteCount(c.bodyLength))
                    .font(.system(size: 10, design: .monospaced))
                    .foregroundStyle(.tertiary)
            }
            Text(URL(string: c.responseUrl)?.host ?? "?")
                .font(.system(size: 11, weight: .medium))
            Text(URL(string: c.responseUrl)?.path ?? c.responseUrl)
                .font(.system(size: 10, design: .monospaced))
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .truncationMode(.middle)
        }
        .padding(.vertical, 2)
    }

    private var inspector: some View {
        Group {
            if let id = selectedID, let c = captures.first(where: { $0.id == id }) {
                CaptureInspector(capture: c)
            } else {
                ContentUnavailableView(
                    "Pick a capture",
                    systemImage: "doc.plaintext",
                    description: Text("Select a row to see the decoded request/response body.")
                )
            }
        }
    }

    private func reload() {
        captures = []
        let url = entry.capturesFile
        guard FileManager.default.fileExists(atPath: url.path) else { return }
        do {
            let replayer = try BrowserReplayer(capturesFile: url)
            captures = replayer.captures
        } catch {
            loadError = String(describing: error)
        }
    }

    private func statusColor(_ status: Int) -> Color {
        switch status {
        case 200..<300: return .green
        case 300..<400: return .blue
        case 400..<500: return .orange
        case 500..<600: return .red
        default: return .secondary
        }
    }

    private func humanByteCount(_ n: Int) -> String {
        ByteCountFormatter.string(fromByteCount: Int64(n), countStyle: .file)
    }
}

private struct CaptureInspector: View {
    let capture: Capture

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                Grid(alignment: .leadingFirstTextBaseline, horizontalSpacing: 12, verticalSpacing: 4) {
                    GridRow {
                        Text("URL").foregroundStyle(.secondary)
                        Text(capture.responseUrl)
                            .font(.system(.body, design: .monospaced))
                            .textSelection(.enabled)
                    }
                    GridRow {
                        Text("Method").foregroundStyle(.secondary)
                        Text("\(capture.method) (\(capture.kind.rawValue))")
                            .font(.system(.body, design: .monospaced))
                    }
                    GridRow {
                        Text("Status").foregroundStyle(.secondary)
                        Text("\(capture.status)").font(.system(.body, design: .monospaced))
                    }
                    GridRow {
                        Text("Body bytes").foregroundStyle(.secondary)
                        Text("\(capture.bodyLength)").font(.system(.body, design: .monospaced))
                    }
                }
                if !capture.requestBody.isEmpty {
                    section("Request body", body: pretty(capture.requestBody))
                }
                section("Response body", body: pretty(capture.body))
            }
            .padding(14)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private func section(_ title: String, body: String) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(title).font(.headline)
            ScrollView(.horizontal) {
                Text(body)
                    .font(.system(size: 11, design: .monospaced))
                    .textSelection(.enabled)
                    .padding(8)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(
                        RoundedRectangle(cornerRadius: 4)
                            .fill(Color(nsColor: .textBackgroundColor))
                    )
            }
        }
    }

    private func pretty(_ raw: String) -> String {
        guard let data = raw.data(using: .utf8),
              let obj = try? JSONSerialization.jsonObject(with: data, options: [.fragmentsAllowed]),
              let pretty = try? JSONSerialization.data(withJSONObject: obj, options: [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes]),
              let s = String(data: pretty, encoding: .utf8) else {
            return raw
        }
        return s
    }
}

extension Notification.Name {
    /// Posted by `CaptureScene` after it writes captures back to the
    /// recipe's `fixtures/captures.jsonl` so the FixturesTab reloads.
    static let toolkitFixturesChanged = Notification.Name("ToolkitFixturesChanged")
}
