import Foundation

/// Loads `.forage` recipes from a directory laid out as
/// `<root>/<platform>/recipe.forage` and exposes them keyed by `recipe.name`.
///
/// Pointed at `Bundle.main.resourceURL/recipes` for release builds and at a
/// dev-checkout `recipes/` directory (via env var) for hot-reload builds.
/// Either way the registry is the same type; the consumer just hands it a
/// different `root`.
///
/// On parse / validation failure, the offending file is logged and skipped;
/// `loadAll()` only throws on unrecoverable IO errors (missing root, etc).
/// During hot-reload a failed reload keeps the previously-loaded version of
/// the recipe in place.
@MainActor
@Observable
public final class RecipeRegistry {
    public private(set) var recipes: [String: Recipe] = [:]

    private let root: URL
    private let watch: Bool
    private let logger: ((String) -> Void)?
    private var watcher: RecipeWatcher?
    private var recipePaths: [String: URL] = [:]

    public init(
        root: URL,
        watch: Bool = false,
        logger: ((String) -> Void)? = nil
    ) {
        self.root = root
        self.watch = watch
        self.logger = logger
    }

    public func loadAll() throws {
        let fm = FileManager.default
        var isDir: ObjCBool = false
        guard fm.fileExists(atPath: root.path, isDirectory: &isDir), isDir.boolValue else {
            throw RecipeRegistryError.rootMissing(root)
        }

        let entries = try fm.contentsOfDirectory(
            at: root,
            includingPropertiesForKeys: [.isDirectoryKey]
        )

        var loaded: [String: Recipe] = [:]
        var loadedPaths: [String: URL] = [:]
        for entry in entries {
            let entryIsDir = (try? entry.resourceValues(forKeys: [.isDirectoryKey]).isDirectory) ?? false
            guard entryIsDir else { continue }
            let file = entry.appendingPathComponent("recipe.forage")
            guard fm.fileExists(atPath: file.path) else { continue }
            if let recipe = loadFile(file) {
                if let previousPath = loadedPaths[recipe.name] {
                    logger?("Recipe '\(recipe.name)' loaded from \(file.path) overrides earlier recipe at \(previousPath.path)")
                }
                loaded[recipe.name] = recipe
                loadedPaths[recipe.name] = file
            }
        }
        recipes = loaded
        recipePaths = loadedPaths

        if watch && watcher == nil {
            let w = RecipeWatcher(root: root) { [weak self] url in
                self?.reload(url)
            }
            watcher = w
            w.start()
        }
    }

    public func recipe(forName name: String) -> Recipe? {
        recipes[name]
    }

    // MARK: - Internal

    private func loadFile(_ file: URL) -> Recipe? {
        let source: String
        do {
            source = try String(contentsOf: file, encoding: .utf8)
        } catch {
            logger?("Failed to read \(file.lastPathComponent): \(error)")
            return nil
        }
        let recipe: Recipe
        do {
            recipe = try Parser.parse(source: source)
        } catch {
            logger?("Failed to parse \(file.lastPathComponent): \(error)")
            return nil
        }
        let issues = Validator.validate(recipe)
        if issues.hasErrors {
            let first = issues.errors.first.map { "\($0.message) [\($0.location)]" } ?? "unknown"
            logger?("Recipe \(recipe.name) failed validation: \(first)")
            return nil
        }
        return recipe
    }

    private func reload(_ file: URL) {
        let fm = FileManager.default
        guard fm.fileExists(atPath: file.path) else {
            rescan()
            return
        }
        guard let recipe = loadFile(file) else {
            logger?("Hot-reload of \(file.lastPathComponent) failed; keeping previous version")
            return
        }
        if let previousPath = recipePaths[recipe.name], previousPath != file {
            logger?("Recipe '\(recipe.name)' loaded from \(file.path) overrides earlier recipe at \(previousPath.path)")
        }
        recipes[recipe.name] = recipe
        recipePaths[recipe.name] = file
        logger?("Hot-reloaded recipe '\(recipe.name)'")
    }

    private func rescan() {
        let fm = FileManager.default
        guard let entries = try? fm.contentsOfDirectory(
            at: root,
            includingPropertiesForKeys: [.isDirectoryKey]
        ) else {
            return
        }
        var seen: [String: URL] = [:]
        for entry in entries {
            let isDir = (try? entry.resourceValues(forKeys: [.isDirectoryKey]).isDirectory) ?? false
            guard isDir else { continue }
            let file = entry.appendingPathComponent("recipe.forage")
            guard fm.fileExists(atPath: file.path) else { continue }
            if let recipe = loadFile(file) {
                if let previousPath = seen[recipe.name] {
                    logger?("Recipe '\(recipe.name)' loaded from \(file.path) overrides earlier recipe at \(previousPath.path)")
                }
                recipes[recipe.name] = recipe
                seen[recipe.name] = file
            }
        }
        for name in recipes.keys where seen[name] == nil {
            recipes.removeValue(forKey: name)
            recipePaths.removeValue(forKey: name)
            logger?("Recipe '\(name)' removed (file no longer present)")
        }
        for (name, file) in seen {
            recipePaths[name] = file
        }
    }
}

public enum RecipeRegistryError: Error, CustomStringConvertible {
    case rootMissing(URL)

    public var description: String {
        switch self {
        case .rootMissing(let url):
            return "RecipeRegistry: root directory missing or not a directory: \(url.path)"
        }
    }
}
