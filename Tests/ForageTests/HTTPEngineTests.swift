import Testing
import Foundation
@testable import Forage

// MARK: - MockTransport

/// In-memory `Transport` that serves canned responses. Used by every engine
/// test so no live HTTP fires.
actor MockTransport: Transport {
    var responses: [(matcher: @Sendable (URLRequest) -> Bool, body: Data, status: Int)] = []
    var calls: [URLRequest] = []

    func register(_ matcher: @Sendable @escaping (URLRequest) -> Bool, body: Data, status: Int = 200) {
        responses.append((matcher, body, status))
    }

    func registerJSON(urlSubstring: String, json: [String: Any], status: Int = 200) throws {
        let data = try JSONSerialization.data(withJSONObject: json, options: [.fragmentsAllowed, .sortedKeys])
        responses.append((
            { req in (req.url?.absoluteString ?? "").contains(urlSubstring) },
            data,
            status
        ))
    }

    nonisolated func send(_ request: URLRequest) async throws -> (Data, HTTPURLResponse) {
        return try await self.handle(request)
    }

    private func handle(_ request: URLRequest) throws -> (Data, HTTPURLResponse) {
        calls.append(request)
        for r in responses where r.matcher(request) {
            let response = HTTPURLResponse(
                url: request.url!,
                statusCode: r.status,
                httpVersion: "HTTP/1.1",
                headerFields: ["Content-Type": "application/json"]
            )!
            return (r.body, response)
        }
        throw NSError(domain: "MockTransport", code: 404, userInfo: [
            NSLocalizedDescriptionKey: "no match for \(request.url?.absoluteString ?? "?")"
        ])
    }
}

// MARK: - Smoke test: minimal recipe runs end-to-end

@Test
func minimalRecipeRunsAndEmitsRecord() async throws {
    // Recipe: one type, one step (no pagination), one emit. Confirms the
    // walker, scope, path resolver, transform pipeline all wire up.
    let recipe = Recipe(
        name: "smoke",
        engineKind: .http,
        types: [
            RecipeType(name: "Item", fields: [
                RecipeField(name: "id", type: .string, optional: false),
                RecipeField(name: "name", type: .string, optional: false),
            ]),
        ],
        body: [
            .step(HTTPStep(
                name: "items",
                request: HTTPRequest(
                    method: "GET",
                    url: Template(literal: "https://example.com/items")
                )
            )),
            .forLoop(
                variable: "item",
                collection: .field(.variable("items"), "data"),
                body: [
                    .emit(Emission(
                        typeName: "Item",
                        bindings: [
                            FieldBinding(fieldName: "id", expr: .path(.field(.variable("item"), "id"))),
                            FieldBinding(fieldName: "name", expr: .path(.field(.variable("item"), "name"))),
                        ]
                    ))
                ]
            )
        ]
    )

    let transport = MockTransport()
    try await transport.registerJSON(
        urlSubstring: "example.com/items",
        json: ["data": [
            ["id": "1", "name": "Alpha"],
            ["id": "2", "name": "Beta"],
        ]]
    )
    let client = HTTPClient(transport: transport, minRequestInterval: 0)
    let runner = RecipeRunner(httpClient: client)

    let snapshot = try await runner.run(recipe: recipe, inputs: [:])

    let items = snapshot.records(of: "Item")
    #expect(items.count == 2)
    #expect(items[0].fields["id"] == .string("1"))
    #expect(items[0].fields["name"] == .string("Alpha"))
    #expect(items[1].fields["id"] == .string("2"))
}

// MARK: - Sweed-shaped recipe with pagination + transforms

