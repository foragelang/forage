import ArgumentParser
import Foundation
import Forage

struct PublishCommand: AsyncParsableCommand {
    static let configuration = CommandConfiguration(
        commandName: "publish",
        abstract: "Push a recipe to the Forage hub. Dry-run by default; pass --publish to POST."
    )

    @Argument(help: "Recipe directory (or `.forage` file). Optional sibling files: fixtures/captures.jsonl, expected.snapshot.json, recipe.json.")
    var recipeDir: String

    // Metadata: read from recipe.json / recipe.yml if present, otherwise the
    // user supplies via flags. CLI flags always win when both are present.
    @Option(help: "Recipe name to publish under. Defaults to recipe.name if no recipe.json.")
    var name: String?

    @Option(help: "Namespace to publish under. Defaults to 'forage' (the official namespace).")
    var namespace: String?

    @Option(help: "Human-readable display name.")
    var displayName: String?

    @Option(help: "One-line summary.")
    var summary: String?

    @Option(help: "Author handle (defaults to omitted).")
    var author: String?

    @Option(parsing: .singleValue, help: "Tag (repeatable). Comma-split if a single arg contains commas.")
    var tags: [String] = []

    @Flag(help: "Actually POST to the hub. Default behaviour is dry-run: print the payload, exit.")
    var publish = false

    @Flag(help: "Alias of --publish (mirrors hub-api expectations).")
    var noDryRun = false

    func run() async throws {
        let recipeFile = try RunCommand.resolveRecipePath(recipeDir)
        let recipeURL = URL(fileURLWithPath: recipeFile)
        let dirURL = recipeURL.deletingLastPathComponent()

        let src: String
        do {
            src = try String(contentsOf: recipeURL, encoding: .utf8)
        } catch {
            FileHandle.standardError.write("read failed: \(error)\n".data(using: .utf8)!)
            throw ExitCode.failure
        }

        let recipe: Recipe
        do {
            recipe = try Parser.parse(source: src)
        } catch {
            FileHandle.standardError.write("parse failed: \(error)\n".data(using: .utf8)!)
            throw ExitCode.failure
        }

        // We validate the *parsed* recipe — imports aren't flattened for
        // publish since the body that lands in the registry is exactly the
        // text the user shipped (consumers of the published recipe will
        // resolve imports themselves at run time).
        // To still catch silly errors, we run the validator only when there
        // are no unresolved imports.
        if recipe.imports.isEmpty {
            let issues = Validator.validate(recipe)
            if issues.hasErrors {
                FileHandle.standardError.write("validation failed:\n".data(using: .utf8)!)
                for e in issues.errors {
                    FileHandle.standardError.write(" - \(e.message) [\(e.location)]\n".data(using: .utf8)!)
                }
                throw ExitCode.failure
            }
        }

        // Optional sibling files.
        let fixturesURL = dirURL.appendingPathComponent("fixtures/captures.jsonl")
        let snapshotURL = dirURL.appendingPathComponent("expected.snapshot.json")
        let metaJSONURL = dirURL.appendingPathComponent("recipe.json")

        let fixturesText: String? = readIfExists(fixturesURL)
        let snapshotText: String? = readIfExists(snapshotURL)
        let fileMeta = readMetadataFile(metaJSONURL)

        // Merge metadata: CLI flags override file metadata; file metadata
        // overrides recipe-derived defaults.
        let resolvedName = name ?? fileMeta?.name ?? defaultName(for: recipe)
        let resolvedNamespace = namespace ?? fileMeta?.namespace ?? "forage"
        let resolvedDisplayName = displayName ?? fileMeta?.displayName ?? recipe.name
        let resolvedSummary = summary ?? fileMeta?.summary
        let resolvedAuthor = author ?? fileMeta?.author
        let resolvedTags = tags.isEmpty
            ? (fileMeta?.tags ?? [])
            : tags.flatMap { $0.split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty } }

