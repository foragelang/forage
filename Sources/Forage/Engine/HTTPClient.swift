import Foundation

/// Polite HTTP client used by the runtime. Honest UA, ~1 req/sec rate limit
/// per host, exponential backoff on 429/5xx, JSON / form-encoded body support.
///
/// Pluggable via a `Transport` protocol so the test suite can replay fixtures
/// without making real requests.
public protocol Transport: Sendable {
    func send(_ request: URLRequest) async throws -> (Data, HTTPURLResponse)
}

/// Real-network transport backed by URLSession.
public struct URLSessionTransport: Transport {
    public let session: URLSession

    public init(session: URLSession = .shared) {
        self.session = session
    }

    public func send(_ request: URLRequest) async throws -> (Data, HTTPURLResponse) {
        let (data, response) = try await session.data(for: request)
        guard let http = response as? HTTPURLResponse else {
            throw HTTPClientError.notHTTPResponse
        }
        return (data, http)
    }
}

public actor HTTPClient {
    public let transport: Transport
    public let userAgent: String
    public let minRequestInterval: TimeInterval
    public let maxRetries: Int

    private var lastRequestAt: [String: Date] = [:]

    public init(
        transport: Transport,
        userAgent: String = "forage/0.0.3 (+https://github.com/dminkovsky/forage)",
        minRequestInterval: TimeInterval = 1.0,
        maxRetries: Int = 3
    ) {
        self.transport = transport
        self.userAgent = userAgent
        self.minRequestInterval = minRequestInterval
        self.maxRetries = maxRetries
    }

    public func send(_ request: URLRequest) async throws -> (Data, HTTPURLResponse) {
        let host = request.url?.host ?? "?"
        await waitIfNeeded(host: host)

        var request = request
        if request.value(forHTTPHeaderField: "User-Agent") == nil {
            request.setValue(userAgent, forHTTPHeaderField: "User-Agent")
        }

        var attempt = 0
        while true {
            do {
                let (data, response) = try await transport.send(request)
                if (200..<300).contains(response.statusCode) {
                    return (data, response)
                }
                if (response.statusCode == 429 || (500..<600).contains(response.statusCode)) && attempt < maxRetries {
                    attempt += 1
                    let delay = pow(2.0, Double(attempt))
                    try? await Task.sleep(nanoseconds: UInt64(delay * 1_000_000_000))
                    continue
                }
                let body = String(data: data, encoding: .utf8)?.prefix(200).description
                throw HTTPClientError.badStatus(code: response.statusCode, snippet: body)
            } catch {
                if Self.isTransientNetworkError(error) && attempt < maxRetries {
                    attempt += 1
                    let delay = pow(2.0, Double(attempt))
                    try? await Task.sleep(nanoseconds: UInt64(delay * 1_000_000_000))
                    continue
                }
                throw error
            }
        }
    }

    /// True for thrown errors that represent a transient network condition
    /// worth retrying (connection drops, DNS hiccups, timeouts). Everything
    /// else — `HTTPClientError.badStatus`, decode failures, mock-transport
    /// failures, programmer mistakes — fails fast.
    static func isTransientNetworkError(_ error: Error) -> Bool {
        if let urlError = error as? URLError {
            return urlError.isTransient
        }
        return false
    }

    private func waitIfNeeded(host: String) async {
        if let last = lastRequestAt[host] {
            let elapsed = Date().timeIntervalSince(last)
            if elapsed < minRequestInterval {
                try? await Task.sleep(nanoseconds: UInt64((minRequestInterval - elapsed) * 1_000_000_000))
            }
        }
        lastRequestAt[host] = Date()
    }
}

extension URLError {
    /// Network-layer codes that are worth retrying — packet loss, DNS,
    /// timeouts, transient unreachability. 4xx/5xx HTTP responses surface
    /// as `HTTPURLResponse` (handled separately) and aren't covered here.
    var isTransient: Bool {
        switch code {
        case .notConnectedToInternet,
             .timedOut,
             .networkConnectionLost,
             .dnsLookupFailed,
             .cannotConnectToHost,
             .cannotFindHost:
            return true
        default:
            return false
        }
    }
}

public enum HTTPClientError: Error, CustomStringConvertible {
    case notHTTPResponse
    case badStatus(code: Int, snippet: String?)

    public var description: String {
        switch self {
        case .notHTTPResponse:
            return "response was not an HTTPURLResponse"
        case .badStatus(let code, let snippet):
            return "HTTP \(code)\(snippet.map { ": \($0)" } ?? "")"
        }
    }
}

// MARK: - Body / URL building helpers used by the engine

public enum BodyEncoding {
    /// URL-encode a flat key/value list. Allowed character set covers RFC
    /// 3986 unreserved + the sub-delims that x-www-form-urlencoded permits.
    public static func formEncoded(_ pairs: [(String, String)]) -> Data {
        var allowed = CharacterSet.alphanumerics
        allowed.insert(charactersIn: "-._~")
        let body = pairs.map { (k, v) in
            "\(k.addingPercentEncoding(withAllowedCharacters: allowed) ?? k)=\(v.addingPercentEncoding(withAllowedCharacters: allowed) ?? v)"
        }.joined(separator: "&")
        return body.data(using: .utf8) ?? Data()
    }
}
