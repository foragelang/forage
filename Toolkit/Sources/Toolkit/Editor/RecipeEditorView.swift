import SwiftUI
import Forage

/// Tabbed recipe editor scene. Top toolbar carries the run / capture
/// affordances; the content area is one of five tabs.
struct RecipeEditorView: View {
    let slug: String

    @Environment(LibraryStore.self) private var library
    @Environment(RunResultStore.self) private var runResults

    @State private var source: String = ""
    @State private var selectedTab: EditorTab = .source
    @State private var isDirty: Bool = false
    @State private var saveError: String?
    @State private var showCaptureSheet = false
    @State private var runError: String?
    @State private var isRunning = false
    @State private var runMode: RunController.Mode?

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider()
            TabView(selection: $selectedTab) {
                SourceTab(source: $source, onSave: save)
                    .tabItem { Label("Source", systemImage: "doc.text") }
                    .tag(EditorTab.source)
                FixturesTab(entry: entry, onCaptureFresh: { showCaptureSheet = true })
                    .tabItem { Label("Fixtures", systemImage: "tray.full") }
                    .tag(EditorTab.fixtures)
                SnapshotTab(slug: slug)
                    .tabItem { Label("Snapshot", systemImage: "doc.text.below.ecg") }
                    .tag(EditorTab.snapshot)
                DiagnosticTab(slug: slug)
                    .tabItem { Label("Diagnostic", systemImage: "stethoscope") }
                    .tag(EditorTab.diagnostic)
                PublishTab(slug: slug, source: source)
                    .tabItem { Label("Publish", systemImage: "paperplane") }
                    .tag(EditorTab.publish)
            }
            .padding(.horizontal, 4)
            .padding(.top, 4)
        }
        .onAppear { loadSource() }
        .onChange(of: source) { _, _ in isDirty = true }
        .onReceive(NotificationCenter.default.publisher(for: .toolkitRunLive)) { _ in
            Task { await run(mode: .live) }
        }
        .onReceive(NotificationCenter.default.publisher(for: .toolkitRunReplay)) { _ in
            Task { await run(mode: .replay) }
        }
        .onReceive(NotificationCenter.default.publisher(for: .toolkitCapture)) { _ in
            showCaptureSheet = true
        }
        .alert("Couldn't save", isPresented: .constant(saveError != nil)) {
            Button("OK") { saveError = nil }
        } message: { Text(saveError ?? "") }
        .alert("Run failed", isPresented: .constant(runError != nil)) {
            Button("OK") { runError = nil }
        } message: { Text(runError ?? "") }
        .sheet(isPresented: $showCaptureSheet) {
            CaptureScene(entry: entry, isPresented: $showCaptureSheet)
        }
    }

    private var entry: RecipeEntry {
        library.entry(for: slug) ?? RecipeEntry(
            slug: slug,
            directory: library.rootDirectory.appending(path: slug)
        )
    }

    private var header: some View {
        HStack(spacing: 10) {
            VStack(alignment: .leading, spacing: 1) {
                HStack(spacing: 4) {
                    Text(slug)
                        .font(.title3.weight(.semibold))
                    if isDirty {
                        Text("•")
                            .foregroundStyle(.orange)
                            .font(.title3)
                    }
                }
                Text(entry.directory.path)
                    .font(.system(size: 10, design: .monospaced))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.head)
            }

            Spacer()

            runProgressView

            Button {
                Task { await run(mode: .live) }
            } label: {
                Label("Run live", systemImage: "play.fill")
            }
            .disabled(isRunning)
            .help("Run against the live network using URLSession / WKWebView")

            Button {
                Task { await run(mode: .replay) }
            } label: {
                Label("Run replay", systemImage: "arrow.clockwise")
            }
            .disabled(isRunning || !entry.hasFixtures)
            .help(entry.hasFixtures
                  ? "Replay against fixtures/captures.jsonl"
                  : "No fixtures recorded yet — capture some first")

            Button {
                showCaptureSheet = true
            } label: {
                Label("Capture", systemImage: "dot.radiowaves.left.and.right")
            }
            .help("Open a WKWebView to capture requests")

            Button {
                save()
            } label: {
                Label("Save", systemImage: "tray.and.arrow.down")
            }
            .keyboardShortcut("s", modifiers: [.command])
            .help("Save the recipe source to disk")
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 8)
    }

    @ViewBuilder
    private var runProgressView: some View {
        if isRunning {
            HStack(spacing: 6) {
                ProgressView().controlSize(.small)
                Text(runMode == .live ? "Running…" : "Replaying…")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
    }

    private func loadSource() {
        source = library.source(for: slug)
        isDirty = false
    }

    private func save() {
        do {
            try library.writeSource(source, for: slug)
            isDirty = false
        } catch {
            saveError = String(describing: error)
        }
    }

    private func run(mode: RunController.Mode) async {
        if isDirty { save() }
        isRunning = true
        runMode = mode
        defer {
            isRunning = false
            runMode = nil
        }
        do {
            let controller = RunController()
            let result = try await controller.run(entry: entry, mode: mode, source: source)
            runResults.setLatest(result, for: slug)
            selectedTab = .snapshot
        } catch {
            runError = String(describing: error)
        }
    }
}

enum EditorTab: String, Hashable {
    case source, fixtures, snapshot, diagnostic, publish
}
