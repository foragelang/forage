import SwiftUI
import Forage

/// Modal sheet hosting a live `WKWebView` + capture feed. Keep/skip per
/// row; on Save, append (or replace) the kept rows to the recipe's
/// `fixtures/captures.jsonl`.
struct CaptureScene: View {
    let entry: RecipeEntry
    @Binding var isPresented: Bool

    @State private var session = CaptureSession()
    @State private var urlString: String = "https://"
    @State private var saveMode: SaveMode = .append
    @State private var saveError: String?

    enum SaveMode: String, CaseIterable, Identifiable {
        case append, replace
        var id: String { rawValue }
        var label: String {
            switch self {
            case .append: return "Append to existing fixtures"
            case .replace: return "Replace existing fixtures"
            }
        }
    }

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider()
            HSplitView {
                WebViewMount(webView: session.webView)
                    .frame(minWidth: 520, minHeight: 480)
                captureSidebar
                    .frame(minWidth: 320, idealWidth: 380, maxWidth: 500)
            }
            Divider()
            footer
        }
        .frame(minWidth: 1000, minHeight: 600)
        .alert("Couldn't save fixtures", isPresented: .constant(saveError != nil)) {
            Button("OK") { saveError = nil }
        } message: { Text(saveError ?? "") }
    }

    private var header: some View {
        HStack(spacing: 8) {
            Text("Capture for \(entry.slug)")
                .font(.headline)
            Spacer()
            TextField("URL", text: $urlString, onCommit: load)
                .textFieldStyle(.roundedBorder)
                .frame(minWidth: 320, idealWidth: 500)
                .font(.system(.body, design: .monospaced))
            Button("Go", action: load)
                .keyboardShortcut(.return, modifiers: [])
                .buttonStyle(.borderedProminent)
            Button {
                session.clearCaptures()
            } label: {
                Label("Clear", systemImage: "trash")
            }
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 8)
    }

    private var captureSidebar: some View {
        VStack(spacing: 0) {
            HStack(spacing: 6) {
                Text("\(session.captures.count) captures")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                Spacer()
                Text("\(keptCount) kept")
                    .font(.caption)
                    .foregroundStyle(.green)
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 6)
            Divider()
            if session.captures.isEmpty {
                ContentUnavailableView(
                    "Nothing captured yet",
                    systemImage: "tray",
                    description: Text("Load a URL above. Every fetch / XHR on the page is captured.")
                )
            } else {
                List($session.captures) { $row in
                    captureRow($row)
                }
                .listStyle(.inset)
            }
        }
    }

    private func captureRow(_ row: Binding<LiveCapture>) -> some View {
        let c = row.wrappedValue.capture
        return HStack(alignment: .top, spacing: 8) {
            Toggle("", isOn: row.keep)
                .labelsHidden()
                .toggleStyle(.checkbox)
                .help("Keep this capture when saving to fixtures")
            VStack(alignment: .leading, spacing: 2) {
                HStack {
                    Text(c.method)
                        .font(.system(size: 11, weight: .semibold, design: .monospaced))
                    Text("\(c.status)")
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
        }
        .padding(.vertical, 2)
    }

    private var footer: some View {
        HStack(spacing: 10) {
            Picker("Save mode", selection: $saveMode) {
                ForEach(SaveMode.allCases) { mode in
                    Text(mode.label).tag(mode)
                }
            }
            .pickerStyle(.menu)
            .frame(maxWidth: 280)

            Spacer()

            Button("Cancel") { isPresented = false }
                .keyboardShortcut(.escape, modifiers: [])
            Button("Save to \(entry.slug)'s fixtures") {
                save()
            }
            .buttonStyle(.borderedProminent)
            .disabled(keptCount == 0)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
    }

    private var keptCount: Int { session.captures.filter(\.keep).count }

    private func load() {
        let trimmed = urlString.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        session.load(urlString: trimmed)
    }

    private func save() {
        do {
            try session.saveKeptCaptures(to: entry.capturesFile, append: saveMode == .append)
            NotificationCenter.default.post(name: .toolkitFixturesChanged, object: nil)
            isPresented = false
        } catch {
            saveError = String(describing: error)
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