        let resolvedSlug = "\(resolvedNamespace)/\(resolvedName)"
        guard validateSlug(resolvedSlug) else {
            FileHandle.standardError.write(
                "invalid slug '\(resolvedSlug)' — namespace and name must each match ^[a-z0-9][a-z0-9-]{1,63}$\n"
                    .data(using: .utf8)!
            )
            throw ExitCode.failure
        }

        let payload = HubPublishPayload(
            slug: resolvedSlug,
            author: resolvedAuthor,
            displayName: resolvedDisplayName,
            summary: resolvedSummary,
            tags: resolvedTags,
            body: src,
            fixtures: fixturesText,
            snapshot: snapshotText
        )

        let shouldPublish = publish || noDryRun

        if !shouldPublish {
            try printPayload(payload)
            print("# dry-run — pass --publish to POST")
            return
        }

        // Token sourcing order (post-M11):
        // 1. `FORAGE_HUB_TOKEN` env var — admin / legacy / CI override.
        // 2. Auth-store JWT from `forage auth login` (per-host).
        let baseURL = RunCommand.hubURL()
        let envToken = ProcessInfo.processInfo.environment["FORAGE_HUB_TOKEN"]
        let storedToken = AuthStore.read(host: baseURL)?.accessToken
        let token: String
        if let env = envToken, !env.isEmpty {
            token = env
        } else if let stored = storedToken {
            token = stored
        } else {
            FileHandle.standardError.write(
                "--publish requires a hub token: set FORAGE_HUB_TOKEN or run `forage auth login`\n".data(using: .utf8)!
            )
            throw ExitCode.failure
        }

        let client = HubClient(baseURL: baseURL, token: token)
        do {
            let result = try await client.publish(payload)
            print("published \(result.slug) v\(result.version)")
            print("sha256: \(result.sha256)")
            let detailURL = baseURL
                .appendingPathComponent("v1")
                .appendingPathComponent("recipes")
                .appendingPathComponent(result.slug)
            print("curl -fsSL \(detailURL.absoluteString)")
        } catch {
            FileHandle.standardError.write("publish failed: \(error)\n".data(using: .utf8)!)
            throw ExitCode.failure
        }
    }

    // MARK: - Helpers

    private func readIfExists(_ url: URL) -> String? {
        guard FileManager.default.fileExists(atPath: url.path) else { return nil }
        return try? String(contentsOf: url, encoding: .utf8)
    }

    /// Sibling `recipe.json` (or `recipe.yml`) — optional metadata.
    /// JSON only for v1; we don't ship a YAML decoder.
    private struct FileMetadata: Decodable {
        let name: String?
        let namespace: String?
        let displayName: String?
        let summary: String?
        let author: String?
        let tags: [String]?
    }

    private func readMetadataFile(_ url: URL) -> FileMetadata? {
        guard FileManager.default.fileExists(atPath: url.path),
              let data = try? Data(contentsOf: url)
        else { return nil }
        return try? JSONDecoder().decode(FileMetadata.self, from: data)
    }

    private func defaultName(for recipe: Recipe) -> String {
        // recipe.name is already lowercase-ish in our convention; lower it
        // explicitly and replace stray spaces. Real slug validation runs
        // after this, so a bad name will be rejected with a clear error.
        recipe.name.lowercased().replacingOccurrences(of: " ", with: "-")
    }

    private func validateSlug(_ slug: String) -> Bool {
        // Server-side regex is /^[a-z0-9][a-z0-9-]{1,63}$/ per segment.
        let parts = slug.split(separator: "/").map(String.init)
        guard parts.count == 2 else { return false }
        for p in parts {
            guard let first = p.first,
                  (first.isLowercase && first.isLetter) || first.isNumber
            else { return false }
            guard p.count >= 2, p.count <= 64 else { return false }
            for c in p {
                let ok = (c.isLowercase && c.isLetter) || c.isNumber || c == "-"
                if !ok { return false }
            }
        }
        return true
    }

    private func printPayload(_ payload: HubPublishPayload) throws {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes]
        let data = try encoder.encode(payload)
        if let s = String(data: data, encoding: .utf8) {
            print(s)
        }
    }
}
