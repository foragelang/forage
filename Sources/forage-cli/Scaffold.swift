import ArgumentParser
import Foundation
import Forage

struct ScaffoldCommand: AsyncParsableCommand {
    static let configuration = CommandConfiguration(
        commandName: "scaffold",
        abstract: "Build a starter .forage recipe from a captures JSONL file."
    )

    @Argument(help: "Path to a captures JSONL produced by `forage capture`.")
    var capturesFile: String

    @Option(name: .customLong("host"),
            help: "Substring filter — only captures whose host contains this string are considered.")
    var host: String?

    @Option(name: .customLong("out"), help: "Write recipe to this path instead of stdout.")
    var out: String?

    func run() async throws {
        let url = URL(fileURLWithPath: capturesFile)
        let data: Data
        do {
            data = try Data(contentsOf: url)
        } catch {
            FileHandle.standardError.write("read failed: \(error)\n".data(using: .utf8)!)
            throw ExitCode.failure
        }
        let captures = try Scaffolder.parseJSONL(data)
        let recipe = Scaffolder.scaffold(captures: captures, hostFilter: host)

        if let out {
            try recipe.write(toFile: out, atomically: true, encoding: .utf8)
        } else {
            print(recipe)
        }
    }
}
