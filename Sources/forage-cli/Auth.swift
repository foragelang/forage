import ArgumentParser
import Foundation
import Forage

/// `forage auth` — sign in / sign out / whoami via the hub OAuth flow.
///
/// Tokens are stored at `~/Library/Forage/Auth/<host>.json` with mode
/// 600. The CLI's other subcommands (notably `forage publish`) read the
/// access token from the same file and attach it as a Bearer header
/// when calling `api.foragelang.com`.
struct AuthCommand: AsyncParsableCommand {
    static let configuration = CommandConfiguration(
        commandName: "auth",
        abstract: "Sign in to api.foragelang.com via GitHub.",
        subcommands: [
            AuthLogin.self,
            AuthLogout.self,
            AuthWhoami.self,
        ]
    )
}

// MARK: - login

struct AuthLogin: AsyncParsableCommand {
    static let configuration = CommandConfiguration(
        commandName: "login",
        abstract: "Sign in to the hub via the GitHub device-code flow."
    )

    @Option(name: .customLong("hub"), help: "Hub API URL. Defaults to FORAGE_HUB_URL or https://api.foragelang.com.")
    var hub: String?

    @Option(name: .customLong("poll-seconds"), help: "Override the GitHub-suggested polling interval (seconds).")
    var pollSeconds: Int?

    func run() async throws {
        let hubURL = hub ?? ProcessInfo.processInfo.environment["FORAGE_HUB_URL"] ?? "https://api.foragelang.com"
        guard let base = URL(string: hubURL) else {
            FileHandle.standardError.write("invalid hub URL: \(hubURL)\n".data(using: .utf8)!)
            throw ExitCode.failure
        }

        // 1) Kick off device-code flow.
        let start: DeviceStartResponse
        do {
            start = try await postJSON(
                url: base.appendingPathComponent("v1/oauth/device"),
                body: EmptyBody()
            )
        } catch {
            FileHandle.standardError.write("oauth device start failed: \(error)\n".data(using: .utf8)!)
            throw ExitCode.failure
        }

        print("Open \(start.verificationURL) in a browser.")
        print("Enter the user code: \(start.userCode)")
        print()

        let interval = pollSeconds ?? start.interval
        let timeout = TimeInterval(start.expiresIn)
        let deadline = Date().addingTimeInterval(timeout)

        // 2) Poll until the user completes the GitHub side.
        while Date() < deadline {
            try await Task.sleep(nanoseconds: UInt64(interval) * 1_000_000_000)
            let resp: DevicePollResponse
            do {
                resp = try await postJSON(
                    url: base.appendingPathComponent("v1/oauth/device/poll"),
                    body: DevicePollBody(deviceCode: start.deviceCode)
                )
            } catch let HTTPError.status(code, payload) where code == 202 {
                // Pending; try again.
                _ = payload
                continue
            } catch {
                FileHandle.standardError.write("oauth poll failed: \(error)\n".data(using: .utf8)!)
                throw ExitCode.failure
            }
            if resp.status == "ok", let access = resp.accessToken, let refresh = resp.refreshToken, let user = resp.user {
                try AuthStore.write(host: base, access: access, refresh: refresh, login: user.login)
                print("✓ Signed in as \(user.login)" + (user.name.map { " (\($0))" } ?? ""))
                return
            }
            if resp.status == "pending" { continue }
        }
        FileHandle.standardError.write("oauth flow timed out\n".data(using: .utf8)!)
        throw ExitCode.failure
    }
}

// MARK: - logout

struct AuthLogout: AsyncParsableCommand {
    static let configuration = CommandConfiguration(
        commandName: "logout",
        abstract: "Forget the cached hub credentials. Optionally revoke the refresh token server-side."
    )

    @Option(name: .customLong("hub"), help: "Hub API URL.")
    var hub: String?

    @Flag(name: .customLong("revoke"), help: "Also revoke the token server-side.")
    var revoke: Bool = false

    func run() async throws {
        let hubURL = hub ?? ProcessInfo.processInfo.environment["FORAGE_HUB_URL"] ?? "https://api.foragelang.com"
        guard let base = URL(string: hubURL) else {
            FileHandle.standardError.write("invalid hub URL: \(hubURL)\n".data(using: .utf8)!)
            throw ExitCode.failure
        }
        if revoke {
            if let creds = AuthStore.read(host: base) {
                _ = try? await postBearer(
                    url: base.appendingPathComponent("v1/oauth/revoke"),
                    bearer: creds.accessToken,
                    body: EmptyBody()
                ) as VoidResponse?
            }
        }
        AuthStore.delete(host: base)
        print("Signed out.")
    }
}

// MARK: - whoami

struct AuthWhoami: AsyncParsableCommand {
    static let configuration = CommandConfiguration(
        commandName: "whoami",
        abstract: "Print the currently signed-in GitHub login."
    )

