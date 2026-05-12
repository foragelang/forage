import Testing
import Foundation
@testable import Studio

/// `LibraryStore` enumerates recipes under a directory we don't control —
/// `~/Library/Forage/Recipes/`. To keep these tests hermetic, we use the
/// store's public API to create a slug and then verify it appears via
/// `refresh()`. We can't redirect the root without exposing it, so the
/// test treats the user's real directory as test fixtures and cleans up
/// after itself.

@MainActor
@Test
func libraryStoreRoundTripsNewRecipe() throws {
    let store = LibraryStore()
    let before = Set(store.entries.map(\.slug))
    store.createNewRecipe()
    let createdSlug = try #require(store.lastCreatedSlug)
    let after = Set(store.entries.map(\.slug))

    defer {
        // Clean up — remove the slug we just created so we don't leave
        // stray test artifacts in the user's real library.
        let dir = store.rootDirectory.appending(path: createdSlug)
        try? FileManager.default.removeItem(at: dir)
    }

    #expect(after.contains(createdSlug))
    #expect(!before.contains(createdSlug))

    let entry = try #require(store.entry(for: createdSlug))
    #expect(FileManager.default.fileExists(atPath: entry.recipeFile.path))
    let body = try String(contentsOf: entry.recipeFile, encoding: .utf8)
    #expect(body.contains("recipe \"\(createdSlug)\""))
}

@MainActor
@Test
func libraryStoreWriteSourceRoundTrips() throws {
    let store = LibraryStore()
    store.createNewRecipe()
    let slug = try #require(store.lastCreatedSlug)
    defer {
        let dir = store.rootDirectory.appending(path: slug)
        try? FileManager.default.removeItem(at: dir)
    }

    let newSource = """
        recipe "\(slug)" {
            engine http
        }
        """
    try store.writeSource(newSource, for: slug)
    #expect(store.source(for: slug) == newSource)
}
