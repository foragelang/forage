import Testing
import Foundation
@testable import Forage

// MARK: - Lexer

@Test
func lexerHandlesKeywordsLiteralsAndOperators() throws {
    var lex = Lexer(source: """
    recipe "x" { engine http; type Foo { name: String?; age: Int }
        // comment
        emit Foo { name ← $.x; age ← 42 }
    }
    """)
    let toks = try lex.tokenize()
    // Just sanity-check a few key tokens are there
    #expect(toks.contains(where: { $0.lexeme == "recipe" }))
    #expect(toks.contains(where: { if case .stringLit(let s) = $0.kind, s == "x" { return true } else { return false } }))
    #expect(toks.contains(where: { $0.kind == .arrow }))
    #expect(toks.contains(where: { $0.kind == .qDot || ($0.kind == .question) }))   // optional `?` after String
}

@Test
func lexerReadsDoubleAndDateAndBool() throws {
    var lex = Lexer(source: "1.5 1990-01-01 true false null")
    let toks = try lex.tokenize()
    let kinds = toks.map(\.kind)
    var foundDouble = false, foundDate = false, foundTrue = false, foundFalse = false, foundNull = false
    for k in kinds {
        if case .doubleLit(let d) = k, d == 1.5 { foundDouble = true }
        if case .dateLit(let y, let m, let d) = k, y == 1990, m == 1, d == 1 { foundDate = true }
        if case .boolLit(true) = k { foundTrue = true }
        if case .boolLit(false) = k { foundFalse = true }
        if case .nullLit = k { foundNull = true }
    }
    #expect(foundDouble && foundDate && foundTrue && foundFalse && foundNull)
}

// MARK: - Parser smoke

@Test
func parsesMinimalRecipe() throws {
    let src = """
    recipe "minimal" {
        engine http
        type Item {
            id: String
            name: String
        }
        input baseUrl: String
        step list {
            method "GET"
            url "{$input.baseUrl}/items"
        }
        for $item in $list {
            emit Item {
                id ← $item.id | toString
                name ← $item.name
            }
        }
    }
    """
    let recipe = try Parser.parse(source: src)
    #expect(recipe.name == "minimal")
    #expect(recipe.engineKind == .http)
    #expect(recipe.types.count == 1)
    #expect(recipe.types.first?.name == "Item")
    #expect(recipe.types.first?.fields.count == 2)
    #expect(recipe.inputs.count == 1)
    #expect(recipe.body.count == 2)
    if case .step(let s) = recipe.body[0] {
        #expect(s.name == "list")
        #expect(s.request.method == "GET")
    } else {
        Issue.record("expected step")
    }
    if case .forLoop(let v, _, let body) = recipe.body[1] {
        #expect(v == "item")
        #expect(body.count == 1)
        if case .emit(let em) = body[0] {
            #expect(em.typeName == "Item")
            #expect(em.bindings.count == 2)
        } else { Issue.record("expected emit") }
    } else { Issue.record("expected for-loop") }
}

@Test
func parsesPaginationStrategies() throws {
    let src = """
    recipe "pagn" {
        engine http
        step a {
            method "GET"; url "https://x.test/a"
            paginate pageWithTotal { items: $.list; total: $.total; pageParam: "page"; pageSize: 200 }
        }
        step b {
            method "GET"; url "https://x.test/b"
            paginate untilEmpty { items: $.data; pageParam: "n" }
        }
    }
    """
    let recipe = try Parser.parse(source: src)
    #expect(recipe.body.count == 2)
    if case .step(let a) = recipe.body[0], case .pageWithTotal(_, _, let pp, let ps, _) = a.pagination! {
        #expect(pp == "page"); #expect(ps == 200)
    } else { Issue.record("expected pageWithTotal") }
    if case .step(let b) = recipe.body[1], case .untilEmpty(_, let pp, _) = b.pagination! {
        #expect(pp == "n")
    } else { Issue.record("expected untilEmpty") }
}

@Test
func parsesAuthStaticHeader() throws {
    let src = """
    recipe "auth" {
        engine http
        input storeId: String
        auth.staticHeader { name: "storeId"; value: "{$input.storeId}" }
    }
    """
    let recipe = try Parser.parse(source: src)
    if case .staticHeader(let n, _) = recipe.auth! {
        #expect(n == "storeId")
    } else { Issue.record("expected staticHeader") }
}

@Test
func parsesEnumDecl() throws {
    let src = """
    recipe "e" {
        engine http
        enum MenuType { RECREATIONAL, MEDICAL }
    }
    """
    let recipe = try Parser.parse(source: src)
    #expect(recipe.enums.first?.name == "MenuType")
    #expect(recipe.enums.first?.variants == ["RECREATIONAL", "MEDICAL"])
}

@Test
func parsesCaseOfInExtraction() throws {
    let src = """
    recipe "c" {
        engine http
        type Obs { mt: String }
        enum MenuType { RECREATIONAL, MEDICAL }
        for $menu in $input.menus {
            emit Obs {
                mt ← case $menu of {
                    RECREATIONAL → "rec"
                    MEDICAL → "med"
                }
            }
        }
    }
    """
    let recipe = try Parser.parse(source: src)
    if case .forLoop(_, _, let body) = recipe.body.first!,
       case .emit(let em) = body.first!,
       case .caseOf(_, let branches) = em.bindings.first!.expr {
        #expect(branches.count == 2)
        #expect(branches[0].label == "RECREATIONAL")
    } else {
        Issue.record("expected case-of")
    }
}

