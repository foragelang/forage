import Testing
import Foundation
@testable import Forage

private let bundledRecipesURL: URL = URL(fileURLWithPath: #filePath)
    .deletingLastPathComponent()  // Tests/ForageTests
    .deletingLastPathComponent()  // Tests
    .deletingLastPathComponent()  // package root
    .appendingPathComponent("recipes", isDirectory: true)

@MainActor
@Test
func registryLoadsBundledRecipes() throws {
    let registry = RecipeRegistry(root: bundledRecipesURL)
    try registry.loadAll()
    let keys = Set(registry.recipes.keys)
    #expect(keys.count >= 3)
    #expect(keys.contains("sweed"))
    #expect(keys.contains("leafbridge"))
    #expect(keys.contains("jane"))
}

@MainActor
@Test
func registryRecipeByNameReturnsParsedRecipe() throws {
    let registry = RecipeRegistry(root: bundledRecipesURL)
    try registry.loadAll()
    let jane = registry.recipe(forName: "jane")
    #expect(jane != nil)
    #expect(jane?.engineKind == .browser)
    let sweed = registry.recipe(forName: "sweed")
    #expect(sweed?.engineKind == .http)
    #expect(registry.recipe(forName: "doesNotExist") == nil)
}

@MainActor
@Test
func registrySkipsInvalidRecipeAndLogs() throws {
    let root = try makeTempRoot()
    defer { try? FileManager.default.removeItem(at: root) }

    try writeRecipe(into: root, platform: "good", contents: """
        recipe "good" {
            engine http
            type Item { id: String }
            emit Item { id ← "x" }
        }
        """)
    try writeRecipe(into: root, platform: "bad", contents: """
        recipe "bad" {
            engine http
            type Item { id: String }
            emit NotAType { id ← "x" }
        }
        """)

    var logs: [String] = []
    let registry = RecipeRegistry(root: root, logger: { logs.append($0) })
    try registry.loadAll()

    #expect(registry.recipes.keys.contains("good"))
    #expect(!registry.recipes.keys.contains("bad"))
    #expect(logs.contains(where: { $0.contains("failed validation") && $0.contains("bad") }))
}

@MainActor
@Test
func registryWithEmptySubdirsLoadsNothing() throws {
    let root = try makeTempRoot()
    defer { try? FileManager.default.removeItem(at: root) }
    try FileManager.default.createDirectory(at: root.appendingPathComponent("empty1"), withIntermediateDirectories: false)
    try FileManager.default.createDirectory(at: root.appendingPathComponent("empty2"), withIntermediateDirectories: false)

    let registry = RecipeRegistry(root: root)
    try registry.loadAll()
    #expect(registry.recipes.isEmpty)
}

@MainActor
@Test
func registryMissingRootThrows() throws {
    let bogus = URL(fileURLWithPath: "/tmp/forage-recipe-registry-does-not-exist-\(UUID().uuidString)")
    let registry = RecipeRegistry(root: bogus)
    #expect(throws: RecipeRegistryError.self) {
        try registry.loadAll()
    }
}

@MainActor
@Test
func registrySkipsUnparseableRecipeAndLogs() throws {
    let root = try makeTempRoot()
    defer { try? FileManager.default.removeItem(at: root) }
    try writeRecipe(into: root, platform: "garbage", contents: "this is not a recipe at all")

    var logs: [String] = []
    let registry = RecipeRegistry(root: root, logger: { logs.append($0) })
    try registry.loadAll()
    #expect(registry.recipes.isEmpty)
    #expect(logs.contains(where: { $0.contains("Failed to parse") }))
}

@MainActor
@Test
func registryHotReloadsModifiedRecipe() async throws {
    if ProcessInfo.processInfo.environment["FORAGE_SKIP_HOT_RELOAD_TEST"] != nil {
        return
    }
    let root = try makeTempRoot()
    defer { try? FileManager.default.removeItem(at: root) }

    let initial = """
        recipe "ham" {
            engine http
            type Item { id: String }
            emit Item { id ← "v1" }
        }
        """
    try writeRecipe(into: root, platform: "ham", contents: initial)

    var logs: [String] = []
    let registry = RecipeRegistry(root: root, watch: true, logger: { logs.append($0) })
    try registry.loadAll()
    #expect(registry.recipes["ham"] != nil)

    // Rewrite with a marker we can spot in the parsed recipe.
    let updated = """
        recipe "ham" {
            engine http
            type Item { id: String; nickname: String? }
            emit Item { id ← "v2" }
        }
        """
    // Sleep one second to make sure the mtime resolution captures the change
    // (HFS+/APFS mtimes are typically 1s-resolution). Then write.
    try await Task.sleep(nanoseconds: 1_100_000_000)
    try writeRecipe(into: root, platform: "ham", contents: updated)

    // Poll-and-wait for the registry to notice the change (up to ~3s).
    var observed = false
    for _ in 0..<30 {
        try await Task.sleep(nanoseconds: 100_000_000)
        if let r = registry.recipes["ham"], r.types.first?.fields.contains(where: { $0.name == "nickname" }) == true {
            observed = true
            break
        }
    }
    #expect(observed, "registry did not observe the on-disk change within 3s; logs: \(logs)")
    #expect(logs.contains(where: { $0.contains("Hot-reloaded recipe 'ham'") }))
}

@MainActor
@Test
func registryHotReloadKeepsPreviousOnValidationFailure() async throws {
    if ProcessInfo.processInfo.environment["FORAGE_SKIP_HOT_RELOAD_TEST"] != nil {
        return
    }
    let root = try makeTempRoot()
    defer { try? FileManager.default.removeItem(at: root) }

    try writeRecipe(into: root, platform: "ham", contents: """
        recipe "ham" {
            engine http
            type Item { id: String }
            emit Item { id ← "v1" }
        }
        """)

    var logs: [String] = []
    let registry = RecipeRegistry(root: root, watch: true, logger: { logs.append($0) })
    try registry.loadAll()
    let originalRecipe = try #require(registry.recipes["ham"])

    try await Task.sleep(nanoseconds: 1_100_000_000)
    // Write a recipe that parses but references an unknown type → validation error.
    try writeRecipe(into: root, platform: "ham", contents: """
        recipe "ham" {
            engine http
            type Item { id: String }
            emit NotAType { id ← "v2" }
        }
        """)

    // Wait long enough for the watcher to tick at least a few times.
    for _ in 0..<30 {
        try await Task.sleep(nanoseconds: 100_000_000)
        if logs.contains(where: { $0.contains("Hot-reload of") && $0.contains("failed") }) { break }
    }
    let still = try #require(registry.recipes["ham"])
    #expect(still == originalRecipe, "expected old recipe to remain after validation failure")
    #expect(logs.contains(where: { $0.contains("Hot-reload of") && $0.contains("failed") }))
}

// MARK: - Helpers

private func makeTempRoot() throws -> URL {
    let url = URL(fileURLWithPath: NSTemporaryDirectory())
        .appendingPathComponent("forage-recipe-registry-\(UUID().uuidString)", isDirectory: true)
    try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
    return url
}

private func writeRecipe(into root: URL, platform: String, contents: String) throws {
    let dir = root.appendingPathComponent(platform, isDirectory: true)
    try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    try contents.write(
        to: dir.appendingPathComponent("recipe.forage"),
        atomically: true,
        encoding: .utf8
    )
}
