import Foundation
import AppKit
import Forage

/// Bridges the UI's "Run live / Run replay" buttons to the Forage runtime.
///
/// For HTTP-engine recipes we use `RecipeRunner` (live) or build an
/// `HTTPReplayer` from the recipe's `fixtures/` directory (replay). For
/// browser-engine recipes we drive `BrowserEngine` directly, which requires
/// being on the main actor and an active `NSApplication` (which the Toolkit
/// guarantees).
@MainActor
struct RunController {
    enum Mode {
        case live
        case replay
    }

    let mfaCoordinator: MFAPromptCoordinator?

    init(mfaCoordinator: MFAPromptCoordinator? = nil) {
        self.mfaCoordinator = mfaCoordinator
    }

    func run(entry: RecipeEntry, mode: Mode, source: String) async throws -> RunResult {
        let recipe = try Parser.parse(source: source)
        let issues = Validator.validate(recipe)
        if issues.hasErrors {
            let messages = issues.errors.map { "  - \($0.message) [\($0.location)]" }.joined(separator: "\n")
            throw RunError.validationFailed(messages)
        }

        switch recipe.engineKind {
        case .http:
            return try await runHTTP(recipe: recipe, entry: entry, mode: mode)
        case .browser:
            return try await runBrowser(recipe: recipe, entry: entry, mode: mode)
        }
    }

    // MARK: - HTTP

    private func runHTTP(recipe: Recipe, entry: RecipeEntry, mode: Mode) async throws -> RunResult {
        let inputs = try Self.loadInputs(at: entry)
        let mfa: MFAProvider? = mfaCoordinator.map { SheetMFAProvider(coordinator: $0) }
        switch mode {
        case .live:
            let runner = RecipeRunner(
                httpClient: HTTPClient(transport: URLSessionTransport()),
                secretResolver: EnvironmentSecretResolver(),
                mfaProvider: mfa
            )
            return try await runner.run(recipe: recipe, inputs: inputs)
        case .replay:
            let fixtures = try Self.loadHTTPFixtures(at: entry.fixturesDir)
            guard !fixtures.isEmpty else {
                throw RunError.noFixtures(entry.fixturesDir.path)
            }
            let replayer = HTTPReplayer(fixtures: fixtures)
            let runner = RecipeRunner(
                httpClient: HTTPClient(
                    transport: replayer,
                    minRequestInterval: 0,
                    maxRetries: 0
                ),
                secretResolver: EnvironmentSecretResolver(),
                mfaProvider: mfa
            )
            return try await runner.run(recipe: recipe, inputs: inputs)
        }
    }

    // MARK: - Browser

    private func runBrowser(recipe: Recipe, entry: RecipeEntry, mode: Mode) async throws -> RunResult {
        let inputs = try Self.loadInputs(at: entry)
        switch mode {
        case .live:
            // Toolkit's NSApp is already running; BrowserEngine spawns a
            // window and drives the SPA. Settle / timeout defaults match
            // the CLI.
            let engine = BrowserEngine(recipe: recipe, inputs: inputs, visible: true)
            return try await engine.run()
        case .replay:
            let captures = try Self.loadBrowserCaptures(at: entry.fixturesDir)
            guard !captures.isEmpty else {
                throw RunError.noFixtures(entry.capturesFile.path)
            }
            let replayer = BrowserReplayer(captures: captures)
            let engine = BrowserEngine(
                recipe: recipe,
                inputs: inputs,
                visible: false,
                replayer: replayer
            )
            return try await engine.run()
        }
    }

    // MARK: - Inputs

