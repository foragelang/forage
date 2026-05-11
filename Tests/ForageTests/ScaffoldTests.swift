import Testing
import Foundation
@testable import Forage

// MARK: - Fixtures

private func captureJSONL(_ records: [[String: Any]]) -> Data {
    var out = Data()
    for r in records {
        let data = try! JSONSerialization.data(withJSONObject: r, options: [.sortedKeys])
        out.append(data)
        out.append(0x0A)
    }
    return out
}

private func syntheticTwoPages() -> [[String: Any]] {
    [
        [
            "method": "GET",
            "requestUrl": "https://shop.example.com/api/products?page=1",
            "responseUrl": "https://shop.example.com/api/products?page=1",
            "status": 200,
            "contentType": "application/json",
            "body": #"{"products":[{"id":"a1","name":"First","price":9.99,"available":true,"image_urls":["x","y"]},{"id":"a2","name":"Second","price":12.50,"available":false,"image_urls":["z"]},{"id":"a3","name":"Third","price":7.00,"available":true,"image_urls":[]}]}"#,
        ],
        [
            "method": "GET",
            "requestUrl": "https://shop.example.com/api/products?page=2",
            "responseUrl": "https://shop.example.com/api/products?page=2",
            "status": 200,
            "contentType": "application/json",
            "body": #"{"products":[{"id":"a4","name":"Fourth","price":5.00,"available":false,"image_urls":["w","v"]},{"id":"a5","name":"Fifth","price":15.00,"available":true,"image_urls":["u"]}]}"#,
        ],
    ]
}

// MARK: - URL canonicalization

@Test
func canonicalizeStripsQueryAndLowercasesHost() {
    let result = Scaffolder.canonicalize("https://Shop.Example.com/api/Products/42?page=1")
    #expect(result == "https://shop.example.com/api/Products/:id")
}

@Test
func canonicalizeCollapsesNumericAndHexSegments() {
    // `12345` is numeric → :id; `cafebabe1234` is 12 hex chars → :id.
    let result = Scaffolder.canonicalize("https://x.com/api/12345/cafebabe1234/list")
    #expect(result == "https://x.com/api/:id/:id/list")
}

@Test
func canonicalizePreservesShortHexLikeWords() {
    // Real path words can look hex-ish but be too short to be IDs.
    // `face`/`beef`/`dad` are 3-4 hex chars; should stay literal.
    let result = Scaffolder.canonicalize("https://x.com/api/face/beef/list")
    #expect(result == "https://x.com/api/face/beef/list")
}

@Test
func canonicalizePreservesShortAlphaSegments() {
    let result = Scaffolder.canonicalize("https://x.com/api/products/list")
    #expect(result == "https://x.com/api/products/list")
}

// MARK: - Array discovery

@Test
func findBestArrayPicksLargestObjectArray() throws {
    let body = #"{"products":[{"id":1,"name":"a"},{"id":2,"name":"b"},{"id":3,"name":"c"}],"meta":{"counts":{"a":1,"b":2}}}"#
    let json = try JSONSerialization.jsonObject(with: body.data(using: .utf8)!)
    let found = Scaffolder.findBestArray(in: json)
    #expect(found != nil)
    #expect(found?.path == "$.products")
    #expect(found?.array.count == 3)
}

@Test
func findBestArrayPrefersHomogeneousArraysOnTieLength() throws {
    // Two arrays of equal length: one homogeneous (all share keys), one not.
    let body = #"{"a":[{"x":1,"y":2},{"z":9}],"b":[{"id":1,"name":"a"},{"id":2,"name":"b"}]}"#
    let json = try JSONSerialization.jsonObject(with: body.data(using: .utf8)!)
    let found = Scaffolder.findBestArray(in: json)
    #expect(found != nil)
    // `b` has 2 shared keys; `a` has 0. Homogeneity wins.
    #expect(found?.path == "$.b")
}

@Test
func findBestArrayRejectsSingletonsAndMixedArrays() throws {
    let body = #"{"singleton":[{"id":1}],"mixed":[{"id":1},"oops",{"id":2}]}"#
    let json = try JSONSerialization.jsonObject(with: body.data(using: .utf8)!)
    let found = Scaffolder.findBestArray(in: json)
    #expect(found == nil)
}

// MARK: - Type inference

@Test
func inferTypePicksStringForIdName() {
    #expect(Scaffolder.inferType(forKey: "id", values: [123]) == "String")
    #expect(Scaffolder.inferType(forKey: "externalId", values: [123]) == "String")
    #expect(Scaffolder.inferType(forKey: "productId", values: [123]) == "String")
    #expect(Scaffolder.inferType(forKey: "name", values: [123]) == "String")
    #expect(Scaffolder.inferType(forKey: "title", values: [123]) == "String")
    #expect(Scaffolder.inferType(forKey: "description", values: ["x"]) == "String")
    #expect(Scaffolder.inferType(forKey: "slug", values: ["x"]) == "String")
    #expect(Scaffolder.inferType(forKey: "image_url", values: ["x"]) == "String")
}

@Test
func inferTypePicksDoubleForPriceLikeKeys() {
    #expect(Scaffolder.inferType(forKey: "price", values: [9]) == "Double")
    #expect(Scaffolder.inferType(forKey: "salePrice", values: [9]) == "Double")
    #expect(Scaffolder.inferType(forKey: "promo_price", values: [9]) == "Double")
}

