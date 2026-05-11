import ArgumentParser
import Foundation
import Forage

struct TestCommand: AsyncParsableCommand {
    static let configuration = CommandConfiguration(
        commandName: "test",
        abstract: "Run a recipe against fixtures and diff against an expected snapshot."
    )

    @Argument(help: "Recipe directory containing recipe.forage, fixtures/, and optional expected.snapshot.json.")
    var recipeDir: String

    @Flag(name: .customLong("update"),
          help: "Write the produced snapshot to expected.snapshot.json (golden-file workflow).")
    var update: Bool = false

    func run() async throws {
        let outcome = try await TestHarness.run(recipeDir: recipeDir, update: update)
        FileHandle.standardOutput.write((outcome.stdout + "\n").data(using: .utf8)!)
        if !outcome.stderr.isEmpty {
            FileHandle.standardError.write((outcome.stderr + "\n").data(using: .utf8)!)
        }
        if !outcome.success {
            throw ExitCode.failure
        }
    }
}

/// `forage test`'s entire flow, factored out so the test suite can drive it
/// without going through ArgumentParser.
public enum TestHarness {
    public struct Outcome: Sendable {
        public let stdout: String
        public let stderr: String
        public let success: Bool
    }

    public static func run(recipeDir: String, update: Bool) async throws -> Outcome {
        let fm = FileManager.default
        let recipePath = (recipeDir as NSString).appendingPathComponent("recipe.forage")
        let fixturesDir = (recipeDir as NSString).appendingPathComponent("fixtures")
        let expectedPath = (recipeDir as NSString).appendingPathComponent("expected.snapshot.json")
        let inputsPath = (fixturesDir as NSString).appendingPathComponent("inputs.json")

        guard fm.fileExists(atPath: recipePath) else {
            return Outcome(stdout: "", stderr: "missing recipe.forage in \(recipeDir)", success: false)
        }

        let src = try String(contentsOfFile: recipePath, encoding: .utf8)
        let recipe: Recipe
        do {
            recipe = try Parser.parse(source: src)
        } catch {
            return Outcome(stdout: "", stderr: "parse failed: \(error)", success: false)
        }
        let issues = Validator.validate(recipe)
        if issues.hasErrors {
            let msg = issues.errors.map { " - \($0.message) [\($0.location)]" }.joined(separator: "\n")
            return Outcome(stdout: "", stderr: "validation failed:\n\(msg)", success: false)
        }

        let inputs = try loadInputs(at: inputsPath)
        let result: RunResult
        do {
            result = try await runRecipe(recipe: recipe, fixturesDir: fixturesDir, inputs: inputs)
        } catch {
            return Outcome(stdout: "", stderr: "run failed: \(error)", success: false)
        }

        if update {
            let data = try SnapshotIO.encode(result.snapshot)
            try data.write(to: URL(fileURLWithPath: expectedPath))
            return Outcome(
                stdout: "updated \(expectedPath)",
                stderr: "",
                success: true
            )
        }

        var unmetReport = ""
        if !result.report.unmetExpectations.isEmpty {
            unmetReport = "unmet expectations:\n" +
                result.report.unmetExpectations.map { " - \($0)" }.joined(separator: "\n")
        }

        if fm.fileExists(atPath: expectedPath) {
            let expectedData = try Data(contentsOf: URL(fileURLWithPath: expectedPath))
            let expected: Snapshot
            do {
                expected = try SnapshotIO.decode(expectedData)
            } catch {
                return Outcome(stdout: "", stderr: "expected snapshot decode failed: \(error)", success: false)
            }
            let diff = SnapshotDiff.compare(expected: expected, actual: result.snapshot)
            if diff.isMatch {
                let stderr = unmetReport.isEmpty ? "snapshot OK" : "snapshot OK; \(unmetReport)"
                return Outcome(stdout: "ok", stderr: stderr, success: unmetReport.isEmpty)
            } else {
                var stderr = diff.render()
                if !unmetReport.isEmpty { stderr += "\n" + unmetReport }
                return Outcome(stdout: "", stderr: stderr, success: false)
            }
        } else {
            // No expected snapshot — print actual and hint at --update.
            let data = try SnapshotIO.encode(result.snapshot)
            let stdout = String(data: data, encoding: .utf8) ?? "{}"
            let hint = "no expected.snapshot.json; rerun with --update to capture as golden"
            let stderr = unmetReport.isEmpty ? hint : "\(hint)\n\(unmetReport)"
            return Outcome(stdout: stdout, stderr: stderr, success: unmetReport.isEmpty)
        }
    }

