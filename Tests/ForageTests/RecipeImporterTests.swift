import Testing
import Foundation
@testable import Forage

// MARK: - Parser support for `import` directives — parallel-safe.

@Test
func parsesSingleImport() throws {
    let src = """
    import hub://sample-recipe

    recipe "x" {
        engine http
        type Y { id: String }
        emit Y { id ← "1" }
    }
    """
    let recipe = try Parser.parse(source: src)
    #expect(recipe.imports.count == 1)
    #expect(recipe.imports[0].slug == "sample-recipe")
    #expect(recipe.imports[0].version == nil)
}

@Test
func parsesMultipleImportsWithVersions() throws {
    let src = """
    import hub://sample-recipe
    import hub://alice/awesome-recipe v3

    recipe "x" {
        engine http
        type Y { id: String }
        emit Y { id ← "1" }
    }
    """
    let recipe = try Parser.parse(source: src)
    #expect(recipe.imports.count == 2)
    #expect(recipe.imports[0].slug == "sample-recipe")
    #expect(recipe.imports[0].version == nil)
    #expect(recipe.imports[1].slug == "alice/awesome-recipe")
    #expect(recipe.imports[1].version == 3)
}

@Test
func rejectsMalformedHubURL() {
    let src = """
    import hub://a/b/c

    recipe "x" { engine http }
    """
    do {
        _ = try Parser.parse(source: src)
        Issue.record("expected parse error for malformed hub://")
    } catch {
        // expected
    }
}

@Test
func recipeWithoutImportsHasEmptyImportsArray() throws {
    let src = """
    recipe "x" {
        engine http
        type Y { id: String }
        emit Y { id ← "1" }
    }
    """
    let recipe = try Parser.parse(source: src)
    #expect(recipe.imports.isEmpty)
}
