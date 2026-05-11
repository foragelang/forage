import Testing
import Foundation
@testable import Forage

// MARK: - Value-type round trips

@Test
func runResultRoundTripsHashable() {
    let report = DiagnosticReport(
        stallReason: "settled",
        unmatchedCaptures: [
            UnmatchedCapture(url: "https://x.test/a", method: "GET", status: 200, bodyBytes: 12),
        ],
        unfiredRules: ["api/missing"],
        unmetExpectations: [],
        unhandledAffordances: ["Load more"]
    )
    let snapshot = Snapshot(
        records: [ScrapedRecord(typeName: "Item", fields: ["id": .string("x")])],
        observedAt: Date(timeIntervalSince1970: 1_700_000_000)
    )
    let a = RunResult(snapshot: snapshot, report: report)
    let b = RunResult(snapshot: snapshot, report: report)
    #expect(a == b)
    #expect(a.hashValue == b.hashValue)

    // Differ in one field → unequal.
    let differentReport = DiagnosticReport(stallReason: "hard-timeout")
    let c = RunResult(snapshot: snapshot, report: differentReport)
    #expect(a != c)
}

@Test
func diagnosticReportInitDefaultsAreEmpty() {
    let r = DiagnosticReport(stallReason: "completed")
    #expect(r.stallReason == "completed")
    #expect(r.unmatchedCaptures.isEmpty)
    #expect(r.unfiredRules.isEmpty)
    #expect(r.unmetExpectations.isEmpty)
    #expect(r.unhandledAffordances.isEmpty)
}

@Test
func unmatchedCaptureRoundTripsHashable() {
    let a = UnmatchedCapture(url: "https://x.test/y", method: "POST", status: 204, bodyBytes: 0)
    let b = UnmatchedCapture(url: "https://x.test/y", method: "POST", status: 204, bodyBytes: 0)
    #expect(a == b)
    #expect(a.hashValue == b.hashValue)

    let c = UnmatchedCapture(url: "https://x.test/y", method: "POST", status: 500, bodyBytes: 0)
    #expect(a != c)
}

/// `BrowserEngine.projectedUnmatchedCaptures()` discards the body and keeps
/// only the URL / method / status / byte count. Exercise the projection
/// shape directly so the unit test doesn't need a `WKWebView`.
@Test
func unmatchedCaptureProjectsFromCapture() {
    let cap = Capture(
        timestamp: Date(timeIntervalSince1970: 0),
        kind: .fetch,
        method: "GET",
        requestUrl: "https://x.test/req",
        responseUrl: "https://x.test/resp",
        requestBody: "",
        status: 418,
        bodyLength: 7,
        body: "teapot!"   // 7 UTF-8 bytes
    )
    let projected = UnmatchedCapture(
        url: cap.responseUrl,
        method: cap.method,
        status: cap.status,
        bodyBytes: cap.body.utf8.count
    )
    #expect(projected.url == "https://x.test/resp")
    #expect(projected.method == "GET")
    #expect(projected.status == 418)
    #expect(projected.bodyBytes == 7)
}

// MARK: - HTTP engine integration

@Test
func httpEngineSuccessReportsCompleted() async throws {
    let recipe = Recipe(
        name: "diag-ok",
        engineKind: .http,
        types: [
            RecipeType(name: "Item", fields: [
                RecipeField(name: "id", type: .string, optional: false),
            ]),
        ],
        body: [
            .step(HTTPStep(
                name: "items",
                request: HTTPRequest(method: "GET", url: Template(literal: "https://example.com/items"))
            )),
            .forLoop(
                variable: "item",
                collection: .field(.variable("items"), "data"),
                body: [
                    .emit(Emission(
                        typeName: "Item",
                        bindings: [
                            FieldBinding(fieldName: "id", expr: .path(.field(.variable("item"), "id"))),
                        ]
                    ))
                ]
            )
        ]
    )

    let mock = MockTransport()
    try await mock.registerJSON(
        urlSubstring: "items",
        json: ["data": [["id": "a"], ["id": "b"]]]
    )
    let client = HTTPClient(transport: mock, minRequestInterval: 0)
    let engine = HTTPEngine(client: client)

    let result = await engine.run(recipe: recipe, inputs: [:])
    #expect(result.snapshot.records.count == 2)
    #expect(result.report.stallReason == "completed")
    #expect(result.report.unmatchedCaptures.isEmpty)
    #expect(result.report.unfiredRules.isEmpty)
    #expect(result.report.unmetExpectations.isEmpty)
    #expect(result.report.unhandledAffordances.isEmpty)
}

@Test
func httpEngineFailureReportsFailedWithEmptySnapshot() async throws {
    let recipe = Recipe(
        name: "diag-fail",
        engineKind: .http,
        types: [
            RecipeType(name: "Item", fields: [
                RecipeField(name: "id", type: .string, optional: false),
            ]),
        ],
        body: [
            .step(HTTPStep(
                name: "items",
                request: HTTPRequest(method: "GET", url: Template(literal: "https://example.com/never"))
            )),
        ]
    )

    // No matchers registered → transport throws on send. maxRetries: 0 keeps
    // the test instant.
    let mock = MockTransport()
    let client = HTTPClient(transport: mock, minRequestInterval: 0, maxRetries: 0)
    let engine = HTTPEngine(client: client)

    let result = await engine.run(recipe: recipe, inputs: [:])
    #expect(result.report.stallReason.hasPrefix("failed: "))
    #expect(result.snapshot.records.isEmpty)
    #expect(result.report.unmatchedCaptures.isEmpty)
    #expect(result.report.unfiredRules.isEmpty)
}

@Test
func httpEngineFailureMidWalkKeepsRecordsEmittedSoFar() async throws {
    // Two-step recipe: first step succeeds and emits a record, second step
    // hits an unregistered URL and throws. The returned snapshot should
    // carry the first record; stallReason should be "failed: …".
    let recipe = Recipe(
        name: "diag-partial",
        engineKind: .http,
        types: [
            RecipeType(name: "Item", fields: [
                RecipeField(name: "id", type: .string, optional: false),
            ]),
        ],
        body: [
            .step(HTTPStep(
                name: "first",
                request: HTTPRequest(method: "GET", url: Template(literal: "https://example.com/first"))
            )),
            .forLoop(
                variable: "x",
                collection: .field(.variable("first"), "data"),
                body: [
                    .emit(Emission(
                        typeName: "Item",
                        bindings: [
                            FieldBinding(fieldName: "id", expr: .path(.field(.variable("x"), "id"))),
                        ]
                    ))
                ]
            ),
            .step(HTTPStep(
                name: "second",
                request: HTTPRequest(method: "GET", url: Template(literal: "https://example.com/never"))
            )),
        ]
    )

    let mock = MockTransport()
    try await mock.registerJSON(urlSubstring: "first", json: ["data": [["id": "kept"]]])
    let client = HTTPClient(transport: mock, minRequestInterval: 0, maxRetries: 0)
    let engine = HTTPEngine(client: client)

    let result = await engine.run(recipe: recipe, inputs: [:])
    #expect(result.report.stallReason.hasPrefix("failed: "))
    #expect(result.snapshot.records.count == 1)
    #expect(result.snapshot.records.first?.fields["id"] == .string("kept"))
}
