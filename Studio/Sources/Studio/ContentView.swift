import SwiftUI

/// Three-column NavigationSplitView: sidebar with recipe library on the left,
/// editor in the middle, and a contextual right rail (snapshot summary,
/// diagnostic, or empty state) is folded into the editor's tabbed inspector
/// instead — so the split is really two columns plus the editor's own tabs.
struct ContentView: View {
    @Environment(LibraryStore.self) private var library
    @State private var selectedSlug: String?
    @State private var hubImportPresented: Bool = false

    var body: some View {
        NavigationSplitView {
            LibrarySidebar(selectedSlug: $selectedSlug)
                .navigationSplitViewColumnWidth(min: 220, ideal: 280, max: 360)
        } detail: {
            if let slug = selectedSlug, let entry = library.entry(for: slug) {
                RecipeEditorView(slug: slug)
                    .id(entry.slug)
            } else {
                ContentUnavailableView(
                    "Pick a recipe",
                    systemImage: "doc.text.magnifyingglass",
                    description: Text("Choose a recipe from the sidebar, or create a new one to start authoring.")
                )
            }
        }
        .navigationTitle("Forage Studio")
        .onChange(of: library.lastCreatedSlug) { _, new in
            if let new { selectedSlug = new }
        }
        .onReceive(NotificationCenter.default.publisher(for: .studioImportFromHub)) { _ in
            hubImportPresented = true
        }
        .sheet(isPresented: $hubImportPresented) {
            HubImportSheet(isPresented: $hubImportPresented)
        }
    }
}
