import Foundation
import SwiftUI

/// Enumerates recipes under `~/Library/Forage/Recipes/<slug>/`. Pure local
/// filesystem; hub-imports get stored under the same root with the slug as
/// the canonical key. New-recipe creates the directory + a minimal template.
@MainActor
@Observable
final class LibraryStore {
    private(set) var entries: [RecipeEntry] = []
    private(set) var rootDirectory: URL
    /// Set whenever `createNewRecipe` (or an importer) adds a new slug, so
    /// `ContentView` can auto-select it.
    private(set) var lastCreatedSlug: String?

    init() {
        let fm = FileManager.default
        let home = fm.homeDirectoryForCurrentUser
        let root = home.appending(path: "Library/Forage/Recipes", directoryHint: .isDirectory)
        try? fm.createDirectory(at: root, withIntermediateDirectories: true)
        self.rootDirectory = root
    }

    func refresh() {
        let fm = FileManager.default
        guard let contents = try? fm.contentsOfDirectory(
            at: rootDirectory,
            includingPropertiesForKeys: [.isDirectoryKey],
            options: [.skipsHiddenFiles]
        ) else {
            entries = []
            return
        }
        let dirs = contents.filter {
            (try? $0.resourceValues(forKeys: [.isDirectoryKey]).isDirectory) == true
        }
        entries = dirs
            .filter { fm.fileExists(atPath: $0.appending(path: "recipe.forage").path) }
            .map { RecipeEntry(slug: $0.lastPathComponent, directory: $0) }
            .sorted { $0.slug.localizedStandardCompare($1.slug) == .orderedAscending }
    }

    func entry(for slug: String) -> RecipeEntry? {
        entries.first(where: { $0.slug == slug })
    }

    /// Generate a fresh slug like `untitled-1`, `untitled-2`, … Creates the
    /// directory tree + minimal `recipe.forage` + an empty fixtures folder so
    /// the editor's tabs all have something to render against.
    func createNewRecipe() {
        let fm = FileManager.default
        let slug = nextUntitledSlug()
        let dir = rootDirectory.appending(path: slug, directoryHint: .isDirectory)
        do {
            try fm.createDirectory(at: dir, withIntermediateDirectories: true)
            try fm.createDirectory(at: dir.appending(path: "fixtures"), withIntermediateDirectories: true)
            try Self.minimalTemplate(forSlug: slug).write(
                to: dir.appending(path: "recipe.forage"),
                atomically: true,
                encoding: .utf8
            )
        } catch {
            NSLog("LibraryStore.createNewRecipe: \(error)")
            return
        }
        refresh()
        lastCreatedSlug = slug
    }

    /// Read the source of a recipe. Returns "" on read failure rather than
    /// surfacing the error — the editor's empty state is fine.
    func source(for slug: String) -> String {
        guard let entry = entry(for: slug),
              let data = try? Data(contentsOf: entry.recipeFile),
              let s = String(data: data, encoding: .utf8) else { return "" }
        return s
    }

    func writeSource(_ source: String, for slug: String) throws {
        guard let entry = entry(for: slug) else {
            throw LibraryError.unknownSlug(slug)
        }
        try source.write(to: entry.recipeFile, atomically: true, encoding: .utf8)
    }

    /// Compute next unused `untitled-N` slug.
    private func nextUntitledSlug() -> String {
        let existing = Set(entries.map(\.slug))
        var i = 1
        while existing.contains("untitled-\(i)") { i += 1 }
        return "untitled-\(i)"
    }

    private static func minimalTemplate(forSlug slug: String) -> String {
        """
        // \(slug).forage — author your recipe here.
        //
        // This template is a starting point for an HTTP-engine recipe.
        // For browser-engine recipes, add a `browser { … }` block and
        // declare `captures.match` rules instead of HTTP steps.

        recipe "\(slug)" {
            engine http

            type Item {
                id: String
                name: String
            }

            input apiUrl: String

            step list {
                method "GET"
                url    "{$input.apiUrl}"
            }

            for $item in $list[*] {
                emit Item {
                    id   ← $item.id | toString
                    name ← $item.name
                }
            }

            expect { records.where(typeName == "Item").count >= 1 }
        }
        """
    }
}

enum LibraryError: Error, CustomStringConvertible {
    case unknownSlug(String)

    var description: String {
        switch self {
        case .unknownSlug(let s): return "unknown recipe slug: \(s)"
        }
    }
}
