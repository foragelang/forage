import Testing
import Foundation
@testable import Forage

// Drift-detection: both the Swift runtime and the TypeScript port read the
// same `.forage` files in `tests/shared-recipes/` and assert against the same
// `expected.json` summary. If a parser/validator change in one implementation
// drifts from the other, one of these tests fails first.

private struct ExpectedFile: Decodable {
    let description: String
    let recipes: [ExpectedRecipe]
}

private struct ExpectedRecipe: Decodable {
    let file: String
    let parses: Bool
    let summary: Summary?
    let types: [ExpectedType]?
    let enums: [ExpectedEnum]?
    let imports: [ExpectedImport]?
    let paginationModes: [String]?
    let secrets: [String]?
    let authSessionVariant: String?
    let validation: ExpectedValidation
}

private struct Summary: Decodable {
    let name: String
    let engineKind: String
    let typeCount: Int
    let enumCount: Int
    let inputCount: Int
    let bodyStatementCount: Int
    let stepNames: [String]
    let topLevelEmits: Int
    let forLoopCount: Int
    let expectationCount: Int
    let importCount: Int
}

private struct ExpectedType: Decodable {
    let name: String
    let fieldNames: [String]
    let requiredFieldCount: Int
}

private struct ExpectedEnum: Decodable {
    let name: String
    let variants: [String]
}

private struct ExpectedImport: Decodable {
    let raw: String
    let registry: String?
    let namespace: String?
    let name: String
    let version: Int?
}

private struct ExpectedValidation: Decodable {
    let errorCount: Int?
    let warningCount: Int?
    let errorCountMin: Int?
    let expectedErrorKeywords: [String]?
}

