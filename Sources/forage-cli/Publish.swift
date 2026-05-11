import ArgumentParser
import Foundation
import Forage

struct PublishCommand: AsyncParsableCommand {
    static let configuration = CommandConfiguration(
        commandName: "publish",
        abstract: "Push a recipe to the Forage hub. (stub for now — wires to live hub once available.)"
    )

    @Argument(help: "Recipe directory (must contain recipe.forage).")
    var recipeDir: String

    func run() async throws {
        let recipePath = try RunCommand.resolveRecipePath(recipeDir)
        let src = try String(contentsOfFile: recipePath, encoding: .utf8)
        let recipe: Recipe
        do {
            recipe = try Parser.parse(source: src)
        } catch {
            FileHandle.standardError.write("parse failed: \(error)\n".data(using: .utf8)!)
            throw ExitCode.failure
        }
        let issues = Validator.validate(recipe)
        if issues.hasErrors {
            FileHandle.standardError.write("validation failed:\n".data(using: .utf8)!)
            for e in issues.errors {
                FileHandle.standardError.write(" - \(e.message) [\(e.location)]\n".data(using: .utf8)!)
            }
            throw ExitCode.failure
        }

        let env = ProcessInfo.processInfo.environment
        let hubURL = env["FORAGE_HUB_URL"] ?? "https://api.foragelang.com"
        let token = env["FORAGE_HUB_TOKEN"]
        let endpoint = "\(hubURL)/v1/recipes"

        guard let token, !token.isEmpty else {
            print("would POST recipe \"\(recipe.name)\" to \(endpoint)")
            print("(set FORAGE_HUB_TOKEN to actually publish)")
            return
        }

        guard let url = URL(string: endpoint) else {
            FileHandle.standardError.write("invalid FORAGE_HUB_URL: \(hubURL)\n".data(using: .utf8)!)
            throw ExitCode.failure
        }
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        let payload: [String: String] = [
            "name": recipe.name,
            "source": src,
        ]
        request.httpBody = try JSONSerialization.data(withJSONObject: payload)

        let (data, response) = try await URLSession.shared.data(for: request)
        guard let http = response as? HTTPURLResponse else {
            FileHandle.standardError.write("non-HTTP response\n".data(using: .utf8)!)
            throw ExitCode.failure
        }
        let body = String(data: data, encoding: .utf8) ?? ""
        if (200..<300).contains(http.statusCode) {
            print("published \"\(recipe.name)\" → \(endpoint)")
            if !body.isEmpty { print(body) }
        } else {
            FileHandle.standardError.write("publish failed (\(http.statusCode)):\n\(body)\n".data(using: .utf8)!)
            throw ExitCode.failure
        }
    }
}
