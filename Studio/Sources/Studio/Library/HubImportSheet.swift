import SwiftUI
import Forage

/// Modal sheet that browses recipes on `api.foragelang.com` (or wherever
/// `ToolkitPreferences.hubURL` points), lets the user pick one, fetches the
/// body, and writes it under `~/Library/Forage/Recipes/<slug>/`. Triggered
/// from the `Import` button in `LibrarySidebar` or the `Recipe → Import
/// from Hub…` menu command.
struct HubImportSheet: View {
    @Binding var isPresented: Bool

    @Environment(LibraryStore.self) private var library
    @Environment(ToolkitPreferences.self) private var preferences

    @State private var loadState: LoadState = .idle
    @State private var recipes: [HubRecipeMeta] = []
    @State private var search: String = ""
    @State private var importing: String? = nil       // slug being imported
    @State private var importError: String? = nil
    @State private var overwritePrompt: HubRecipeMeta? = nil

    enum LoadState: Equatable {
        case idle
        case loading
        case loaded
        case failed(String)
    }

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider()
            content
            Divider()
            footer
        }
        .frame(minWidth: 560, minHeight: 420)
        .task(id: preferences.hubURL) {
            await loadRecipes()
        }
        .alert("Overwrite local recipe?", isPresented: .constant(overwritePrompt != nil)) {
            Button("Cancel", role: .cancel) { overwritePrompt = nil }
            Button("Overwrite", role: .destructive) {
                if let m = overwritePrompt { Task { await importRecipe(m) } }
                overwritePrompt = nil
            }
        } message: {
            if let m = overwritePrompt {
                Text("A recipe at \(m.slug) already exists locally. Replacing will overwrite recipe.forage.")
            }
        }
        .alert("Import failed", isPresented: .constant(importError != nil)) {
            Button("OK", role: .cancel) { importError = nil }
        } message: {
            Text(importError ?? "")
        }
    }

    // MARK: - Sub-views

    private var header: some View {
        HStack(spacing: 8) {
            Image(systemName: "square.and.arrow.down")
                .foregroundStyle(.secondary)
            Text("Import from Hub")
                .font(.headline)
            Spacer()
            TextField("Search", text: $search, prompt: Text("Filter by slug or name"))
                .textFieldStyle(.roundedBorder)
                .frame(width: 200)
            Button("Reload") {
                Task { await loadRecipes() }
            }
            .disabled(loadState == .loading)
        }
        .padding(12)
    }

    @ViewBuilder
    private var content: some View {
        switch loadState {
        case .idle, .loading:
            VStack { Spacer(); ProgressView("Loading…"); Spacer() }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        case .failed(let msg):
            VStack(spacing: 10) {
                Spacer()
                Image(systemName: "exclamationmark.triangle")
                    .font(.title)
                    .foregroundStyle(.orange)
                Text("Couldn't load hub recipes")
                    .font(.headline)
                Text(msg)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
                    .padding(.horizontal, 24)
                Button("Try again") { Task { await loadRecipes() } }
                Spacer()
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        case .loaded:
            if filtered.isEmpty {
                VStack {
                    Spacer()
                    Text(search.isEmpty ? "No recipes published yet." : "No matches for “\(search)”.")
                        .foregroundStyle(.secondary)
                    Spacer()
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                List(filtered, id: \.slug) { meta in
                    rowView(for: meta)
                        .contentShape(Rectangle())
                }
                .listStyle(.inset)
            }
        }
    }

    private func rowView(for meta: HubRecipeMeta) -> some View {
        HStack(alignment: .top, spacing: 10) {
            VStack(alignment: .leading, spacing: 3) {
                HStack(spacing: 6) {
                    Text(meta.displayName).font(.system(size: 13, weight: .medium))
                    Text("(\(meta.slug))")
                        .font(.system(size: 11))
                        .foregroundStyle(.tertiary)
                }
                if let s = meta.summary, !s.isEmpty {
                    Text(s)
                        .font(.system(size: 11))
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                }
                if !meta.tags.isEmpty {
                    HStack(spacing: 4) {
                        ForEach(meta.tags.prefix(4), id: \.self) { tag in
                            Text(tag)
                                .font(.system(size: 10))
                                .padding(.horizontal, 5)
                                .padding(.vertical, 1)
                                .background(.quaternary, in: Capsule())
                        }
                    }
                }
            }
            Spacer()
            Button {
                handleImportTap(meta)
            } label: {
                if importing == meta.slug {
                    ProgressView().controlSize(.small)
                } else {
                    Text("Import")
                }
            }
            .disabled(importing != nil)
        }
        .padding(.vertical, 4)
    }

    private var footer: some View {
        HStack {
            Text("From \(preferences.hubURL)")
                .font(.system(size: 11))
                .foregroundStyle(.tertiary)
            Spacer()
            Button("Done") { isPresented = false }
                .keyboardShortcut(.cancelAction)
        }
        .padding(12)
    }

    // MARK: - Filtering

    private var filtered: [HubRecipeMeta] {
        guard !search.isEmpty else { return recipes }
        let q = search.lowercased()
        return recipes.filter {
            $0.slug.lowercased().contains(q) || $0.displayName.lowercased().contains(q)
        }
    }

    // MARK: - Actions

    private func handleImportTap(_ meta: HubRecipeMeta) {
        if library.hasLocalRecipe(slug: meta.slug) {
            overwritePrompt = meta
        } else {
            Task { await importRecipe(meta) }
        }
    }

    /// Build a `HubClient` from the current preferences. Reads the API key
    /// from Keychain — `nil` is fine for `GET` endpoints, which are public.
    private func makeClient() -> HubClient {
        let baseURL = URL(string: preferences.hubURL) ?? URL(string: "https://api.foragelang.com")!
        let token = try? Keychain.readAPIKey()
        return HubClient(baseURL: baseURL, token: token)
    }

    private func loadRecipes() async {
        loadState = .loading
        let client = makeClient()
        do {
            let (items, _) = try await client.list(limit: 100)
            recipes = items
            loadState = .loaded
        } catch {
            loadState = .failed(String(describing: error))
        }
    }

    private func importRecipe(_ meta: HubRecipeMeta) async {
        importing = meta.slug
        defer { importing = nil }
        let client = makeClient()
        do {
            let ref = try HubRecipeRef(parsing: meta.slug, version: nil)
            let recipe = try await client.get(ref)
            try library.importHubRecipe(slug: meta.slug, body: recipe.body)
            isPresented = false
        } catch {
            importError = String(describing: error)
        }
    }
}
