import Foundation

/// Transport that serves saved HTTP exchanges instead of hitting the network.
/// Used by `forage test` (replay mode) so a recipe can be validated against
/// captured fixtures without going live.
///
/// Fixture matching strategy: each `Fixture` is keyed by request method +
/// URL substring + (optional) request-body substring. The replayer walks
/// the fixtures in registration order and serves the first match. This is
/// deliberately permissive — recipes are run against captured fixtures
/// from the same recipe, so collisions are rare; and explicit ordering
/// (call N → fixture N) is brittle if the recipe ever changes its step
/// order.
public actor HTTPReplayer: Transport {
    public struct Fixture: Sendable {
        public let method: String
        public let urlSubstring: String
        public let bodySubstring: String?
        public let responseStatus: Int
        public let responseBody: Data
        public let responseHeaders: [String: String]

        public init(
            method: String,
            urlSubstring: String,
            bodySubstring: String? = nil,
            responseStatus: Int = 200,
            responseBody: Data,
            responseHeaders: [String: String] = ["Content-Type": "application/json"]
        ) {
            self.method = method
            self.urlSubstring = urlSubstring
            self.bodySubstring = bodySubstring
            self.responseStatus = responseStatus
            self.responseBody = responseBody
            self.responseHeaders = responseHeaders
        }
    }

    public private(set) var fixtures: [Fixture]
    public private(set) var unmatched: [URLRequest] = []

    public init(fixtures: [Fixture] = []) {
        self.fixtures = fixtures
    }

    public func add(_ fixture: Fixture) {
        fixtures.append(fixture)
    }

    public func send(_ request: URLRequest) async throws -> (Data, HTTPURLResponse) {
        let urlString = request.url?.absoluteString ?? ""
        let method = request.httpMethod ?? "GET"
        let body = request.httpBody.flatMap { String(data: $0, encoding: .utf8) } ?? ""

        for fixture in fixtures {
            if fixture.method != method { continue }
            if !urlString.contains(fixture.urlSubstring) { continue }
            if let bs = fixture.bodySubstring, !body.contains(bs) { continue }
            let response = HTTPURLResponse(
                url: request.url!,
                statusCode: fixture.responseStatus,
                httpVersion: "HTTP/1.1",
                headerFields: fixture.responseHeaders
            )!
            return (fixture.responseBody, response)
        }
        unmatched.append(request)
        throw HTTPReplayerError.noFixtureMatch(method: method, url: urlString, bodySnippet: String(body.prefix(120)))
    }
}

public enum HTTPReplayerError: Error, CustomStringConvertible {
    case noFixtureMatch(method: String, url: String, bodySnippet: String)

    public var description: String {
        switch self {
        case .noFixtureMatch(let m, let u, let b):
            return "no fixture matched: \(m) \(u) [body: \(b)]"
        }
    }
}

// MARK: - FixtureStore

/// On-disk directory layout for a recipe's fixtures + snapshot:
///
///     <recipe-dir>/
///       recipe.forage
///       fixtures/
///         01-prime.html
///         02-categories.json
///         03-products-flower-rec-1.json
///         …
///       snapshot.json
///       last-run.json   (gitignored)
///
/// `loadFixtures(at:)` reads each file and builds a list of `Fixture` values
/// keyed by its filename (so the recipe author can see which fixture
/// matched each capture in `last-run.json`). The matching logic in
/// `HTTPReplayer` only uses URL+method+body substrings, so the filename is
/// purely for human inspection.
public enum FixtureStore {
    /// Read the fixtures directory. Returns the raw `Data` of each file
    /// keyed by its basename — caller decides how to wire each one to
    /// `HTTPReplayer.Fixture` values (via a manifest or per-recipe
    /// convention). Manifest-driven loading lands when the on-disk layout
    /// stabilizes.
    public static func readFixtures(at directoryURL: URL) throws -> [String: Data] {
        let fm = FileManager.default
        var out: [String: Data] = [:]
        guard fm.fileExists(atPath: directoryURL.path) else { return out }
        let contents = try fm.contentsOfDirectory(at: directoryURL, includingPropertiesForKeys: nil)
        for url in contents where !url.lastPathComponent.hasPrefix(".") {
            let data = try Data(contentsOf: url)
            out[url.lastPathComponent] = data
        }
        return out
    }
}
