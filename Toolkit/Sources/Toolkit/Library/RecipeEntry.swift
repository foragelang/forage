import Foundation

/// One slug under `~/Library/Forage/Recipes/<slug>/`. Identifies the entry by
/// slug; `directory` resolves to the on-disk folder.
struct RecipeEntry: Identifiable, Hashable, Sendable {
    let slug: String
    let directory: URL

    var id: String { slug }
    var recipeFile: URL { directory.appending(path: "recipe.forage") }
    var fixturesDir: URL { directory.appending(path: "fixtures") }
    var capturesFile: URL { fixturesDir.appending(path: "captures.jsonl") }
    var snapshotsDir: URL { directory.appending(path: "snapshots") }

    var hasFixtures: Bool {
        FileManager.default.fileExists(atPath: capturesFile.path)
    }
}