@Test
func inferTypePicksBoolForIsAvailableAble() {
    #expect(Scaffolder.inferType(forKey: "available", values: [true]) == "Bool")
    #expect(Scaffolder.inferType(forKey: "is_active", values: [true]) == "Bool")
    #expect(Scaffolder.inferType(forKey: "has_stock", values: [true]) == "Bool")
}

@Test
func inferTypeFallsBackOnValueShape() {
    #expect(Scaffolder.inferType(forKey: "count", values: [1, 2, 3]) == "Int")
    #expect(Scaffolder.inferType(forKey: "rating", values: [4.5]) == "Double")
    #expect(Scaffolder.inferType(forKey: "tags", values: [["a", "b"]]) == "[String]")
    #expect(Scaffolder.inferType(forKey: "active", values: [true, false]) == "Bool")
    #expect(Scaffolder.inferType(forKey: "wat", values: ["a", "b"]) == "String")
}

// MARK: - Item shape

@Test
func buildItemShapeUnionsKeysAndMarksOptional() throws {
    let array: [Any] = [
        ["id": "a", "name": "X", "price": 1.0],
        ["id": "b", "name": "Y", "price": 2.0, "extra": "z"],
    ]
    let shape = Scaffolder.buildItemShape(from: array)
    let fields = Dictionary(uniqueKeysWithValues: shape.fields.map { ($0.name, $0.type) })
    #expect(fields["id"] == "String")
    #expect(fields["name"] == "String")
    #expect(fields["price"] == "Double")
    // `extra` only on second element → optional.
    #expect(fields["extra"] == "String?")
}

@Test
func buildItemShapeSkipsNestedObjects() throws {
    let array: [Any] = [
        ["id": "a", "brand": ["name": "B1"]],
        ["id": "b", "brand": ["name": "B2"]],
    ]
    let shape = Scaffolder.buildItemShape(from: array)
    #expect(shape.fields.map(\.name) == ["id"])
    #expect(shape.skippedNestedKeys == ["brand"])
}

// MARK: - End-to-end scaffold

@Test
func scaffoldFromSyntheticCapturesProducesValidRecipe() throws {
    let data = captureJSONL(syntheticTwoPages())
    let captures = try Scaffolder.parseJSONL(data)
    #expect(captures.count == 2)

    let recipeSource = Scaffolder.scaffold(captures: captures, hostFilter: nil)

    // The recipe must round-trip through parser + validator without errors.
    let recipe = try Parser.parse(source: recipeSource)
    let issues = Validator.validate(recipe)
    #expect(!issues.hasErrors, "validation errors: \(issues.errors)")

    // Engine choice + shape sanity.
    #expect(recipe.engineKind == .http)
    let product = try #require(recipe.type("Product"))
    let fieldNames = Set(product.fields.map(\.name))
    #expect(fieldNames == ["id", "name", "price", "available", "image_urls"])

    // Standard inputs.
    #expect(recipe.inputs.map(\.name) == ["dispensarySlug", "dispensaryName", "siteOrigin"])

    // One expect clause for the inferred type.
    #expect(recipe.expectations.count == 1)
}

@Test
func scaffoldHostFilterNarrowsCandidateSet() throws {
    let multi: [[String: Any]] = syntheticTwoPages() + [
        [
            "method": "GET",
            "requestUrl": "https://other-host.com/api/list",
            "responseUrl": "https://other-host.com/api/list",
            "status": 200,
            "contentType": "application/json",
            // Bigger array — would win without --host filter.
            "body": #"{"items":[{"sku":"s1","name":"X","price":1.0},{"sku":"s2","name":"Y","price":2.0},{"sku":"s3","name":"Z","price":3.0},{"sku":"s4","name":"W","price":4.0},{"sku":"s5","name":"V","price":5.0},{"sku":"s6","name":"U","price":6.0},{"sku":"s7","name":"T","price":7.0},{"sku":"s8","name":"S","price":8.0}]}"#,
        ],
    ]
    let data = captureJSONL(multi)
    let captures = try Scaffolder.parseJSONL(data)
    let recipeSource = Scaffolder.scaffold(captures: captures, hostFilter: "shop.example.com")
    let recipe = try Parser.parse(source: recipeSource)
    let issues = Validator.validate(recipe)
    #expect(!issues.hasErrors)

    // The Product type has shape from shop.example.com, not the bigger array.
    let product = try #require(recipe.type("Product"))
    let fieldNames = Set(product.fields.map(\.name))
    #expect(fieldNames == ["id", "name", "price", "available", "image_urls"])
}

@Test
func scaffoldFallsBackToEmptyStubWhenNoJSONFound() throws {
    let html: [[String: Any]] = [[
        "method": "GET",
        "requestUrl": "https://example.com/",
        "responseUrl": "https://example.com/",
        "status": 200,
        "contentType": "text/html",
        "body": "<html><body>nothing here</body></html>",
    ]]
    let data = captureJSONL(html)
    let captures = try Scaffolder.parseJSONL(data)
    let recipeSource = Scaffolder.scaffold(captures: captures, hostFilter: nil)
    let recipe = try Parser.parse(source: recipeSource)
    let issues = Validator.validate(recipe)
    #expect(!issues.hasErrors)
    #expect(recipe.engineKind == .browser)
}
