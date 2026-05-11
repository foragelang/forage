import ArgumentParser
import Foundation
import Forage

struct TestCommand: AsyncParsableCommand {
    static let configuration = CommandConfiguration(
        commandName: "test",
        abstract: "Run a recipe against fixtures and diff against an expected snapshot."
    )

    @Argument(help: "Recipe directory containing recipe.forage, fixtures/, and optional expected.snapshot.json.")
    var recipeDir: String

    @Flag(name: .customLong("update"),
          help: "Write the produced snapshot to expected.snapshot.json (golden-file workflow).")
    var update: Bool = false

    func run() async throws {
        let outcome = await TestHarness.run(recipeDir: recipeDir, update: update)
        if !outcome.stdout.isEmpty {
            FileHandle.standardOutput.write((outcome.stdout + "\n").data(using: .utf8)!)
        }
        if !outcome.stderr.isEmpty {
            FileHandle.standardError.write((outcome.stderr + "\n").data(using: .utf8)!)
        }
        switch outcome.exit {
        case .ok: return
        case .diff: throw ExitCode(1)
        case .setupError: throw ExitCode(2)
        }
    }
}
