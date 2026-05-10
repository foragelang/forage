import Foundation

/// Top-level entry point. Picks the right engine for a recipe and runs it.
/// Returns a `Snapshot` (records only) plus the basic run outcome.
///
/// `BrowserEngine`-driven recipes will plug in here in Phase E.
public actor RecipeRunner {
    public let httpClient: HTTPClient
    public let evaluator: ExtractionEvaluator

    public init(
        httpClient: HTTPClient,
        evaluator: ExtractionEvaluator = ExtractionEvaluator()
    ) {
        self.httpClient = httpClient
        self.evaluator = evaluator
    }

    public func run(recipe: Recipe, inputs: [String: JSONValue]) async throws -> Snapshot {
        switch recipe.engineKind {
        case .http:
            let engine = HTTPEngine(client: httpClient, evaluator: evaluator)
            return try await engine.run(recipe: recipe, inputs: inputs)
        case .browser:
            // Browser engine lands in Phase E; until then, this is the surface
            // we throw from rather than a half-implemented stub.
            throw RecipeRunnerError.browserEngineNotYetImplemented(recipeName: recipe.name)
        }
    }
}

public enum RecipeRunnerError: Error, CustomStringConvertible {
    case browserEngineNotYetImplemented(recipeName: String)

    public var description: String {
        switch self {
        case .browserEngineNotYetImplemented(let n):
            return "browser engine not yet implemented (recipe: \(n))"
        }
    }
}
