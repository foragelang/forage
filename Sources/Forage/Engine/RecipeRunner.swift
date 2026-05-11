import Foundation

/// Top-level entry point. Picks the right engine for a recipe and runs it.
/// Returns a `RunResult` carrying the snapshot plus a `DiagnosticReport`
/// explaining how the run terminated.
///
/// `progress` is a long-lived `HTTPProgress` instance the runner reuses
/// across runs (reset at the start of each `run(...)`). Consumers grab the
/// reference once and observe it for the lifetime of the runner; SwiftUI
/// tracking on `@Observable` does the rest.
public actor RecipeRunner {
    public let httpClient: HTTPClient
    public let evaluator: ExtractionEvaluator

    public nonisolated let progress: HTTPProgress

    public init(
        httpClient: HTTPClient,
        evaluator: ExtractionEvaluator = ExtractionEvaluator()
    ) {
        self.httpClient = httpClient
        self.evaluator = evaluator
        self.progress = HTTPProgress()
    }

    public func run(recipe: Recipe, inputs: [String: JSONValue]) async throws -> RunResult {
        switch recipe.engineKind {
        case .http:
            await MainActor.run { progress.reset() }
            let engine = HTTPEngine(client: httpClient, evaluator: evaluator, progress: progress)
            return await engine.run(recipe: recipe, inputs: inputs)
        case .browser:
            // BrowserEngine requires NSApplication + main-actor isolation,
            // so the runner can't drive it directly from this actor. Hosts
            // (forage-probe CLI, in-app BrowserProbe, future production
            // browser runner) construct a BrowserEngine and call run() on
            // the main actor themselves. The runner refuses here so the
            // dispatch path is explicit.
            throw RecipeRunnerError.browserEngineRequiresMainActorHost(recipeName: recipe.name)
        }
    }
}

public enum RecipeRunnerError: Error, CustomStringConvertible {
    case browserEngineRequiresMainActorHost(recipeName: String)

    public var description: String {
        switch self {
        case .browserEngineRequiresMainActorHost(let n):
            return "browser engine recipe '\(n)' must be run from a MainActor host that owns NSApplication (use BrowserEngine directly, not RecipeRunner.run)"
        }
    }
}