    @Option(name: .customLong("hub"), help: "Hub API URL.")
    var hub: String?

    func run() async throws {
        let hubURL = hub ?? ProcessInfo.processInfo.environment["FORAGE_HUB_URL"] ?? "https://api.foragelang.com"
        guard let base = URL(string: hubURL) else {
            FileHandle.standardError.write("invalid hub URL: \(hubURL)\n".data(using: .utf8)!)
            throw ExitCode.failure
        }
        guard let creds = AuthStore.read(host: base) else {
            print("not signed in")
            return
        }
        print(creds.login)
    }
}

// MARK: - Storage

/// Auth-token storage. `~/Library/Forage/Auth/<host>.json`, chmod 600.
/// Per-host so a user can be signed into multiple hubs (e.g. a local
/// dev hub at `http://localhost:8787` alongside production).
enum AuthStore {
    struct Credentials: Codable {
        let host: String
        let login: String
        let accessToken: String
        let refreshToken: String
        let updatedAt: Date
    }

    static func file(host: URL) -> URL {
        let fm = FileManager.default
        let lib = fm.urls(for: .libraryDirectory, in: .userDomainMask).first ?? fm.homeDirectoryForCurrentUser.appendingPathComponent("Library")
        let dir = lib
            .appendingPathComponent("Forage", isDirectory: true)
            .appendingPathComponent("Auth", isDirectory: true)
        let key = (host.host ?? "default") + (host.port.map { ":\($0)" } ?? "")
        return dir.appendingPathComponent("\(key).json")
    }

    static func read(host: URL) -> Credentials? {
        guard let data = try? Data(contentsOf: file(host: host)) else { return nil }
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        return try? decoder.decode(Credentials.self, from: data)
    }

    static func write(host: URL, access: String, refresh: String, login: String) throws {
        let creds = Credentials(
            host: host.absoluteString,
            login: login,
            accessToken: access,
            refreshToken: refresh,
            updatedAt: Date()
        )
        let url = file(host: host)
        try FileManager.default.createDirectory(
            at: url.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        let data = try encoder.encode(creds)
        FileManager.default.createFile(
            atPath: url.path,
            contents: nil,
            attributes: [.posixPermissions: 0o600]
        )
        try data.write(to: url, options: .atomic)
        try FileManager.default.setAttributes(
            [.posixPermissions: 0o600],
            ofItemAtPath: url.path
        )
    }

    static func delete(host: URL) {
        _ = try? FileManager.default.removeItem(at: file(host: host))
    }
}

// MARK: - HTTP plumbing

struct EmptyBody: Encodable {}

struct DevicePollBody: Encodable {
    let deviceCode: String
}

struct DeviceStartResponse: Decodable {
    let deviceCode: String
    let userCode: String
    let verificationURL: String
    let interval: Int
    let expiresIn: Int
}

struct DevicePollResponse: Decodable {
    let status: String
    let accessToken: String?
    let refreshToken: String?
    let user: PolledUser?
    let error: String?
}

struct PolledUser: Decodable {
    let login: String
    let name: String?
    let avatarUrl: String?
}

struct VoidResponse: Decodable {}

enum HTTPError: Error, CustomStringConvertible {
    case status(Int, String)
    case malformed(String)

    var description: String {
        switch self {
        case .status(let c, let body): return "HTTP \(c): \(body)"
        case .malformed(let r):        return "malformed response: \(r)"
        }
    }
}

func postJSON<R: Decodable, B: Encodable>(url: URL, body: B) async throws -> R {
    var req = URLRequest(url: url)
    req.httpMethod = "POST"
    req.setValue("application/json", forHTTPHeaderField: "Content-Type")
    req.setValue("application/json", forHTTPHeaderField: "Accept")
    req.httpBody = try JSONEncoder().encode(body)
    let (data, resp) = try await URLSession.shared.data(for: req)
    guard let http = resp as? HTTPURLResponse else {
        throw HTTPError.malformed("non-HTTP response")
    }
    if !(200..<300).contains(http.statusCode) {
        throw HTTPError.status(http.statusCode, String(data: data, encoding: .utf8) ?? "")
    }
    return try JSONDecoder().decode(R.self, from: data)
}

func postBearer<R: Decodable, B: Encodable>(url: URL, bearer: String, body: B) async throws -> R? {
    var req = URLRequest(url: url)
    req.httpMethod = "POST"
    req.setValue("Bearer \(bearer)", forHTTPHeaderField: "Authorization")
    req.setValue("application/json", forHTTPHeaderField: "Content-Type")
    req.httpBody = try JSONEncoder().encode(body)
    let (_, resp) = try await URLSession.shared.data(for: req)
    guard let http = resp as? HTTPURLResponse else { return nil }
    if !(200..<300).contains(http.statusCode) {
        throw HTTPError.status(http.statusCode, "")
    }
    return nil
}