    /// `fixtures/inputs.json` follows the same convention `forage test`
    /// uses. Missing file → empty inputs; the validator will already have
    /// surfaced any required ones at edit time.
    private static func loadInputs(at entry: RecipeEntry) throws -> [String: JSONValue] {
        let url = entry.fixturesDir.appending(path: "inputs.json")
        guard FileManager.default.fileExists(atPath: url.path) else { return [:] }
        let data = try Data(contentsOf: url)
        guard let dict = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            return [:]
        }
        var out: [String: JSONValue] = [:]
        for (k, v) in dict { out[k] = JSONValue(fromAny: v) }
        return out
    }

    // MARK: - Fixture loading
    //
    // The Toolkit's on-disk layout matches `forage test`'s, so we mirror its
    // logic: any `.jsonl` under `fixtures/` is treated as either browser
    // captures or HTTP fixtures depending on the recipe's engine.

    private static func loadBrowserCaptures(at dir: URL) throws -> [Capture] {
        let fm = FileManager.default
        guard fm.fileExists(atPath: dir.path) else { return [] }
        let urls = try fm.contentsOfDirectory(at: dir, includingPropertiesForKeys: nil)
        var out: [Capture] = []
        for url in urls where url.pathExtension == "jsonl" {
            let replayer = try BrowserReplayer(capturesFile: url)
            out.append(contentsOf: replayer.captures)
        }
        return out
    }

    private static func loadHTTPFixtures(at dir: URL) throws -> [HTTPReplayer.Fixture] {
        let fm = FileManager.default
        guard fm.fileExists(atPath: dir.path) else { return [] }
        let urls = try fm.contentsOfDirectory(at: dir, includingPropertiesForKeys: nil)
        var out: [HTTPReplayer.Fixture] = []
        for url in urls where url.pathExtension == "jsonl" {
            let data = try Data(contentsOf: url)
            out.append(contentsOf: try parseHTTPFixtureJSONL(data))
        }
        return out
    }

    /// Each line is `{method, url, body?, status?, response, headers?}` or
    /// a `Capture`-shaped record (which we coerce to a fixture by treating
    /// the response body as the literal response).
    private static func parseHTTPFixtureJSONL(_ data: Data) throws -> [HTTPReplayer.Fixture] {
        var out: [HTTPReplayer.Fixture] = []
        guard !data.isEmpty else { return out }
        var lineStart = data.startIndex
        for i in data.indices {
            if data[i] == 0x0A {
                if i > lineStart, let fx = decodeFixture(data[lineStart..<i]) { out.append(fx) }
                lineStart = data.index(after: i)
            }
        }
        if lineStart < data.endIndex, let fx = decodeFixture(data[lineStart..<data.endIndex]) {
            out.append(fx)
        }
        return out
    }

    private static func decodeFixture(_ slice: Data) -> HTTPReplayer.Fixture? {
        guard let obj = try? JSONSerialization.jsonObject(with: slice) as? [String: Any] else { return nil }
        let method = (obj["method"] as? String) ?? "GET"
        let urlString = (obj["responseUrl"] as? String)
            ?? (obj["url"] as? String)
            ?? (obj["urlSubstring"] as? String)
            ?? ""
        let body = (obj["requestBody"] as? String) ?? (obj["body"] as? String)
        let status = (obj["status"] as? Int) ?? 200
        let headers = (obj["headers"] as? [String: String]) ?? ["Content-Type": "application/json"]
        let responseBody: Data
        if let s = obj["body"] as? String, !s.isEmpty, let d = s.data(using: .utf8) {
            // `Capture`-shaped row: body is the response.
            responseBody = d
        } else if let s = obj["response"] as? String, let d = s.data(using: .utf8) {
            responseBody = d
        } else if let any = obj["response"] {
            responseBody = (try? JSONSerialization.data(withJSONObject: any)) ?? Data()
        } else {
            responseBody = Data()
        }
        return HTTPReplayer.Fixture(
            method: method,
            urlSubstring: urlString,
            bodySubstring: body,
            responseStatus: status,
            responseBody: responseBody,
            responseHeaders: headers
        )
    }
}

enum RunError: Error, CustomStringConvertible {
    case validationFailed(String)
    case noFixtures(String)

    var description: String {
        switch self {
        case .validationFailed(let s):
            return "Recipe failed validation:\n\(s)"
        case .noFixtures(let path):
            return "No fixtures found at \(path). Use \"Capture\" to record some first."
        }
    }
}
