import Foundation

/// Resolves `import hub://...` directives by fetching each imported recipe
/// from the hub, parsing it, and unioning its declarations into the
/// importing recipe. The result is a flat `Recipe` whose
/// `types`/`enums`/`inputs` are the union across the root and all
/// imports, and whose `body` is the original root's body (imports
/// contribute declarations, not statements — body re-execution would
/// double-fire HTTP traffic).
///
/// Conflict policy (v1, simple):
/// - Same type / enum name in two imports → error.
/// - Same input name across imports / root → error.
/// - The root recipe always wins over an import (so a recipe can override
///   an imported type by declaring it locally).
///
/// Cycle detection: each fetch records the (slug, version) it's resolving;
/// re-entering the same pair throws.
public actor RecipeImporter {
    public enum Error: Swift.Error, Sendable, CustomStringConvertible {
        case cycle([String])
        case typeCollision(String, between: String, and: String)
        case enumCollision(String, between: String, and: String)
        case inputCollision(String, between: String, and: String)
        case hub(HubClient.Error, slug: String)
        case parse(String, slug: String)

        public var description: String {
            switch self {
            case .cycle(let chain):
                return "import: cycle detected: \(chain.joined(separator: " → "))"
            case .typeCollision(let n, let a, let b):
                return "import: type '\(n)' is declared in both '\(a)' and '\(b)'"
            case .enumCollision(let n, let a, let b):
                return "import: enum '\(n)' is declared in both '\(a)' and '\(b)'"
            case .inputCollision(let n, let a, let b):
                return "import: input '\(n)' is declared in both '\(a)' and '\(b)'"
            case .hub(let err, let slug):
                return "import: hub fetch failed for '\(slug)': \(err)"
            case .parse(let msg, let slug):
                return "import: failed to parse '\(slug)': \(msg)"
            }
        }
    }

    private let client: HubClient
    private let cacheRoot: URL?

    /// `cacheRoot` is the directory where fetched recipe bodies are cached
    /// (one file per slug+version). Pass `nil` to skip on-disk caching.
    public init(client: HubClient, cacheRoot: URL? = nil) {
        self.client = client
        self.cacheRoot = cacheRoot
    }

    /// Default cache dir: `~/Library/Forage/Cache/hub/`. Convenience for
    /// CLI/Toolkit callers — pass it as `cacheRoot` to `init`.
    public static func defaultCacheRoot() -> URL? {
        guard let home = FileManager.default.urls(
            for: .libraryDirectory, in: .userDomainMask
        ).first else {
            return nil
        }
        return home
            .appendingPathComponent("Forage", isDirectory: true)
            .appendingPathComponent("Cache", isDirectory: true)
            .appendingPathComponent("hub", isDirectory: true)
    }

    /// Resolve and flatten `recipe`'s imports. Returns a recipe with no
    /// `imports` field set and merged declarations.
    public func flatten(_ recipe: Recipe) async throws -> Recipe {
        var inProgress = Set<String>()
        var visited = Set<String>()
        return try await flattenRecursive(
            recipe,
            label: "(root: \(recipe.name))",
            inProgress: &inProgress,
            visited: &visited
        )
    }

    // MARK: - Recursive resolver

    private func flattenRecursive(
        _ recipe: Recipe,
        label: String,
        inProgress: inout Set<String>,
        visited: inout Set<String>
    ) async throws -> Recipe {
        var mergedTypes = recipe.types
        var mergedEnums = recipe.enums
        var mergedInputs = recipe.inputs
        var typeOwner: [String: String] = Dictionary(
            uniqueKeysWithValues: recipe.types.map { ($0.name, label) }
        )
        var enumOwner: [String: String] = Dictionary(
            uniqueKeysWithValues: recipe.enums.map { ($0.name, label) }
        )
        var inputOwner: [String: String] = Dictionary(
            uniqueKeysWithValues: recipe.inputs.map { ($0.name, label) }
        )

        for ref in recipe.imports {
            let key = cacheKey(for: ref)
            if inProgress.contains(key) {
                throw Error.cycle(inProgress.sorted() + [key])
            }
            if visited.contains(key) {
                // Already merged in this run; skip duplicate imports of the
                // same slug+version.
                continue
            }
            inProgress.insert(key)
            defer { inProgress.remove(key) }

            let body = try await fetchBody(for: ref)
            let imported: Recipe
            do {
                imported = try Parser.parse(source: body)
            } catch {
                throw Error.parse("\(error)", slug: ref.slug)
            }

            // Recurse into the imported recipe's own imports first.
            let resolved = try await flattenRecursive(
                imported,
                label: "hub://\(ref.slug)",
                inProgress: &inProgress,
                visited: &visited
            )
            visited.insert(key)

            // Union declarations. Root-or-earlier-import wins; a brand-new
            // name is added.
            for t in resolved.types {
                if let existing = typeOwner[t.name] {
                    if existing == label { continue }    // root override beats import
                    throw Error.typeCollision(t.name, between: existing, and: "hub://\(ref.slug)")
                }
                mergedTypes.append(t)
                typeOwner[t.name] = "hub://\(ref.slug)"
            }
            for e in resolved.enums {
                if let existing = enumOwner[e.name] {
                    if existing == label { continue }
                    throw Error.enumCollision(e.name, between: existing, and: "hub://\(ref.slug)")
                }
                mergedEnums.append(e)
                enumOwner[e.name] = "hub://\(ref.slug)"
            }
            for inp in resolved.inputs {
                if let existing = inputOwner[inp.name] {
                    if existing == label { continue }
                    throw Error.inputCollision(inp.name, between: existing, and: "hub://\(ref.slug)")
                }
                mergedInputs.append(inp)
                inputOwner[inp.name] = "hub://\(ref.slug)"
            }
        }

        return Recipe(
            name: recipe.name,
            engineKind: recipe.engineKind,
            types: mergedTypes,
            enums: mergedEnums,
            inputs: mergedInputs,
            auth: recipe.auth,
            body: recipe.body,
            browser: recipe.browser,
            expectations: recipe.expectations,
            imports: []
        )
    }

    // MARK: - Cache + fetch

    private func cacheKey(for ref: HubRecipeRef) -> String {
        let v = ref.version.map(String.init) ?? "latest"
        return "\(ref.slug)@\(v)"
    }

    private func cachePath(for ref: HubRecipeRef) -> URL? {
        guard let root = cacheRoot else { return nil }
        let safeSlug = ref.slug.replacingOccurrences(of: "/", with: "__")
        let v = ref.version.map(String.init) ?? "latest"
        return root
            .appendingPathComponent(safeSlug, isDirectory: true)
            .appendingPathComponent(v, isDirectory: true)
            .appendingPathComponent("recipe.forage")
    }

    private func fetchBody(for ref: HubRecipeRef) async throws -> String {
        // Pinned versions can be served from on-disk cache; `latest` always
        // hits the network so the user gets the newest publish.
        if ref.version != nil, let cached = readCache(for: ref) {
            return cached
        }
        let recipe: HubRecipe
        do {
            recipe = try await client.get(ref)
        } catch let err as HubClient.Error {
            throw Error.hub(err, slug: ref.slug)
        }
        writeCache(for: ref, body: recipe.body)
        return recipe.body
    }

    private func readCache(for ref: HubRecipeRef) -> String? {
        guard let path = cachePath(for: ref),
              FileManager.default.fileExists(atPath: path.path)
        else { return nil }
        return try? String(contentsOf: path, encoding: .utf8)
    }

    private func writeCache(for ref: HubRecipeRef, body: String) {
        guard let path = cachePath(for: ref) else { return }
        let dir = path.deletingLastPathComponent()
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        try? body.write(to: path, atomically: true, encoding: .utf8)
    }
}
