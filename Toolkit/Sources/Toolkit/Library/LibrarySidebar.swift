import SwiftUI

struct LibrarySidebar: View {
    @Environment(LibraryStore.self) private var library
    @Binding var selectedSlug: String?

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider()
            list
            Divider()
            footer
        }
        .frame(maxHeight: .infinity, alignment: .top)
    }

    private var header: some View {
        HStack {
            Text("Recipes")
                .font(.headline)
            Spacer()
            Button {
                library.refresh()
            } label: {
                Image(systemName: "arrow.clockwise")
            }
            .buttonStyle(.borderless)
            .help("Reload from disk")
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
    }

    private var list: some View {
        Group {
            if library.entries.isEmpty {
                VStack(spacing: 8) {
                    Spacer()
                    Text("No local recipes yet.")
                        .foregroundStyle(.secondary)
                    Text("Click + to create one, or import from the hub.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .multilineTextAlignment(.center)
                    Spacer()
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .padding()
            } else {
                List(library.entries, selection: $selectedSlug) { entry in
                    HStack(spacing: 6) {
                        Image(systemName: "doc.text")
                            .foregroundStyle(.secondary)
                        VStack(alignment: .leading, spacing: 1) {
                            Text(entry.slug)
                                .font(.system(size: 13, weight: .medium))
                            if entry.hasFixtures {
                                Text("fixtures present")
                                    .font(.system(size: 10))
                                    .foregroundStyle(.secondary)
                            } else {
                                Text("no fixtures")
                                    .font(.system(size: 10))
                                    .foregroundStyle(.tertiary)
                            }
                        }
                        Spacer()
                    }
                    .tag(entry.slug)
                }
                .listStyle(.sidebar)
            }
        }
    }

    private var footer: some View {
        HStack(spacing: 8) {
            Button {
                library.createNewRecipe()
            } label: {
                Label("New", systemImage: "plus")
            }
            .help("Create a new local recipe")

            Button {
                NotificationCenter.default.post(name: .toolkitImportFromHub, object: nil)
            } label: {
                Label("Import", systemImage: "square.and.arrow.down")
            }
            .help("Import a recipe from the hub (coming in M4)")
            .disabled(true)

            Spacer()
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 6)
        .buttonStyle(.borderless)
    }
}

extension Notification.Name {
    static let toolkitImportFromHub = Notification.Name("ToolkitImportFromHub")
}