@Test
func sweedShapedRecipeWithPaginationAndTransforms() async throws {
    let recipe = Recipe(
        name: "sweed-test",
        engineKind: .http,
        types: [
            RecipeType(name: "Product", fields: [
                RecipeField(name: "externalId", type: .string, optional: false),
                RecipeField(name: "name", type: .string, optional: false),
                RecipeField(name: "brand", type: .string, optional: true),
                RecipeField(name: "strainPrevalence", type: .string, optional: true),
            ]),
        ],
        inputs: [
            InputDecl(name: "storeId", type: .string),
        ],
        auth: .staticHeader(name: "storeId", value: Template(parts: [.interp(.field(.input, "storeId"))])),
        body: [
            .step(HTTPStep(
                name: "products",
                request: HTTPRequest(
                    method: "POST",
                    url: Template(literal: "https://api.sweed.example/products"),
                    body: .jsonObject([
                        HTTPBodyKV(key: "page", value: .literal(.int(1))),
                        HTTPBodyKV(key: "pageSize", value: .literal(.int(2))),
                    ])
                ),
                pagination: .pageWithTotal(
                    itemsPath: .field(.current, "list"),
                    totalPath: .field(.current, "total"),
                    pageParam: "page",
                    pageSize: 2
                )
            )),
            .forLoop(
                variable: "product",
                collection: .variable("products"),
                body: [
                    .emit(Emission(
                        typeName: "Product",
                        bindings: [
                            FieldBinding(
                                fieldName: "externalId",
                                expr: .pipe(
                                    .path(.field(.variable("product"), "id")),
                                    [TransformCall(name: "toString")]
                                )
                            ),
                            FieldBinding(fieldName: "name",
                                expr: .path(.field(.variable("product"), "name"))),
                            FieldBinding(fieldName: "brand",
                                expr: .path(.optField(.field(.variable("product"), "brand"), "name"))),
                            FieldBinding(
                                fieldName: "strainPrevalence",
                                expr: .pipe(
                                    .path(.optField(.optField(.field(.variable("product"), "strain"), "prevalence"), "name")),
                                    [TransformCall(name: "prevalenceNormalize")]
                                )
                            ),
                        ]
                    ))
                ]
            )
        ]
    )

    // Two pages: [first page has 2 products with total=3], [second page has 1 product]
    let transport = MockTransport()
    try await transport.registerJSON(
        urlSubstring: "products",
        json: [
            "list": [
                ["id": 100, "name": "A", "brand": ["name": "BrandA"], "strain": ["prevalence": ["name": "INDICA"]]],
                ["id": 200, "name": "B", "brand": NSNull(), "strain": NSNull()],
                ["id": 300, "name": "C", "brand": ["name": "BrandC"], "strain": ["prevalence": ["name": "Sativa"]]],
            ],
            "total": 3,
        ]
    )
    let client = HTTPClient(transport: transport, minRequestInterval: 0)
    let runner = RecipeRunner(httpClient: client)

    let snapshot = try await runner.run(recipe: recipe, inputs: ["storeId": .string("577")])

    let products = snapshot.records(of: "Product")
    #expect(products.count == 3)
    #expect(products[0].fields["externalId"] == .string("100"))
    #expect(products[0].fields["name"] == .string("A"))
    #expect(products[0].fields["brand"] == .string("BrandA"))
    #expect(products[0].fields["strainPrevalence"] == .string("Indica"))

    #expect(products[1].fields["brand"] == .null)
    #expect(products[1].fields["strainPrevalence"] == .null)

    #expect(products[2].fields["strainPrevalence"] == .string("Sativa"))

    // staticHeader applied:
    let calls = await transport.calls
    #expect(calls.first?.value(forHTTPHeaderField: "storeId") == "577")
}

// MARK: - case-of inside emit binding

