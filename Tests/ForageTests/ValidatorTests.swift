import Testing
import Foundation
@testable import Forage

@Test
func validatorAcceptsCleanRecipe() throws {
    let recipe = try Parser.parse(source: """
    recipe "ok" {
        engine http
        type Item { id: String; name: String? }
        input baseUrl: String
        step list { method "GET"; url "{$input.baseUrl}/items" }
        for $i in $list {
            emit Item { id ← $i.id | toString; name ← $i.name }
        }
    }
    """)
    let issues = Validator.validate(recipe)
    #expect(!issues.hasErrors, "expected no errors, got \(issues)")
}

@Test
func validatorCatchesUnknownType() throws {
    let recipe = try Parser.parse(source: """
    recipe "bad" {
        engine http
        type Item { id: String }
        emit NotAType { id ← "x" }
    }
    """)
    let issues = Validator.validate(recipe)
    #expect(issues.hasErrors)
    #expect(issues.errors.contains(where: { $0.message.contains("NotAType") }))
}

@Test
func validatorCatchesUnknownField() throws {
    let recipe = try Parser.parse(source: """
    recipe "bad" {
        engine http
        type Item { id: String }
        emit Item { id ← "x"; bogus ← "y" }
    }
    """)
    let issues = Validator.validate(recipe)
    #expect(issues.hasErrors)
    #expect(issues.errors.contains(where: { $0.message.contains("bogus") }))
}

@Test
func validatorCatchesUnknownTransform() throws {
    let recipe = try Parser.parse(source: """
    recipe "bad" {
        engine http
        type Item { id: String }
        emit Item { id ← "x" | thisDoesNotExist }
    }
    """)
    let issues = Validator.validate(recipe)
    #expect(issues.hasErrors)
    #expect(issues.errors.contains(where: { $0.message.contains("thisDoesNotExist") }))
}

@Test
func validatorWarnsAboutUnboundRequiredField() throws {
    let recipe = try Parser.parse(source: """
    recipe "bad" {
        engine http
        type Item { id: String; name: String }
        emit Item { id ← "x" }
    }
    """)
    let issues = Validator.validate(recipe)
    #expect(issues.warnings.contains(where: { $0.message.contains("name") }))
}

@Test
func validatorAcceptsExampleRecipes() throws {
    for path in [
        "/Users/dima/dev/forage-runtime/recipes/examples/sweed-zen-leaf.forage",
        "/Users/dima/dev/forage-runtime/recipes/examples/leafbridge-remedy.forage",
        "/Users/dima/dev/forage-runtime/recipes/examples/jane-trilogy.forage",
    ] {
        let src = try String(contentsOfFile: path, encoding: .utf8)
        let recipe = try Parser.parse(source: src)
        let issues = Validator.validate(recipe)
        if issues.hasErrors {
            Issue.record("\(path): \(issues.errors)")
        }
    }
}

@Test
func snapshotRoundTripsThroughCodable() throws {
    let snapshot = Snapshot(
        records: [
            ScrapedRecord(typeName: "Product", fields: [
                "name": .string("X"),
                "price": .double(42.5),
                "available": .bool(true),
                "tags": .array([.string("a"), .string("b")]),
            ]),
            ScrapedRecord(typeName: "Variant", fields: [
                "size": .double(3.5),
            ]),
        ],
        observedAt: Date(timeIntervalSince1970: 1_700_000_000)
    )
    let data = try SnapshotIO.encode(snapshot)
    let decoded = try SnapshotIO.decode(data)
    #expect(decoded.records.count == 2)
    #expect(decoded.records[0].typeName == "Product")
    #expect(decoded.records[0].fields["name"] == .string("X"))
    #expect(decoded.records[0].fields["tags"] == .array([.string("a"), .string("b")]))
    #expect(decoded.observedAt == snapshot.observedAt)
}

@Test
func httpReplayerServesByMatcher() async throws {
    let replayer = HTTPReplayer()
    await replayer.add(.init(
        method: "GET",
        urlSubstring: "/items",
        responseBody: #"{"data":[{"id":"1"}]}"#.data(using: .utf8)!
    ))
    var req = URLRequest(url: URL(string: "https://x.test/items?q=1")!)
    req.httpMethod = "GET"
    let (data, response) = try await replayer.send(req)
    #expect(response.statusCode == 200)
    let s = String(data: data, encoding: .utf8) ?? ""
    #expect(s.contains("\"id\":\"1\""))
}
