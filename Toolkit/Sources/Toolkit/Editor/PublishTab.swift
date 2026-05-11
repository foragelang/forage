import SwiftUI
import Forage

/// Publish form. Slug / name / summary / tags / license. Buttons: Validate,
/// Preview payload, Publish. The hub wiring is stubbed for M3 — Publish
/// calls `HubClient.publish` which prints "would POST" and returns the
/// payload to the UI.
struct PublishTab: View {
    let slug: String
    let source: String

    @Environment(ToolkitPreferences.self) private var preferences

    @State private var displayName: String = ""
    @State private var summary: String = ""
    @State private var tagsText: String = ""
    @State private var license: String = "MIT"
    @State private var validationOutput: String?
    @State private var previewedPayload: String?
    @State private var publishOutput: String?
    @State private var isPublishing = false

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                Form {
                    Section("Metadata") {
                        LabeledContent("Slug") {
                            Text(slug)
                                .font(.system(.body, design: .monospaced))
                                .textSelection(.enabled)
                                .foregroundStyle(.secondary)
                        }
                        TextField("Display name", text: $displayName, prompt: Text("Sweed dispensary scraper"))
                        TextField("Summary", text: $summary, prompt: Text("One-line description"))
                        TextField("Tags (comma-separated)", text: $tagsText, prompt: Text("cannabis, dispensary, jane"))
                        TextField("License", text: $license)
                    }
                }
                .formStyle(.grouped)
                .scrollDisabled(true)

                actionsBar

                if let validationOutput {
                    panel(title: "Validation", body: validationOutput, monospace: true)
                }
                if let previewedPayload {
                    panel(title: "Payload preview", body: previewedPayload, monospace: true)
                }
                if let publishOutput {
                    panel(title: "Publish output", body: publishOutput, monospace: false)
                }
            }
            .padding(20)
            .frame(maxWidth: 800, alignment: .topLeading)
        }
    }

    private var actionsBar: some View {
        HStack(spacing: 8) {
            Button("Validate") {
                validationOutput = runValidation()
            }
            Button("Preview payload") {
                previewedPayload = generatePayloadString()
            }
            Spacer()
            Button(isPublishing ? "Publishing…" : "Publish") {
                Task { await runPublish() }
            }
            .buttonStyle(.borderedProminent)
            .disabled(isPublishing)
        }
    }

    @ViewBuilder
    private func panel(title: String, body: String, monospace: Bool) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(title).font(.headline)
            ScrollView(.horizontal) {
                Text(body)
                    .font(monospace
                          ? .system(size: 11, design: .monospaced)
                          : .system(size: 12))
                    .textSelection(.enabled)
                    .padding(10)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(
                        RoundedRectangle(cornerRadius: 4)
                            .fill(Color(nsColor: .textBackgroundColor))
                    )
            }
        }
    }

    private func runValidation() -> String {
        let trimmed = source.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return "Recipe source is empty." }
        do {
            let recipe = try Parser.parse(source: source)
            let issues = Validator.validate(recipe)
            if issues.isEmpty {
                return "Parsed and validated. Recipe \"\(recipe.name)\" (engine: \(recipe.engineKind.rawValue))."
            }
            var lines: [String] = []
            for issue in issues {
                let tag = issue.severity == .error ? "ERROR" : "WARN"
                lines.append("[\(tag)] \(issue.message) [\(issue.location)]")
            }
            return lines.joined(separator: "\n")
        } catch {
            return "Parse error: \(error)"
        }
    }

    private func generatePayloadString() -> String {
        let payload = buildPayload()
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes]
        do {
            let data = try encoder.encode(payload)
            return String(data: data, encoding: .utf8) ?? "(failed to encode)"
        } catch {
            return "(encode error: \(error))"
        }
    }

    private func buildPayload() -> HubPublishPayload {
        let tags = tagsText
            .split(separator: ",")
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
        return HubPublishPayload(
            slug: slug,
            displayName: displayName,
            summary: summary.isEmpty ? nil : summary,
            tags: tags,
            body: source
        )
    }

    @MainActor
    private func runPublish() async {
        isPublishing = true
        defer { isPublishing = false }

        let token: String?
        do {
            token = try Keychain.readAPIKey()
        } catch {
            publishOutput = "Configure your API key in Preferences (Cmd-,) before publishing."
            return
        }
        guard let token, !token.isEmpty else {
            publishOutput = "Configure your API key in Preferences (Cmd-,) before publishing."
            return
        }

        guard let baseURL = URL(string: preferences.hubURL) else {
            publishOutput = "Hub URL is not a valid URL: \(preferences.hubURL)"
            return
        }

        let client = HubClient(baseURL: baseURL, token: token)
        do {
            let result = try await client.publish(buildPayload())
            publishOutput = """
                Published \(result.slug) v\(result.version)
                sha256: \(result.sha256)
                """
        } catch {
            publishOutput = "Publish failed: \(error)"
        }
    }
}