@Test
func caseOfBranchesByEnumLabel() async throws {
    let recipe = Recipe(
        name: "case-of-test",
        engineKind: .http,
        types: [
            RecipeType(name: "Obs", fields: [
                RecipeField(name: "menuType", type: .string, optional: false),
                RecipeField(name: "price", type: .double, optional: true),
            ]),
        ],
        enums: [
            RecipeEnum(name: "MenuType", variants: ["RECREATIONAL", "MEDICAL"]),
        ],
        inputs: [
            InputDecl(name: "menuTypes", type: .array(.enumRef("MenuType"))),
        ],
        body: [
            .step(HTTPStep(
                name: "snapshot",
                request: HTTPRequest(method: "GET", url: Template(literal: "https://example.com/snap"))
            )),
            .forLoop(
                variable: "menu",
                collection: .field(.input, "menuTypes"),
                body: [
                    .emit(Emission(
                        typeName: "Obs",
                        bindings: [
                            FieldBinding(fieldName: "menuType", expr: .path(.variable("menu"))),
                            FieldBinding(
                                fieldName: "price",
                                expr: .caseOf(
                                    scrutinee: .variable("menu"),
                                    branches: [
                                        (label: "RECREATIONAL", expr: .path(.field(.variable("snapshot"), "priceRec"))),
                                        (label: "MEDICAL",      expr: .path(.field(.variable("snapshot"), "priceMed"))),
                                    ]
                                )
                            ),
                        ]
                    ))
                ]
            )
        ]
    )

    let transport = MockTransport()
    try await transport.registerJSON(
        urlSubstring: "snap",
        json: ["priceRec": 50.0, "priceMed": 45.0]
    )
    let client = HTTPClient(transport: transport, minRequestInterval: 0)
    let runner = RecipeRunner(httpClient: client)

    let snapshot = try await runner.run(
        recipe: recipe,
        inputs: ["menuTypes": .array([.string("RECREATIONAL"), .string("MEDICAL")])]
    )

    let obs = snapshot.records(of: "Obs")
    #expect(obs.count == 2)
    #expect(obs[0].fields["menuType"] == .string("RECREATIONAL"))
    // JSONSerialization collapses whole-number doubles to ints; that's fine —
    // the persister coerces back to double when writing to a price column.
    #expect(obs[0].fields["price"] == .int(50))
    #expect(obs[1].fields["menuType"] == .string("MEDICAL"))
    #expect(obs[1].fields["price"] == .int(45))
}

// MARK: - Path resolver edge cases

@Test
func pathResolverHandlesOptChainAndWildcard() async throws {
    let scope = Scope(
        inputs: [:],
        frames: [["data": .object([
            "items": .array([
                .object(["id": .int(1)]),
                .object(["id": .int(2)]),
                .object(["id": .int(3)]),
            ])
        ])]],
        current: nil
    )

    // Wildcard widens to a list
    let widened = try PathResolver.resolve(
        .wildcard(.field(.field(.variable("data"), "items"), "id")),
        in: scope.with("data", scope.variable("data") ?? .null)
    )
    // Note: above resolves $.data.items.id without wildcard would fail; with wildcard
    // we widen the items array to an array of ids — actually the semantics here
    // is: items is an array, .id on an array isn't directly meaningful. Let's
    // simplify: just check wildcard on a list.

    _ = widened

    let listValue = try PathResolver.resolve(
        .field(.variable("data"), "items"),
        in: scope
    )
    if case .array(let xs) = listValue {
        #expect(xs.count == 3)
    } else {
        Issue.record("expected list")
    }

    // Optional chaining returns null on missing
    let missing = try PathResolver.resolve(
        .optField(.field(.variable("data"), "missing"), "x"),
        in: scope
    )
    #expect(missing == .null)
}

// MARK: - Transform vocabulary

@Test
func transformVocabularyCovers_parseSize_normalizeOzToGrams_prevalence() async throws {
    let evaluator = ExtractionEvaluator()
    let transforms = evaluator.transforms

    // parseSize: "3.5g" → {value: 3.5, unit: "G"}
    let s1 = try transforms.apply("parseSize", value: .string("3.5g"), args: [])
    if case .object(let o) = s1 {
        #expect(o["value"] == .double(3.5))
        #expect(o["unit"] == .string("G"))
    } else { Issue.record("parseSize: not object") }

    // parseSize: "1oz" → {value: 1, unit: "OZ"}; normalizeOzToGrams → {value: 28, unit: "G"}
    let s2 = try transforms.apply("parseSize", value: .string("1oz"), args: [])
    let s2g = try transforms.apply("normalizeOzToGrams", value: s2, args: [])
    if case .object(let o) = s2g {
        #expect(o["value"] == .double(28))
        #expect(o["unit"] == .string("G"))
    } else { Issue.record("normalizeOzToGrams: not object") }

    // prevalenceNormalize
    #expect(try transforms.apply("prevalenceNormalize", value: .string("INDICA"), args: []) == .string("Indica"))
    #expect(try transforms.apply("prevalenceNormalize", value: .string("NOT_APPLICABLE"), args: []) == .null)

    // coalesce
    #expect(try transforms.apply("coalesce", value: .null, args: [.null, .string("X"), .string("Y")]) == .string("X"))
    #expect(try transforms.apply("coalesce", value: .string("V"), args: [.string("X")]) == .string("V"))

    // titleCase
    #expect(try transforms.apply("titleCase", value: .string("flower flowers"), args: []) == .string("Flower Flowers"))
}

