import ArgumentParser

@main
struct ForageCLI: AsyncParsableCommand {
    static let configuration = CommandConfiguration(
        commandName: "forage",
        abstract: "Declarative scraping platform — runner, capture probe, scaffolder, test harness.",
        version: "0.0.3",
        subcommands: [
            RunCommand.self,
            CaptureCommand.self,
            ScaffoldCommand.self,
            TestCommand.self,
            PublishCommand.self,
        ]
    )
}
