#if canImport(WebKit)
import Testing
import Foundation
@testable import Forage

// MARK: - Fixtures

private func sampleCaptures(count: Int = 3) -> [Capture] {
    (0..<count).map { i in
        Capture(
            timestamp: Date(timeIntervalSince1970: 1_715_000_000 + Double(i)),
            kind: .fetch,
            method: "GET",
            requestUrl: "https://x.test/req/\(i)",
            responseUrl: "https://x.test/resp/\(i)",
            requestBody: "",
            status: 200,
            bodyLength: 5,
            body: "hello"
        )
    }
}

private func makeTempDir() throws -> URL {
    let url = FileManager.default.temporaryDirectory
        .appendingPathComponent("forage-replayer-tests", isDirectory: true)
        .appendingPathComponent(UUID().uuidString, isDirectory: true)
    try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
    return url
}

private func writeJSONL(_ captures: [Capture]) throws -> Data {
    let encoder = JSONEncoder()
    encoder.dateEncodingStrategy = .iso8601
    encoder.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
    var out = Data()
    for capture in captures {
        let line = try encoder.encode(capture)
        out.append(line)
        out.append(0x0A) // \n
    }
    return out
}

// MARK: - Direct init

@Test
func replayerInitFromCapturesRoundTrips() {
    let caps = sampleCaptures(count: 4)
    let replayer = BrowserReplayer(captures: caps)
    #expect(replayer.captures == caps)
}

@Test
func replayerInitFromEmptyCapturesIsValid() {
    let replayer = BrowserReplayer(captures: [])
    #expect(replayer.captures.isEmpty)
}

// MARK: - File init

@Test
func replayerReadsJSONLFile() throws {
    let caps = sampleCaptures(count: 3)
    let dir = try makeTempDir()
    let file = dir.appendingPathComponent("captures.jsonl")
    try writeJSONL(caps).write(to: file)

    let replayer = try BrowserReplayer(capturesFile: file)
    #expect(replayer.captures == caps)
}

@Test
func replayerReadsJSONLWithoutTrailingNewline() throws {
    let caps = sampleCaptures(count: 2)
    let dir = try makeTempDir()
    let file = dir.appendingPathComponent("captures.jsonl")
    var data = try writeJSONL(caps)
    // Strip the final newline so the last line isn't terminated.
    if data.last == 0x0A { data.removeLast() }
    try data.write(to: file)

    let replayer = try BrowserReplayer(capturesFile: file)
    #expect(replayer.captures == caps)
}

@Test
func replayerOnMissingFileThrows() {
    let nonexistent = FileManager.default.temporaryDirectory
        .appendingPathComponent("forage-replayer-missing-\(UUID().uuidString).jsonl")
    #expect(throws: (any Error).self) {
        _ = try BrowserReplayer(capturesFile: nonexistent)
    }
}

@Test
func replayerOnEmptyFileIsEmpty() throws {
    let dir = try makeTempDir()
    let file = dir.appendingPathComponent("captures.jsonl")
    try Data().write(to: file)

    let replayer = try BrowserReplayer(capturesFile: file)
    #expect(replayer.captures.isEmpty)
}

// MARK: - Archive round-trip

@Test
func replayerReadsArchiveCapturesJSONL() throws {
    let observedAt = Date(timeIntervalSince1970: 1_715_000_000)
    let snapshot = Snapshot(
        records: [
            ScrapedRecord(typeName: "Item", fields: ["id": .string("a")]),
        ],
        observedAt: observedAt
    )
    let report = DiagnosticReport(
        stallReason: "settled",
        unmatchedCaptures: [],
        unfiredRules: [],
        unmetExpectations: [],
        unhandledAffordances: []
    )
    let caps = sampleCaptures(count: 5)
    let meta = ArchiveMeta(
        recipeName: "jane",
        inputs: [:],
        runtimeSeconds: 1.0,
        observedAt: observedAt
    )
    let root = try makeTempDir()
    let handle = try Archive.write(
        root: root, slug: "round-trip",
        snapshot: snapshot, report: report,
        captures: caps, meta: meta
    )

    let replayer = try BrowserReplayer(
        capturesFile: handle.directory.appendingPathComponent("captures.jsonl")
    )
    #expect(replayer.captures == caps)
}

// MARK: - BrowserEngine integration

