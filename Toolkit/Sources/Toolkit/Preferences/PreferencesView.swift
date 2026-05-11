import SwiftUI

/// `Cmd-,` settings pane. Hub URL is plain `@AppStorage`-backed; API key
/// lives in Keychain via `Keychain` helpers.
@MainActor
@Observable
final class ToolkitPreferences {
    var hubURL: String {
        didSet { UserDefaults.standard.set(hubURL, forKey: Self.hubURLKey) }
    }

    init() {
        self.hubURL = UserDefaults.standard.string(forKey: Self.hubURLKey) ?? "https://api.foragelang.com"
    }

    private static let hubURLKey = "toolkit.hub.url"
}

struct PreferencesView: View {
    @Environment(ToolkitPreferences.self) private var prefs

    @State private var apiKeyEntry: String = ""
    @State private var apiKeyStored: Bool = false
    @State private var saveError: String?
    @State private var saveSuccess: Bool = false

    var body: some View {
        @Bindable var bindable = prefs
        Form {
            Section("Hub") {
                TextField("Hub URL", text: $bindable.hubURL, prompt: Text("https://api.foragelang.com"))
                    .textFieldStyle(.roundedBorder)
                    .font(.system(.body, design: .monospaced))
            }
            Section("API key") {
                SecureField("API key", text: $apiKeyEntry, prompt: Text(apiKeyStored ? "(stored in Keychain)" : "Paste your hub API key"))
                    .textFieldStyle(.roundedBorder)
                HStack {
                    Button("Save to Keychain") { saveKey() }
                        .disabled(apiKeyEntry.isEmpty)
                    Button("Delete from Keychain") { deleteKey() }
                        .disabled(!apiKeyStored)
                    Spacer()
                    if saveSuccess {
                        Label("Saved", systemImage: "checkmark.circle.fill")
                            .foregroundStyle(.green)
                            .labelStyle(.titleAndIcon)
                            .font(.caption)
                    }
                }
                if let saveError {
                    Text(saveError)
                        .font(.caption)
                        .foregroundStyle(.red)
                }
            }
        }
        .padding(16)
        .formStyle(.grouped)
        .task { refreshStoredState() }
    }

    private func refreshStoredState() {
        apiKeyStored = (try? Keychain.readAPIKey()) != nil
    }

    private func saveKey() {
        do {
            try Keychain.writeAPIKey(apiKeyEntry)
            apiKeyEntry = ""
            saveError = nil
            saveSuccess = true
            refreshStoredState()
            Task {
                try? await Task.sleep(nanoseconds: 1_500_000_000)
                await MainActor.run { saveSuccess = false }
            }
        } catch {
            saveError = String(describing: error)
        }
    }

    private func deleteKey() {
        do {
            try Keychain.deleteAPIKey()
            saveError = nil
            refreshStoredState()
        } catch {
            saveError = String(describing: error)
        }
    }
}
