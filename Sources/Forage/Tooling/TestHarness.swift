import Foundation

/// `forage test`'s entire flow, factored out so the test suite can drive it
/// without going through ArgumentParser.
///
/// Exit semantics:
///   .ok         — recipe ran clean and snapshot matched (or no expected file)
///   .diff       — snapshot mismatch OR unmet expectations
///   .setupError — bad recipe / missing files / parse / validate / run failure
public enum TestHarness {
    public enum Exit: Sendable {
        case ok
        case diff
        case setupError
    }

    public struct Outcome: Sendable {
        public let stdout: String
        public let stderr: String
        public let exit: Exit

        public init(stdout: String, stderr: String, exit: Exit) {
            self.stdout = stdout
            self.stderr = stderr
            self.exit = exit
        }
    }

    public static func run(recipeDir: String, update: Bool) async -> Outcome {
        let fm = FileManager.default
        let recipePath = (recipeDir as NSString).appendingPathComponent("recipe.forage")
        let fixturesDir = (recipeDir as NSString).appendingPathComponent("fixtures")
        let expectedPath = (recipeDir as NSString).appendingPathComponent("expected.snapshot.json")
        let inputsPath = (fixturesDir as NSString).appendingPathComponent("inputs.json")

        guard fm.fileExists(atPath: recipePath) else {
            return Outcome(stdout: "", stderr: "missing recipe.forage in \(recipeDir)", exit: .setupError)
        }
        let src: String
        do {
            src = try String(contentsOfFile: recipePath, encoding: .utf8)
        } catch {
            return Outcome(stdout: "", stderr: "read failed: \(error)", exit: .setupError)
        }
        let recipe: Recipe
        do {
            recipe = try Parser.parse(source: src)
        } catch {
            return Outcome(stdout: "", stderr: "parse failed: \(error)", exit: .setupError)
        }
        let issues = Validator.validate(recipe)
        if issues.hasErrors {
            let msg = issues.errors.map { " - \($0.message) [\($0.location)]" }.joined(separator: "\n")
            return Outcome(stdout: "", stderr: "validation failed:\n\(msg)", exit: .setupError)
        }

        let inputs: [String: JSONValue]
        do {
            inputs = try loadInputs(at: inputsPath)
        } catch {
            return Outcome(stdout: "", stderr: "inputs load failed: \(error)", exit: .setupError)
        }

        let result: RunResult
        do {
            result = try await runRecipe(recipe: recipe, fixturesDir: fixturesDir, inputs: inputs)
        } catch {
            return Outcome(stdout: "", stderr: "run failed: \(error)", exit: .setupError)
        }

        if update {
            do {
                let data = try SnapshotIO.encode(result.snapshot)
                try data.write(to: URL(fileURLWithPath: expectedPath))
            } catch {
                return Outcome(stdout: "", stderr: "write failed: \(error)", exit: .setupError)
            }
            return Outcome(stdout: "updated \(expectedPath)", stderr: "", exit: .ok)
        }

        let unmet = result.report.unmetExpectations

        if fm.fileExists(atPath: expectedPath) {
            let expected: Snapshot
            do {
                let expectedData = try Data(contentsOf: URL(fileURLWithPath: expectedPath))
                expected = try SnapshotIO.decode(expectedData)
            } catch {
                return Outcome(stdout: "", stderr: "expected snapshot decode failed: \(error)", exit: .setupError)
            }
            let diff = SnapshotDiff.compare(expected: expected, actual: result.snapshot)
            let unmetBlock = renderUnmet(unmet)
            if diff.isMatch && unmet.isEmpty {
                return Outcome(stdout: "ok", stderr: "snapshot OK", exit: .ok)
            }
            var stderr = diff.isMatch ? "snapshot OK" : diff.render()
            if !unmetBlock.isEmpty {
                if !stderr.isEmpty { stderr += "\n" }
                stderr += unmetBlock
            }
            return Outcome(stdout: "", stderr: stderr, exit: .diff)
        }

        let data: Data
        do {
            data = try SnapshotIO.encode(result.snapshot)
        } catch {
            return Outcome(stdout: "", stderr: "snapshot encode failed: \(error)", exit: .setupError)
        }
        let stdout = String(data: data, encoding: .utf8) ?? "{}"
        let hint = "no expected.snapshot.json; rerun with --update to capture as golden"
        let unmetBlock = renderUnmet(unmet)
        let stderr = unmetBlock.isEmpty ? hint : "\(hint)\n\(unmetBlock)"
        let exit: Exit = unmet.isEmpty ? .ok : .diff
        return Outcome(stdout: stdout, stderr: stderr, exit: exit)
    }

    private static func renderUnmet(_ unmet: [String]) -> String {
        guard !unmet.isEmpty else { return "" }
        return "unmet expectations:\n" + unmet.map { " - \($0)" }.joined(separator: "\n")
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
            out.append(contentsOf: try parseHTTPFixtureJSONL(data))
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

/// Set-equality diff of two snapshots. Records compare as `(typeName,
/// fields)` tuples; ordering is ignored. Diff output lists every missing
/// (in expected, not in actual) and every extra (in actual, not in
/// expected). Two records that differ on a single field show up as one
/// missing + one extra — the harness doesn't try to be clever about
/// "this looks like a renamed version of that."
public enum SnapshotDiff {
    public struct Result: Sendable {
        public let isMatch: Bool
        public let missing: [ScrapedRecord]
        public let extra: [ScrapedRecord]

        public init(isMatch: Bool, missing: [ScrapedRecord], extra: [ScrapedRecord]) {
            self.isMatch = isMatch
            self.missing = missing
            self.extra = extra
        }

        public func render() -> String {
            if isMatch { return "ok" }
            var lines: [String] = []
            lines.append("snapshot mismatch: \(missing.count) missing, \(extra.count) extra")
            for r in missing { lines.append("  - missing: \(describe(r))") }
            for r in extra { lines.append("  + extra:   \(describe(r))") }
            return lines.joined(separator: "\n")
        }

        private func describe(_ r: ScrapedRecord) -> String {
            let pairs = r.fields.keys.sorted().map { "\($0)=\(r.fields[$0]!)" }
            return "\(r.typeName) { \(pairs.joined(separator: ", ")) }"
        }
    }

    public static func compare(expected: Snapshot, actual: Snapshot) -> Result {
        let e = Set(expected.records)
        let a = Set(actual.records)
        if e == a {
            return Result(isMatch: true, missing: [], extra: [])
        }
        let missing = e.subtracting(a).sorted(by: recordOrdering)
        let extra = a.subtracting(e).sorted(by: recordOrdering)
        return Result(isMatch: false, missing: missing, extra: extra)
    }

    /// Stable display order: by typeName, then by a sorted-pairs rendering
    /// of the fields. Diff readability matters more than perf here.
    private static func recordOrdering(_ x: ScrapedRecord, _ y: ScrapedRecord) -> Bool {
        if x.typeName != y.typeName { return x.typeName < y.typeName }
        return describeKey(x) < describeKey(y)
    }

    private static func describeKey(_ r: ScrapedRecord) -> String {
        r.fields.keys.sorted().map { "\($0)=\(r.fields[$0]!)" }.joined(separator: ",")
    }
}