private func sharedRecipesDir() -> URL {
    // `#filePath` is …/Tests/ForageTests/SharedRecipesTests.swift; walk up to
    // the repo root, then down into `tests/shared-recipes`.
    let file = URL(fileURLWithPath: #filePath)
    return file
        .deletingLastPathComponent() // ForageTests
        .deletingLastPathComponent() // Tests
        .deletingLastPathComponent() // repo root
        .appendingPathComponent("tests", isDirectory: true)
        .appendingPathComponent("shared-recipes", isDirectory: true)
}

private func loadExpected() throws -> ExpectedFile {
    let url = sharedRecipesDir().appendingPathComponent("expected.json")
    let data = try Data(contentsOf: url)
    return try JSONDecoder().decode(ExpectedFile.self, from: data)
}

@Test
func sharedRecipesParseAndValidateConsistently() throws {
    let expected = try loadExpected()
    for rec in expected.recipes {
        let path = sharedRecipesDir().appendingPathComponent(rec.file)
        let source = try String(contentsOf: path, encoding: .utf8)

        // Parse
        let recipe: Recipe
        do {
            recipe = try Parser.parse(source: source)
        } catch {
            if rec.parses {
                Issue.record("expected \(rec.file) to parse but got: \(error)")
                continue
            }
            // Negative parse case (none currently in vectors): nothing to check
            continue
        }

        if let s = rec.summary {
            #expect(recipe.name == s.name, "\(rec.file): recipe name")
            #expect(recipe.engineKind.rawValue == s.engineKind, "\(rec.file): engineKind")
            #expect(recipe.types.count == s.typeCount, "\(rec.file): typeCount")
            #expect(recipe.enums.count == s.enumCount, "\(rec.file): enumCount")
            #expect(recipe.inputs.count == s.inputCount, "\(rec.file): inputCount")
            #expect(recipe.body.count == s.bodyStatementCount, "\(rec.file): bodyStatementCount")
            #expect(recipe.expectations.count == s.expectationCount, "\(rec.file): expectationCount")
            #expect(recipe.imports.count == s.importCount, "\(rec.file): importCount")
            let stepNames = collectStepNames(recipe.body)
            #expect(stepNames == s.stepNames, "\(rec.file): stepNames \(stepNames) != \(s.stepNames)")
            #expect(countTopLevelEmits(recipe.body) == s.topLevelEmits, "\(rec.file): topLevelEmits")
            #expect(countForLoops(recipe.body) == s.forLoopCount, "\(rec.file): forLoopCount")
        }

        if let types = rec.types {
            for et in types {
                guard let t = recipe.types.first(where: { $0.name == et.name }) else {
                    Issue.record("\(rec.file): missing type \(et.name)")
                    continue
                }
                let names = t.fields.map(\.name)
                #expect(names == et.fieldNames, "\(rec.file): \(et.name) fields \(names) != \(et.fieldNames)")
                let required = t.fields.filter { !$0.optional }.count
                #expect(required == et.requiredFieldCount, "\(rec.file): \(et.name) requiredFieldCount")
            }
        }

        if let enums = rec.enums {
            for ee in enums {
                guard let e = recipe.enums.first(where: { $0.name == ee.name }) else {
                    Issue.record("\(rec.file): missing enum \(ee.name)")
                    continue
                }
                #expect(e.variants == ee.variants, "\(rec.file): \(ee.name) variants")
            }
        }

        if let imports = rec.imports {
            #expect(recipe.imports.count == imports.count, "\(rec.file): import count")
            for (expected, actual) in zip(imports, recipe.imports) {
                #expect(actual.raw == expected.raw, "\(rec.file): import raw")
                #expect(actual.registry == expected.registry, "\(rec.file): import registry")
                #expect(actual.namespace == expected.namespace, "\(rec.file): import namespace")
                #expect(actual.name == expected.name, "\(rec.file): import name")
                #expect(actual.version == expected.version, "\(rec.file): import version")
            }
        }

        if let modes = rec.paginationModes {
            let actualModes = collectPaginationModes(recipe.body)
            #expect(actualModes == modes, "\(rec.file): paginationModes \(actualModes) != \(modes)")
        }

        if let secrets = rec.secrets {
            #expect(recipe.secrets == secrets, "\(rec.file): secrets \(recipe.secrets) != \(secrets)")
        }

        if let variant = rec.authSessionVariant {
            guard case .session(let s) = recipe.auth else {
                Issue.record("\(rec.file): expected auth.session.\(variant), got \(String(describing: recipe.auth))")
                continue
            }
            let actualVariant: String
            switch s.kind {
            case .formLogin: actualVariant = "formLogin"
            case .bearerLogin: actualVariant = "bearerLogin"
            case .cookiePersist: actualVariant = "cookiePersist"
            }
            #expect(actualVariant == variant, "\(rec.file): authSessionVariant \(actualVariant) != \(variant)")
        }

        // Validation
        let issues = Validator.validate(recipe)
        let errors = issues.filter { $0.severity == .error }
        let warnings = issues.filter { $0.severity == .warning }
        if let n = rec.validation.errorCount {
            #expect(errors.count == n, "\(rec.file): errorCount \(errors.count) != \(n)\n  errors: \(errors.map(\.message))")
        }
        if let n = rec.validation.warningCount {
            #expect(warnings.count == n, "\(rec.file): warningCount \(warnings.count) != \(n)")
        }
        if let n = rec.validation.errorCountMin {
            #expect(errors.count >= n, "\(rec.file): errorCount \(errors.count) < min \(n)")
        }
        if let keywords = rec.validation.expectedErrorKeywords {
            for kw in keywords {
                #expect(errors.contains(where: { $0.message.contains(kw) }),
                       "\(rec.file): expected an error containing '\(kw)'; got \(errors.map(\.message))")
            }
        }
    }
}

// MARK: - Helpers

private func collectStepNames(_ stmts: [Statement]) -> [String] {
    var names: [String] = []
    for s in stmts {
        switch s {
        case .step(let st): names.append(st.name)
        case .forLoop(_, _, let body): names.append(contentsOf: collectStepNames(body))
        case .emit: break
        }
    }
    return names
}

private func countTopLevelEmits(_ stmts: [Statement]) -> Int {
    var n = 0
    for s in stmts {
        if case .emit = s { n += 1 }
    }
    return n
}

private func countForLoops(_ stmts: [Statement]) -> Int {
    var n = 0
    for s in stmts {
        switch s {
        case .forLoop(_, _, let body): n += 1 + countForLoops(body)
        case .step: break
        case .emit: break
        }
    }
    return n
}

private func collectPaginationModes(_ stmts: [Statement]) -> [String] {
    var modes: [String] = []
    for s in stmts {
        switch s {
        case .step(let st):
            if let p = st.pagination {
                switch p {
                case .pageWithTotal: modes.append("pageWithTotal")
                case .untilEmpty: modes.append("untilEmpty")
                case .cursor: modes.append("cursor")
                }
            }
        case .forLoop(_, _, let body):
            modes.append(contentsOf: collectPaginationModes(body))
        case .emit: break
        }
    }
    return modes
}