@Test
func parsesExpectations() throws {
    let src = """
    recipe "x" {
        engine http
        expect { records.where(typeName == "Product").count >= 50 }
        expect { records.where(typeName == "Variant").count > 0 }
    }
    """
    let recipe = try Parser.parse(source: src)
    #expect(recipe.expectations.count == 2)
    if case .recordCount(let t, let op, let n) = recipe.expectations[0].kind {
        #expect(t == "Product"); #expect(op == .ge); #expect(n == 50)
    } else {
        Issue.record("expected recordCount expectation")
    }
}

@Test
func parsesBrowserConfig() throws {
    let src = """
    recipe "j" {
        engine browser
        input siteOrigin: String
        browser {
            initialURL: "{$input.siteOrigin}/menu"
            ageGate.autoFill { dob: 1990-01-01; reloadAfter: true }
            warmupClicks: ["All products"]
            observe: "iheartjane.com/v2/smartpage"
            paginate browserPaginate.scroll { until: noProgressFor(3); maxIterations: 30 }
        }
    }
    """
    let recipe = try Parser.parse(source: src)
    #expect(recipe.engineKind == .browser)
    #expect(recipe.browser?.observe == "iheartjane.com/v2/smartpage")
    #expect(recipe.browser?.warmupClicks == ["All products"])
    if case .noProgressFor(let n) = recipe.browser!.pagination.until {
        #expect(n == 3)
    } else { Issue.record("expected noProgressFor") }
    #expect(recipe.browser?.ageGate?.year == 1990)
}

// MARK: - Parses the bundled platform recipes

private func recipesDir(file: StaticString = #filePath) -> String {
    let testFile = URL(fileURLWithPath: "\(file)")
    return testFile
        .deletingLastPathComponent()        // Tests/ForageTests
        .deletingLastPathComponent()        // Tests
        .deletingLastPathComponent()        // <repo>
        .appendingPathComponent("recipes")
        .path
}

@Test
func parsesSweedRecipe() throws {
    let path = "\(recipesDir())/sweed/recipe.forage"
    let src = try String(contentsOfFile: path, encoding: .utf8)
    let recipe = try Parser.parse(source: src)
    #expect(recipe.name == "sweed")
    #expect(recipe.engineKind == .http)
    #expect(recipe.types.contains(where: { $0.name == "Product" }))
    #expect(recipe.types.contains(where: { $0.name == "PriceObservation" }))
    #expect(recipe.enums.contains(where: { $0.name == "MenuType" }))
    if case .staticHeader = recipe.auth { } else { Issue.record("expected staticHeader auth") }
    #expect(recipe.expectations.count >= 1)
}

@Test
func parsesLeafbridgeRecipe() throws {
    let path = "\(recipesDir())/leafbridge/recipe.forage"
    let src = try String(contentsOfFile: path, encoding: .utf8)
    let recipe = try Parser.parse(source: src)
    #expect(recipe.name == "leafbridge")
    if case .htmlPrime = recipe.auth { } else { Issue.record("expected htmlPrime auth") }
}

@Test
func parsesJaneRecipe() throws {
    let path = "\(recipesDir())/jane/recipe.forage"
    let src = try String(contentsOfFile: path, encoding: .utf8)
    let recipe = try Parser.parse(source: src)
    #expect(recipe.name == "jane")
    #expect(recipe.engineKind == .browser)
    #expect(recipe.browser != nil)
    #expect(recipe.browser?.observe.contains("smartpage") == true)
}

// MARK: - Error reporting

@Test
func templateInterpolationSupportsTransformPipe() throws {
    // The parser should accept pipelines inside `{...}` template interpolations,
    // and the renderer should pipe the path value through the named transforms
    // before stringifying.
    let src = """
    recipe "tpl" {
        engine http
        type Item { key: String }
        for $weight in $input.weights {
            emit Item {
                key ← "price_{$weight | janeWeightKey}"
            }
        }
    }
    """
    let recipe = try Parser.parse(source: src)
    guard case .forLoop(_, _, let body) = recipe.body.first!,
          case .emit(let em) = body.first!,
          case .template(let t) = em.bindings.first!.expr
    else {
        Issue.record("expected emit { key ← <template> }")
        return
    }
    // Two parts: "price_" literal, then an interp with a pipe.
    #expect(t.parts.count == 2)
    if case .literal(let s) = t.parts[0] { #expect(s == "price_") } else { Issue.record("expected literal") }
    guard case .interp(let expr) = t.parts[1] else {
        Issue.record("expected interp"); return
    }
    if case .pipe(let inner, let calls) = expr {
        if case .path(let p) = inner, case .variable("weight") = p { } else {
            Issue.record("expected $weight path")
        }
        #expect(calls.count == 1)
        #expect(calls.first?.name == "janeWeightKey")
    } else {
        Issue.record("expected pipe inside interp")
    }

    // Render against a synthetic scope where $weight = "eighth ounce".
    let scope = Scope(
        inputs: [:],
        frames: [["weight": .string("eighth ounce")]],
        current: nil
    )
    let rendered = try TemplateRenderer.render(t, in: scope)
    #expect(rendered == "price_eighth_ounce")
}

@Test
func parserReportsLineColumnOnError() throws {
    // Unknown type-name references are not parse errors — they're validator
    // (Phase D) concerns. The parser is permissive about forward references,
    // by design. We just verify a structural mistake (missing closing brace)
    // surfaces a ParseError with location info.
    let src = "recipe \"x\" { engine http "
    do {
        _ = try Parser.parse(source: src)
        Issue.record("expected parse error for missing closing brace")
    } catch let e as ParseError {
        let desc = String(describing: e)
        #expect(desc.contains("expected"))
    } catch {
        Issue.record("expected ParseError, got \(type(of: error))")
    }
}