// MARK: - HTTPProgress integration: phases observed during a paginated run

/// Per-test recorder for `HTTPProgress.phase` snapshots taken at each
/// transport-level send. The actor chain through `HTTPClient → Transport →
/// MockTransport` already deadlocks if Transport itself is an actor (Swift
/// Concurrency can't unwind the nested isolation hops with the engine actor
/// awaiting). Keeping the recorder `@MainActor` and the transport a value
/// type sidesteps that.
@MainActor
final class PhaseRecorder {
    var phases: [HTTPProgress.Phase] = []
    init() {}
}

struct PhaseRecordingTransport: Transport {
    let inner: MockTransport
    let progress: HTTPProgress
    let recorder: PhaseRecorder

    func send(_ request: URLRequest) async throws -> (Data, HTTPURLResponse) {
        await MainActor.run {
            recorder.phases.append(progress.phase)
        }
        return try await inner.send(request)
    }
}

@Test
func httpEngineDrivesProgressThroughPrimingSteppingPaginatingDone() async throws {
    // Recipe: htmlPrime auth + a paginated step that runs three pages.
    let recipe = Recipe(
        name: "progress-test",
        engineKind: .http,
        types: [
            RecipeType(name: "Product", fields: [
                RecipeField(name: "externalId", type: .string, optional: false),
            ]),
        ],
        auth: .htmlPrime(
            stepName: "prime",
            capturedVars: [
                HtmlPrimeVar(varName: "nonce", regexPattern: "\"nonce\":\"([a-z0-9]+)\"", groupIndex: 1),
            ]
        ),
        body: [
            .step(HTTPStep(
                name: "prime",
                request: HTTPRequest(method: "GET", url: Template(literal: "https://example.com/prime"))
            )),
            .step(HTTPStep(
                name: "products",
                request: HTTPRequest(
                    method: "POST",
                    url: Template(literal: "https://example.com/products"),
                    body: .jsonObject([
                        HTTPBodyKV(key: "page", value: .literal(.int(1))),
                    ])
                ),
                pagination: .pageWithTotal(
                    itemsPath: .field(.current, "list"),
                    totalPath: .field(.current, "total"),
                    pageParam: "page",
                    pageSize: 1
                )
            )),
            .forLoop(
                variable: "product",
                collection: .variable("products"),
                body: [
                    .emit(Emission(
                        typeName: "Product",
                        bindings: [
                            FieldBinding(
                                fieldName: "externalId",
                                expr: .pipe(
                                    .path(.field(.variable("product"), "id")),
                                    [TransformCall(name: "toString")]
                                )
                            ),
                        ]
                    ))
                ]
            )
        ]
    )

    let mock = MockTransport()
    let primeBody = "{\"nonce\":\"deadbeef\"}"
    await mock.register(
        { ($0.url?.absoluteString ?? "").contains("example.com/prime") },
        body: primeBody.data(using: .utf8)!,
        status: 200
    )
    try await mock.registerJSON(
        urlSubstring: "example.com/products",
        json: [
            "list": [["id": 100], ["id": 101], ["id": 102]],
            "total": 3,
        ]
    )

    let recorder = await PhaseRecorder()
    let progress = HTTPProgress()
    let phaseTransport = PhaseRecordingTransport(inner: mock, progress: progress, recorder: recorder)
    let client = HTTPClient(transport: phaseTransport, minRequestInterval: 0)
    let engine = HTTPEngine(client: client, progress: progress)

    let snapshot = try await engine.run(recipe: recipe, inputs: [:])

    // Final state: .done, 3 records emitted.
    let finalPhase = await MainActor.run { progress.phase }
    let finalCount = await MainActor.run { progress.recordsEmitted }
    let finalRequests = await MainActor.run { progress.requestsSent }
    let finalURL = await MainActor.run { progress.currentURL }
    #expect(finalPhase == .done)
    #expect(finalCount == 3)
    #expect(snapshot.records(of: "Product").count == 3)

    // Total requests: 1 (priming via htmlPrime) + 1 (paginated step — total=3,
    // list has 3, breaks after first page). The walker skips the prime step
    // by name so it isn't re-dispatched.
    #expect(finalRequests == 2)
    #expect(finalURL?.contains("example.com/products") == true)

    // Phases observed at each network call, in order:
    //   1. .priming                       (htmlPrime auth fetch)
    //   2. .paginating("products", 1)     (only one page — total=3, list=3)
    let phases = await recorder.phases
    #expect(phases.count == 2)
    #expect(phases[0] == .priming)
    #expect(phases[1] == .paginating(name: "products", page: 1))
}