    // MARK: - Inputs

    private static func loadInputs(at path: String) throws -> [String: JSONValue] {
        let fm = FileManager.default
        guard fm.fileExists(atPath: path) else { return [:] }
        let data = try Data(contentsOf: URL(fileURLWithPath: path))
        guard let dict = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            return [:]
        }
        var out: [String: JSONValue] = [:]
        for (k, v) in dict { out[k] = JSONValue(fromAny: v) }
        return out
    }

    // MARK: - Run dispatch

    private static func runRecipe(recipe: Recipe, fixturesDir: String, inputs: [String: JSONValue]) async throws -> RunResult {
        if recipe.engineKind == .browser {
            let captures = try loadBrowserCaptures(fixturesDir: fixturesDir)
            return try await runBrowserOnMain(recipe: recipe, inputs: inputs, captures: captures)
        } else {
            let fixtures = try loadHTTPFixtures(fixturesDir: fixturesDir)
            let replayer = HTTPReplayer(fixtures: fixtures)
            let runner = RecipeRunner(httpClient: HTTPClient(
                transport: replayer,
                minRequestInterval: 0,
                maxRetries: 0
            ))
            return try await runner.run(recipe: recipe, inputs: inputs)
        }
    }

    @MainActor
    private static func runBrowserOnMain(
        recipe: Recipe,
        inputs: [String: JSONValue],
        captures: [Capture]
    ) async throws -> RunResult {
        let replayer = BrowserReplayer(captures: captures)
        let engine = BrowserEngine(
            recipe: recipe,
            inputs: inputs,
            visible: false,
            replayer: replayer
        )
        return try await engine.run()
    }

    private static func loadBrowserCaptures(fixturesDir: String) throws -> [Capture] {
        let fm = FileManager.default
        var out: [Capture] = []
        guard fm.fileExists(atPath: fixturesDir) else { return out }
        let urls = try fm.contentsOfDirectory(
            at: URL(fileURLWithPath: fixturesDir),
            includingPropertiesForKeys: nil
        )
        for url in urls where url.pathExtension == "jsonl" {
            let replayer = try BrowserReplayer(capturesFile: url)
            out.append(contentsOf: replayer.captures)
        }
        return out
    }

    private static func loadHTTPFixtures(fixturesDir: String) throws -> [HTTPReplayer.Fixture] {
        let fm = FileManager.default
        var out: [HTTPReplayer.Fixture] = []
        guard fm.fileExists(atPath: fixturesDir) else { return out }
        let urls = try fm.contentsOfDirectory(
            at: URL(fileURLWithPath: fixturesDir),
            includingPropertiesForKeys: nil
        )
        for url in urls where url.pathExtension == "jsonl" {
            let data = try Data(contentsOf: url)
            for fixture in try parseHTTPFixtureJSONL(data) {
                out.append(fixture)
            }
        }
        return out
    }

    /// Each line is `{method, url, body?, status?, response, headers?}`.
    /// `response` is the raw response body (string or JSON-encoded object).
    private static func parseHTTPFixtureJSONL(_ data: Data) throws -> [HTTPReplayer.Fixture] {
        var out: [HTTPReplayer.Fixture] = []
        guard !data.isEmpty else { return out }
        var lineStart = data.startIndex
        for i in data.indices {
            if data[i] == 0x0A {
                if i > lineStart, let fx = decodeHTTPFixture(data[lineStart..<i]) { out.append(fx) }
                lineStart = data.index(after: i)
            }
        }
        if lineStart < data.endIndex, let fx = decodeHTTPFixture(data[lineStart..<data.endIndex]) {
            out.append(fx)
        }
        return out
    }

    private static func decodeHTTPFixture(_ slice: Data) -> HTTPReplayer.Fixture? {
        guard let obj = try? JSONSerialization.jsonObject(with: slice) as? [String: Any] else { return nil }
        let method = (obj["method"] as? String) ?? "GET"
        let url = (obj["url"] as? String) ?? (obj["urlSubstring"] as? String) ?? ""
        let body = obj["body"] as? String
        let status = (obj["status"] as? Int) ?? 200
        let headers = (obj["headers"] as? [String: String]) ?? ["Content-Type": "application/json"]

        let responseBody: Data
        if let s = obj["response"] as? String, let d = s.data(using: .utf8) {
            responseBody = d
        } else if let any = obj["response"] {
            responseBody = (try? JSONSerialization.data(withJSONObject: any)) ?? Data()
        } else {
            responseBody = Data()
        }

        return HTTPReplayer.Fixture(
            method: method,
            urlSubstring: url,
            bodySubstring: body,
            responseStatus: status,
            responseBody: responseBody,
            responseHeaders: headers
        )
    }
}

