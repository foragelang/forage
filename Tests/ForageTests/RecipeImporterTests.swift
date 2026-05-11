import Testing
import Foundation
@testable import Forage

// MARK: - Parser support for `import` directives — parallel-safe.

@Test
func parsesSingleImport() throws {
    let src = """
    import sample-recipe

    recipe "x" {
        engine http
        type Y { id: String }
        emit Y { id ← "1" }
    }
    """
    let recipe = try Parser.parse(source: src)
    #expect(recipe.imports.count == 1)
    #expect(recipe.imports[0].raw == "sample-recipe")
    #expect(recipe.imports[0].name == "sample-recipe")
    #expect(recipe.imports[0].namespace == nil)
    #expect(recipe.imports[0].registry == nil)
    #expect(recipe.imports[0].version == nil)
}

@Test
func parsesMultipleImportsWithVersions() throws {
    let src = """
    import sample-recipe
    import alice/awesome-recipe v3

    recipe "x" {
        engine http
        type Y { id: String }
        emit Y { id ← "1" }
    }
    """
    let recipe = try Parser.parse(source: src)
    #expect(recipe.imports.count == 2)
    #expect(recipe.imports[0].raw == "sample-recipe")
    #expect(recipe.imports[0].name == "sample-recipe")
    #expect(recipe.imports[0].version == nil)
    #expect(recipe.imports[1].raw == "alice/awesome-recipe")
    #expect(recipe.imports[1].namespace == "alice")
    #expect(recipe.imports[1].name == "awesome-recipe")
    #expect(recipe.imports[1].version == 3)
}

@Test
func parsesCustomRegistryImports() throws {
    let src = """
    import hub.example.com/team/scraper v1
    import localhost:5000/me/test

    recipe "x" {
        engine http
        type Y { id: String }
        emit Y { id ← "1" }
    }
    """
    let recipe = try Parser.parse(source: src)
    #expect(recipe.imports.count == 2)
    #expect(recipe.imports[0].registry == "hub.example.com")
    #expect(recipe.imports[0].namespace == "team")
    #expect(recipe.imports[0].name == "scraper")
    #expect(recipe.imports[0].version == 1)
    #expect(recipe.imports[1].registry == "localhost:5000")
    #expect(recipe.imports[1].namespace == "me")
    #expect(recipe.imports[1].name == "test")
}

@Test
func rejectsMalformedImportRef() {
    let src = """
    import a/b/c

    recipe "x" { engine http }
    """
    do {
        _ = try Parser.parse(source: src)
        Issue.record("expected parse error for malformed ref a/b/c (registry detection requires `.`/`:`/localhost)")
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