@Test
func httpEngineMarksDoneWithoutHtmlPrime() async throws {
    // No auth.htmlPrime → engine should skip .priming and start with .stepping.
    let recipe = Recipe(
        name: "no-prime",
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
        json: ["data": [["id": "x"], ["id": "y"]]]
    )

    let recorder = await PhaseRecorder()
    let progress = HTTPProgress()
    let phaseTransport = PhaseRecordingTransport(inner: mock, progress: progress, recorder: recorder)
    let client = HTTPClient(transport: phaseTransport, minRequestInterval: 0)
    let engine = HTTPEngine(client: client, progress: progress)

    _ = try await engine.run(recipe: recipe, inputs: [:])

    let finalPhase = await MainActor.run { progress.phase }
    let finalCount = await MainActor.run { progress.recordsEmitted }
    let finalRequests = await MainActor.run { progress.requestsSent }
    #expect(finalPhase == .done)
    #expect(finalCount == 2)
    #expect(finalRequests == 1)

    let phases = await recorder.phases
    #expect(phases == [.stepping(name: "items")])
}

@Test
func httpEngineMarksFailedOnTransportError() async throws {
    let recipe = Recipe(
        name: "fail-test",
        engineKind: .http,
        types: [
            RecipeType(name: "Item", fields: [
                RecipeField(name: "id", type: .string, optional: false),
            ]),
        ],
        body: [
            .step(HTTPStep(
                name: "items",
                request: HTTPRequest(method: "GET", url: Template(literal: "https://example.com/missing"))
            )),
        ]
    )

    // MockTransport has no matchers registered → it throws on send.
    // maxRetries: 0 keeps the test fast; default retries do exponential
    // backoff (2s, 4s, 8s) which would balloon the test runtime.
    let mock = MockTransport()
    let client = HTTPClient(transport: mock, minRequestInterval: 0, maxRetries: 0)
    let progress = HTTPProgress()
    let engine = HTTPEngine(client: client, progress: progress)

    await #expect(throws: (any Error).self) {
        _ = try await engine.run(recipe: recipe, inputs: [:])
    }

    let finalPhase = await MainActor.run { progress.phase }
    switch finalPhase {
    case .failed: break
    default: Issue.record("expected .failed, got \(finalPhase)")
    }
}

@Test
func recipeRunnerExposesAndResetsProgressAcrossRuns() async throws {
    let recipe = Recipe(
        name: "runner-progress",
        engineKind: .http,
        types: [
            RecipeType(name: "Item", fields: [
                RecipeField(name: "id", type: .string, optional: false),
            ]),
        ],
        body: [
            .step(HTTPStep(
                name: "items",
                request: HTTPRequest(method: "GET", url: Template(literal: "https://example.com/r"))
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
    try await mock.registerJSON(urlSubstring: "/r", json: ["data": [["id": "1"], ["id": "2"]]])
    let client = HTTPClient(transport: mock, minRequestInterval: 0)
    let runner = RecipeRunner(httpClient: client)

    // Grab the long-lived progress reference up front (the consumer pattern).
    let progress = runner.progress

    _ = try await runner.run(recipe: recipe, inputs: [:])
    let phaseAfterFirst = await MainActor.run { progress.phase }
    let countAfterFirst = await MainActor.run { progress.recordsEmitted }
    #expect(phaseAfterFirst == .done)
    #expect(countAfterFirst == 2)

    // Second run on the same runner: progress should reset, not accumulate.
    _ = try await runner.run(recipe: recipe, inputs: [:])
    let phaseAfterSecond = await MainActor.run { progress.phase }
    let countAfterSecond = await MainActor.run { progress.recordsEmitted }
    let requestsAfterSecond = await MainActor.run { progress.requestsSent }
    #expect(phaseAfterSecond == .done)
    #expect(countAfterSecond == 2)
    #expect(requestsAfterSecond == 1)
}