/// Minimal snapshot diff for the test harness. Compares records ignoring
/// `observedAt`; records are compared by full structural equality of
/// `_typeName` + `fields`. Order matters — recipes emit in a deterministic
/// order, and a reordering is real signal worth flagging.
public enum SnapshotDiff {
    public struct Result: Sendable {
        public let isMatch: Bool
        public let expectedCount: Int
        public let actualCount: Int
        public let firstMismatchIndex: Int?
        public let firstMismatchKind: MismatchKind?

        public enum MismatchKind: Sendable {
            case missing(ScrapedRecord)
            case extra(ScrapedRecord)
            case different(expected: ScrapedRecord, actual: ScrapedRecord)
        }

        public func render() -> String {
            if isMatch { return "ok" }
            var lines: [String] = []
            if expectedCount != actualCount {
                lines.append("record count differs: expected \(expectedCount), got \(actualCount)")
            }
            if let i = firstMismatchIndex, let kind = firstMismatchKind {
                switch kind {
                case .missing(let r):
                    lines.append("missing record at index \(i): \(describe(r))")
                case .extra(let r):
                    lines.append("extra record at index \(i): \(describe(r))")
                case .different(let e, let a):
                    lines.append("record at index \(i) differs:")
                    lines.append("  expected: \(describe(e))")
                    lines.append("  actual:   \(describe(a))")
                }
            }
            return lines.joined(separator: "\n")
        }

        private func describe(_ r: ScrapedRecord) -> String {
            let pairs = r.fields.keys.sorted().map { "\($0)=\(r.fields[$0]!)" }
            return "\(r.typeName) { \(pairs.joined(separator: ", ")) }"
        }
    }

    public static func compare(expected: Snapshot, actual: Snapshot) -> Result {
        let e = expected.records
        let a = actual.records
        let max = Swift.max(e.count, a.count)
        for i in 0..<max {
            switch (i < e.count, i < a.count) {
            case (true, true):
                if e[i] != a[i] {
                    return Result(
                        isMatch: false,
                        expectedCount: e.count,
                        actualCount: a.count,
                        firstMismatchIndex: i,
                        firstMismatchKind: .different(expected: e[i], actual: a[i])
                    )
                }
            case (true, false):
                return Result(
                    isMatch: false,
                    expectedCount: e.count,
                    actualCount: a.count,
                    firstMismatchIndex: i,
                    firstMismatchKind: .missing(e[i])
                )
            case (false, true):
                return Result(
                    isMatch: false,
                    expectedCount: e.count,
                    actualCount: a.count,
                    firstMismatchIndex: i,
                    firstMismatchKind: .extra(a[i])
                )
            default:
                break
            }
        }
        return Result(
            isMatch: true,
            expectedCount: e.count,
            actualCount: a.count,
            firstMismatchIndex: nil,
            firstMismatchKind: nil
        )
    }
}