/// Minimal recipe: a single `Product` type and one capture rule that emits a
/// `Product` per item in `$.products` from any response URL containing
/// `api/products`. Mirrors the smallest realistic shape a recipe author
/// would write — enough to prove the replay path drives `applyRule` end-to-
/// end.
@MainActor
private func minimalReplayRecipe() -> Recipe {
    let emit = Emission(
        typeName: "Product",
        bindings: [
            FieldBinding(fieldName: "id",
                         expr: .path(.field(.current, "id"))),
            FieldBinding(fieldName: "name",
                         expr: .path(.field(.current, "name"))),
        ]
    )
    let body: [Statement] = [
        .forLoop(
            variable: "p",
            collection: .path(.field(.current, "products")),
            body: [.emit(emit)]
        )
    ]
    let captureRule = CaptureRule(
        urlPattern: "api/products",
        iterPath: .path(.field(.current, "products")),
        body: body
    )
    let bcfg = BrowserConfig(
        initialURL: Template(literal: "https://unused.test/"),
        observe: "api/products",
        pagination: BrowserPaginationConfig(
            mode: .scroll,
            until: .noProgressFor(3)
        ),
        captures: [captureRule]
    )
    return Recipe(
        name: "test-replay",
        engineKind: .browser,
        types: [
            RecipeType(name: "Product", fields: [
                RecipeField(name: "id", type: .string, optional: false),
                RecipeField(name: "name", type: .string, optional: false),
            ]),
        ],
        browser: bcfg
    )
}

@MainActor
@Test
func browserEngineRunsThroughReplayer() async throws {
    let recipe = minimalReplayRecipe()
    let body = #"""
    {"products": [{"id": "a", "name": "Alpha"}, {"id": "b", "name": "Beta"}]}
    """#
    let capture = Capture(
        timestamp: Date(timeIntervalSince1970: 1_715_000_000),
        kind: .fetch,
        method: "GET",
        requestUrl: "https://example.test/api/products?page=1",
        responseUrl: "https://example.test/api/products?page=1",
        requestBody: "",
        status: 200,
        bodyLength: body.utf8.count,
        body: body
    )
    let replayer = BrowserReplayer(captures: [capture])
    let engine = BrowserEngine(
        recipe: recipe,
        inputs: [:],
        visible: false,
        replayer: replayer
    )
    let result = try await engine.run()

    #expect(engine.progress.phase == .done)
    #expect(result.snapshot.records.count == 2)
    let names = result.snapshot.records.compactMap { r -> String? in
        if case .string(let s) = r.fields["name"] { return s } else { return nil }
    }
    #expect(names == ["Alpha", "Beta"])
    #expect(result.report.stallReason == "settled")
    // The one rule matched, so it's not in unfiredRules.
    #expect(result.report.unfiredRules.isEmpty)
    #expect(result.report.unmatchedCaptures.isEmpty)
}

@MainActor
@Test
func browserEngineWithEmptyReplayerFinishesCleanly() async throws {
    let recipe = minimalReplayRecipe()
    let engine = BrowserEngine(
        recipe: recipe,
        inputs: [:],
        visible: false,
        replayer: BrowserReplayer(captures: [])
    )
    let result = try await engine.run()

    #expect(engine.progress.phase == .done)
    #expect(result.snapshot.records.isEmpty)
    // Rule never matched anything → it shows up in unfiredRules.
    #expect(result.report.unfiredRules == ["api/products"])
}

@MainActor
@Test
func browserEngineReplayerCountsUnmatchedCaptures() async throws {
    let recipe = minimalReplayRecipe()
    let unrelated = Capture(
        timestamp: Date(timeIntervalSince1970: 1_715_000_000),
        kind: .fetch,
        method: "GET",
        requestUrl: "https://example.test/api/other",
        responseUrl: "https://example.test/api/other",
        requestBody: "",
        status: 200,
        bodyLength: 2,
        body: "{}"
    )
    let engine = BrowserEngine(
        recipe: recipe,
        inputs: [:],
        visible: false,
        replayer: BrowserReplayer(captures: [unrelated])
    )
    let result = try await engine.run()

    #expect(engine.progress.phase == .done)
    #expect(result.snapshot.records.isEmpty)
    #expect(result.report.unmatchedCaptures.count == 1)
    #expect(result.report.unmatchedCaptures.first?.url == "https://example.test/api/other")
    #expect(result.report.unfiredRules == ["api/products"])
}

// MARK: - Cancellation

/// Constructing a live (no-replayer) engine and immediately cancelling its
/// host Task should finish with `stallReason: "cancelled"` and
/// `phase: .failed("cancelled")`, not hang on the 8s settle or 240s hard
/// timeout. We use generous safety timeouts (30s hard, 8s settle) so that
/// if the cancellation handler is wired wrong the test fails by hanging on
/// the await; if it's wired right, the result returns in milliseconds.
@MainActor
@Test
func browserEngineHonorsTaskCancellation() async throws {
    let recipe = minimalReplayRecipe()
    let engine = BrowserEngine(
        recipe: recipe,
        inputs: [:],
        visible: false,
        settleSeconds: 8,
        hardTimeoutSeconds: 30
    )

    let task = Task { @MainActor in
        try await engine.run()
    }

    // Yield once so the engine's `start()` runs and the continuation is set
    // before we cancel. Without this we'd race the cancellation against
    // continuation setup and the onCancel handler might find no
    // continuation to resume.
    await Task.yield()
    task.cancel()

    let result = try await task.value

    #expect(result.report.stallReason == "cancelled")
    #expect(engine.progress.phase == .failed("cancelled"))
}
#endif
