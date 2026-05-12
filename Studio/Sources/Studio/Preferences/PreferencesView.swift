import SwiftUI
import AppKit

/// `Cmd-,` settings pane. Hub URL is plain `@AppStorage`-backed; API key
/// + OAuth tokens live in Keychain via `Keychain` helpers.
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

    // M11 OAuth state
    @State private var signedInLogin: String?
    @State private var signInUserCode: String?
    @State private var signInVerificationURL: String?
    @State private var signInStatus: String?
    @State private var signInError: String?
    @State private var signInTask: Task<Void, Never>?

    var body: some View {
        @Bindable var bindable = prefs
        Form {
            Section("Hub") {
                TextField("Hub URL", text: $bindable.hubURL, prompt: Text("https://api.foragelang.com"))
                    .textFieldStyle(.roundedBorder)
                    .font(.system(.body, design: .monospaced))
            }
            Section("Account") {
                if let signedInLogin {
                    HStack {
                        Label("Signed in as \(signedInLogin)", systemImage: "person.circle.fill")
                            .foregroundStyle(.green)
                        Spacer()
                        Button("Sign out") { signOut() }
                    }
                } else if let code = signInUserCode, let url = signInVerificationURL {
                    VStack(alignment: .leading, spacing: 8) {
                        Text("Open \(url) and enter the code:")
                            .font(.callout)
                        Text(code)
                            .font(.system(.title3, design: .monospaced).weight(.bold))
                            .textSelection(.enabled)
                        HStack {
                            Button("Open browser") {
                                if let u = URL(string: url) { NSWorkspace.shared.open(u) }
                            }
                            Button("Cancel") { cancelSignIn() }
                            Spacer()
                            ProgressView().controlSize(.small)
                            if let status = signInStatus {
                                Text(status).font(.caption).foregroundStyle(.secondary)
                            }
                        }
                    }
                } else {
                    HStack {
                        Button("Sign in with GitHub") { startSignIn() }
                        Spacer()
                        if let err = signInError {
                            Text(err).font(.caption).foregroundStyle(.red)
                        }
                    }
                }
            }
            Section("API key (legacy / admin)") {
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
        if let tokens = try? Keychain.readOAuthTokens() {
            signedInLogin = tokens.login
        }
    }

    // MARK: - OAuth flow

    private func startSignIn() {
        signInError = nil
        signInStatus = "Requesting device code…"
        let hub = prefs.hubURL
        signInTask = Task { [hub] in
            do {
                let url = URL(string: hub)!
                let start: DeviceStartResp = try await postJSONNoAuth(
                    URL(string: "v1/oauth/device", relativeTo: url)!,
                    body: EmptyBody()
                )
                await MainActor.run {
                    self.signInUserCode = start.userCode
                    self.signInVerificationURL = start.verificationURL
                    self.signInStatus = "Waiting for browser confirmation…"
                }
                if let u = URL(string: start.verificationURL) {
                    await MainActor.run { NSWorkspace.shared.open(u) }
                }
                try await pollUntilDone(hub: url, deviceCode: start.deviceCode, interval: start.interval, expiresIn: start.expiresIn)
            } catch {
                await MainActor.run {
                    self.signInError = String(describing: error)
                    self.signInUserCode = nil
                    self.signInVerificationURL = nil
                    self.signInStatus = nil
                }
            }
        }
    }

    private func pollUntilDone(hub: URL, deviceCode: String, interval: Int, expiresIn: Int) async throws {
        let deadline = Date().addingTimeInterval(TimeInterval(expiresIn))
        while Date() < deadline {
            try? await Task.sleep(nanoseconds: UInt64(interval) * 1_000_000_000)
            let resp: DevicePollResp
            do {
                resp = try await postJSONNoAuth(
                    URL(string: "v1/oauth/device/poll", relativeTo: hub)!,
                    body: DevicePollBody(deviceCode: deviceCode)
                )
            } catch HTTPErr.status(202, _) {
                continue
            }
            if resp.status == "ok",
               let access = resp.accessToken,
               let refresh = resp.refreshToken,
               let user = resp.user
            {
                let tokens = Keychain.OAuthTokens(
                    login: user.login,
                    accessToken: access,
                    refreshToken: refresh,
                    updatedAt: Date()
                )
                try Keychain.writeOAuthTokens(tokens)
                await MainActor.run {
                    self.signedInLogin = user.login
                    self.signInUserCode = nil
                    self.signInVerificationURL = nil
                    self.signInStatus = nil
                }
                return
            }
        }
        throw HTTPErr.timeout
    }

    private func cancelSignIn() {
        signInTask?.cancel()
        signInTask = nil
        signInUserCode = nil
        signInVerificationURL = nil
        signInStatus = nil
    }

    private func signOut() {
        try? Keychain.deleteOAuthTokens()
        signedInLogin = nil
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
