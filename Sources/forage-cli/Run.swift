import ArgumentParser
import Foundation
import AppKit
import Forage

struct RunCommand: AsyncParsableCommand {
    static let configuration = CommandConfiguration(
        commandName: "run",
        abstract: "Parse and execute a .forage recipe; print the snapshot as JSON."
    )

    @Argument(help: "Path to a `.forage` recipe file (or a recipe directory containing recipe.forage).")
    var recipePath: String

    @Option(name: .customLong("input"), parsing: .singleValue,
            help: "k=v input bindings. Numbers, true/false/null, and JSON literals are parsed; everything else is a string.")
    var inputs: [String] = []

    func run() async throws {
        let path = try Self.resolveRecipePath(recipePath)
        let src: String
        do {
            src = try String(contentsOfFile: path, encoding: .utf8)
        } catch {
            FileHandle.standardError.write("read failed: \(error)\n".data(using: .utf8)!)
            throw ExitCode.failure
        }

        var recipe: Recipe
        do {
            recipe = try Parser.parse(source: src)
        } catch {
            FileHandle.standardError.write("parse failed: \(error)\n".data(using: .utf8)!)
            throw ExitCode.failure
        }

        if !recipe.imports.isEmpty {
            do {
                let client = HubClient(
                    baseURL: Self.hubURL(),
                    token: ProcessInfo.processInfo.environment["FORAGE_HUB_TOKEN"]
                )
                let importer = RecipeImporter(
                    client: client,
                    cacheRoot: RecipeImporter.defaultCacheRoot()
                )
                recipe = try await importer.flatten(recipe)
            } catch {
                FileHandle.standardError.write("import failed: \(error)\n".data(using: .utf8)!)
                throw ExitCode.failure
            }
        }

        let issues = Validator.validate(recipe)
        if issues.hasErrors {
            FileHandle.standardError.write("validation failed:\n".data(using: .utf8)!)
            for e in issues.errors {
                FileHandle.standardError.write(" - \(e.message) [\(e.location)]\n".data(using: .utf8)!)
            }
            throw ExitCode.failure
        }
        for w in issues.warnings {
            FileHandle.standardError.write("warning: \(w.message) [\(w.location)]\n".data(using: .utf8)!)
        }

        let inputValues = Self.parseInputs(inputs)

        if recipe.engineKind == .browser {
            try await runBrowser(recipe: recipe, inputs: inputValues)
        } else {
            let runner = RecipeRunner(httpClient: HTTPClient(transport: URLSessionTransport()))
            let result = try await runner.run(recipe: recipe, inputs: inputValues)
            try Self.emitResult(result)
        }
    }

    @MainActor
    private func runBrowser(recipe: Recipe, inputs: [String: JSONValue]) async throws {
        // BrowserEngine needs an active NSApplication. Set the activation policy
        // up front; the event loop will be pumped by the awaited continuation
        // via WebKit's main-thread runloop integration.
        let app = NSApplication.shared
        app.setActivationPolicy(.regular)
        app.activate(ignoringOtherApps: true)

        let engine = BrowserEngine(recipe: recipe, inputs: inputs)
        do {
            let result = try await engine.run()
            try Self.emitResult(result)
        } catch {
            FileHandle.standardError.write("run failed: \(error)\n".data(using: .utf8)!)
            throw ExitCode.failure
        }
    }

    // MARK: - Helpers

    /// Resolve a recipe path to a `.forage` file. Accepts a path to a file or
    /// a directory containing `recipe.forage`.
    static func resolveRecipePath(_ path: String) throws -> String {
        let fm = FileManager.default
        var isDir: ObjCBool = false
        guard fm.fileExists(atPath: path, isDirectory: &isDir) else {
            FileHandle.standardError.write("no such recipe: \(path)\n".data(using: .utf8)!)
            throw ExitCode.failure
        }
        if isDir.boolValue {
            let nested = (path as NSString).appendingPathComponent("recipe.forage")
            guard fm.fileExists(atPath: nested) else {
                FileHandle.standardError.write("recipe directory missing recipe.forage: \(path)\n".data(using: .utf8)!)
                throw ExitCode.failure
            }
            return nested
        }
        return path
    }

    /// Parse `k=v` strings into `[String: JSONValue]`. Numeric → int/double,
    /// `true`/`false`/`null` literal, JSON literals (`[…]` / `{…}`) decoded,
    /// else string.
    static func parseInputs(_ raw: [String]) -> [String: JSONValue] {
        var out: [String: JSONValue] = [:]
        for kv in raw {
            guard let eq = kv.firstIndex(of: "=") else { continue }
            let key = String(kv[..<eq])
            let value = String(kv[kv.index(after: eq)...])
            out[key] = parseInputValue(value)
        }
        return out
    }

    static func parseInputValue(_ raw: String) -> JSONValue {
        if let i = Int(raw) { return .int(i) }
        if let d = Double(raw) { return .double(d) }
        if raw == "true" { return .bool(true) }
        if raw == "false" { return .bool(false) }
        if raw == "null" { return .null }
        if raw.hasPrefix("[") || raw.hasPrefix("{") {
            if let data = raw.data(using: .utf8), let v = try? JSONValue.decode(data) { return v }
        }
        return .string(raw)
    }

    static func emitResult(_ result: RunResult) throws {
        let data = try SnapshotIO.encode(result.snapshot)
        FileHandle.standardOutput.write(data)
        FileHandle.standardOutput.write("\n".data(using: .utf8)!)
        FileHandle.standardError.write("stallReason: \(result.report.stallReason)\n".data(using: .utf8)!)
    }

    /// Hub URL from `FORAGE_HUB_URL` env var (else the production default).
    /// Trailing slashes stripped so path-joining is unambiguous.
    static func hubURL() -> URL {
        let raw = ProcessInfo.processInfo.environment["FORAGE_HUB_URL"] ?? ""
        let trimmed = raw.trimmingCharacters(in: CharacterSet(charactersIn: "/ "))
        if trimmed.isEmpty {
            return HubClient.defaultBaseURL
        }
        return URL(string: trimmed) ?? HubClient.defaultBaseURL
    }
}
