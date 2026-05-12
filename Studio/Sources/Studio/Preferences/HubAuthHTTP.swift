import Foundation

/// Tiny POST-JSON helper for Studio's OAuth device-code flow.
/// Lives here (rather than reusing `HubClient`) because the Forage
/// package's HubClient is recipe-oriented and not parameterized over
/// the OAuth endpoints. Future M11 refactor can fold these into
/// `Sources/Forage/Hub/HubClient.swift` once the shape stabilizes.

struct EmptyBody: Encodable {}

struct DevicePollBody: Encodable {
    let deviceCode: String
}

struct DeviceStartResp: Decodable {
    let deviceCode: String
    let userCode: String
    let verificationURL: String
    let interval: Int
    let expiresIn: Int
}

struct DevicePollResp: Decodable {
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

enum HTTPErr: Error, CustomStringConvertible {
    case status(Int, String)
    case malformed(String)
    case timeout

    var description: String {
        switch self {
        case .status(let c, let body): return "HTTP \(c): \(body)"
        case .malformed(let r):        return "malformed response: \(r)"
        case .timeout:                 return "oauth flow timed out"
        }
    }
}

func postJSONNoAuth<R: Decodable, B: Encodable>(_ url: URL, body: B) async throws -> R {
    var req = URLRequest(url: url)
    req.httpMethod = "POST"
    req.setValue("application/json", forHTTPHeaderField: "Content-Type")
    req.setValue("application/json", forHTTPHeaderField: "Accept")
    req.httpBody = try JSONEncoder().encode(body)
    let (data, resp) = try await URLSession.shared.data(for: req)
    guard let http = resp as? HTTPURLResponse else {
        throw HTTPErr.malformed("non-HTTP response")
    }
    if !(200..<300).contains(http.statusCode) {
        throw HTTPErr.status(http.statusCode, String(data: data, encoding: .utf8) ?? "")
    }
    return try JSONDecoder().decode(R.self, from: data)
}
