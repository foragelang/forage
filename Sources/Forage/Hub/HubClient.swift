import Foundation
#if canImport(FoundationNetworking)
import FoundationNetworking
#endif

/// Pointer to a recipe in the hub. `slug` is either `name` (e.g.
/// `sample-recipe`) or `author/name` (e.g. `alice/awesome-recipe`). The
/// hub URL is always `/v1/recipes/<slug>` — author-qualified slugs use a
/// `/` in the slug; the runtime URL-encodes it when building requests.
public struct HubRecipeRef: Sendable, Hashable {
    public let slug: String
    public let version: Int?

    public init(slug: String, version: Int? = nil) {
        self.slug = slug
        self.version = version
    }

    /// Parse `hub://<slug>` or `hub://<author>/<name>` (with optional ` v<N>`
    /// version suffix handled by the parser, not here). The string is what
    /// the parser sees after stripping `hub://`.
    public static func parse(_ raw: String) -> HubRecipeRef? {
        let trimmed = raw.trimmingCharacters(in: .whitespaces)
        guard !trimmed.isEmpty else { return nil }
        // Slug may be `name` or `author/name`. Validate each segment lightly.
        let segments = trimmed.split(separator: "/").map(String.init)
        if segments.count == 1 {
            return validSlugSegment(segments[0])
                ? HubRecipeRef(slug: segments[0])
                : nil
        }
        if segments.count == 2 {
            return validSlugSegment(segments[0]) && validSlugSegment(segments[1])
                ? HubRecipeRef(slug: "\(segments[0])/\(segments[1])")
                : nil
        }
        return nil
    }

    private static func validSlugSegment(_ s: String) -> Bool {
        guard let first = s.first, first.isLetter || first.isNumber else { return false }
        for c in s {
            let ok = c.isLetter || c.isNumber || c == "-" || c == "_"
            if !ok { return false }
        }
        return s.count >= 1 && s.count <= 64
    }
}

/// Lightweight metadata returned by `list` / `versions`. Mirrors the hub's
/// `ListingItem` shape.
public struct HubRecipeMeta: Sendable, Hashable, Codable {
    public let slug: String
    public let author: String?
    public let displayName: String
    public let summary: String?
    public let tags: [String]
    public let version: Int
    public let sha256: String?
    public let publishedAt: Date?

    public init(
        slug: String,
        author: String? = nil,
        displayName: String,
        summary: String? = nil,
        tags: [String] = [],
        version: Int,
        sha256: String? = nil,
        publishedAt: Date? = nil
    ) {
        self.slug = slug
        self.author = author
        self.displayName = displayName
        self.summary = summary
        self.tags = tags
        self.version = version
        self.sha256 = sha256
        self.publishedAt = publishedAt
    }

    // The hub uses `createdAt` / `updatedAt` rather than `publishedAt`.
    // Be lenient — accept either key from server responses, and decode
    // ISO8601 strings rather than relying on the decoder's date strategy
    // (which is set per-call but easy to drop).
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        slug = try c.decode(String.self, forKey: .slug)
        author = try c.decodeIfPresent(String.self, forKey: .author)
        displayName = try c.decode(String.self, forKey: .displayName)
        summary = try c.decodeIfPresent(String.self, forKey: .summary)
        tags = (try c.decodeIfPresent([String].self, forKey: .tags)) ?? []
        version = try c.decode(Int.self, forKey: .version)
        sha256 = try c.decodeIfPresent(String.self, forKey: .sha256)
        let publishedRaw = try c.decodeIfPresent(String.self, forKey: .publishedAt)
        let updatedRaw = try c.decodeIfPresent(String.self, forKey: .updatedAt)
        let createdRaw = try c.decodeIfPresent(String.self, forKey: .createdAt)
        let date = publishedRaw ?? updatedRaw ?? createdRaw
        publishedAt = date.flatMap(Self.iso8601Date)
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(slug, forKey: .slug)
        try c.encodeIfPresent(author, forKey: .author)
        try c.encode(displayName, forKey: .displayName)
        try c.encodeIfPresent(summary, forKey: .summary)
        try c.encode(tags, forKey: .tags)
        try c.encode(version, forKey: .version)
        try c.encodeIfPresent(sha256, forKey: .sha256)
        if let publishedAt {
            try c.encode(Self.iso8601String(publishedAt), forKey: .publishedAt)
        }
    }

    private enum CodingKeys: String, CodingKey {
        case slug, author, displayName, summary, tags, version, sha256
        case publishedAt, createdAt, updatedAt
    }

    fileprivate static func iso8601Date(_ s: String) -> Date? {
        // ISO8601DateFormatter is not Sendable; build a fresh one per call.
        // Hub responses are small; cost is negligible vs. correctness.
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        if let d = f.date(from: s) { return d }
        f.formatOptions = [.withInternetDateTime]
        return f.date(from: s)
    }
    fileprivate static func iso8601String(_ d: Date) -> String {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return f.string(from: d)
    }
}

/// Full recipe payload returned by `get` — metadata + body source.
public struct HubRecipe: Sendable, Hashable, Codable {
    public let meta: HubRecipeMeta
    public let body: String

    public init(meta: HubRecipeMeta, body: String) {
        self.meta = meta
        self.body = body
    }

    // Hub returns metadata fields and `body` flat in one JSON object — split
    // them apart during decoding.
    public init(from decoder: Decoder) throws {
        meta = try HubRecipeMeta(from: decoder)
        let c = try decoder.container(keyedBy: CodingKeys.self)
        body = try c.decode(String.self, forKey: .body)
    }
    public func encode(to encoder: Encoder) throws {
        try meta.encode(to: encoder)
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(body, forKey: .body)
    }

    private enum CodingKeys: String, CodingKey {
        case body
    }
}

/// Request body for `publish`. Mirrors `hub-api`'s `PublishRequest`.
public struct HubPublishPayload: Sendable, Hashable, Codable {
    public let slug: String
    public let author: String?
    public let displayName: String
    public let summary: String?
    public let tags: [String]
    public let body: String
    public let fixtures: String?
    public let snapshot: String?

    public init(
        slug: String,
        author: String? = nil,
        displayName: String,
        summary: String? = nil,
        tags: [String] = [],
        body: String,
        fixtures: String? = nil,
        snapshot: String? = nil
    ) {
        self.slug = slug
        self.author = author
        self.displayName = displayName
        self.summary = summary
        self.tags = tags
        self.body = body
        self.fixtures = fixtures
        self.snapshot = snapshot
    }
}

/// Response from `publish`.
public struct HubPublishResult: Sendable, Hashable, Codable {
    public let slug: String
    public let version: Int
    public let sha256: String

    public init(slug: String, version: Int, sha256: String) {
        self.slug = slug
        self.version = version
        self.sha256 = sha256
    }
}

/// Talks to `api.foragelang.com` (override via `baseURL`).
///
/// `publish` requires a token; reads (`list`, `get`, `versions`, `health`)
/// don't. The `URLSession` is injectable so tests can wire in a
/// `URLProtocol`-stubbed session — the production caller just uses
/// `.shared`.
public actor HubClient {
    public enum Error: Swift.Error, Sendable, CustomStringConvertible {
        case missingToken
        case http(status: Int, body: String)
        case notFound(slug: String)
        case malformed(reason: String)
        case transport(any Swift.Error)

        public var description: String {
            switch self {
            case .missingToken:
                return "hub: no API token set (export FORAGE_HUB_TOKEN)"
            case .http(let s, let body):
                return "hub: HTTP \(s)\(body.isEmpty ? "" : ": \(body)")"
            case .notFound(let slug):
                return "hub: recipe not found: \(slug)"
            case .malformed(let reason):
                return "hub: malformed response (\(reason))"
            case .transport(let err):
                return "hub: transport error: \(err)"
            }
        }
    }

    public static let defaultBaseURL = URL(string: "https://api.foragelang.com")!

    public let baseURL: URL
    public let token: String?
    private let session: URLSession

    public init(
        baseURL: URL = HubClient.defaultBaseURL,
        token: String? = nil,
        session: URLSession = .shared
    ) {
        self.baseURL = baseURL
        self.token = token
        self.session = session
    }

    // MARK: - Endpoints

    public func health() async throws -> Bool {
        let req = try makeRequest(method: "GET", path: "/v1/health")
        let (data, resp) = try await send(req)
        guard let http = resp as? HTTPURLResponse else {
            throw Error.malformed(reason: "non-HTTP response")
        }
        guard (200..<300).contains(http.statusCode) else {
            throw Error.http(status: http.statusCode, body: String(data: data, encoding: .utf8) ?? "")
        }
        struct Health: Decodable { let status: String }
        do {
            let h = try JSONDecoder().decode(Health.self, from: data)
            return h.status == "ok"
        } catch {
            throw Error.malformed(reason: "\(error)")
        }
    }

    public func list(
        author: String? = nil,
        tag: String? = nil,
        limit: Int = 50,
        cursor: String? = nil
    ) async throws -> (items: [HubRecipeMeta], nextCursor: String?) {
        var items: [URLQueryItem] = []
        if let author { items.append(URLQueryItem(name: "author", value: author)) }
        if let tag { items.append(URLQueryItem(name: "tag", value: tag)) }
        items.append(URLQueryItem(name: "limit", value: String(limit)))
        if let cursor { items.append(URLQueryItem(name: "cursor", value: cursor)) }

        let req = try makeRequest(method: "GET", path: "/v1/recipes", query: items)
        let (data, resp) = try await send(req)
        guard let http = resp as? HTTPURLResponse else {
            throw Error.malformed(reason: "non-HTTP response")
        }
        guard (200..<300).contains(http.statusCode) else {
            throw Error.http(status: http.statusCode, body: String(data: data, encoding: .utf8) ?? "")
        }

        struct Response: Decodable {
            let items: [HubRecipeMeta]
            let nextCursor: String?
        }
        do {
            let r = try JSONDecoder().decode(Response.self, from: data)
            return (r.items, r.nextCursor)
        } catch {
            throw Error.malformed(reason: "\(error)")
        }
    }

    public func get(_ ref: HubRecipeRef) async throws -> HubRecipe {
        var items: [URLQueryItem] = []
        if let v = ref.version { items.append(URLQueryItem(name: "version", value: String(v))) }
        let req = try makeRequest(
            method: "GET",
            path: "/v1/recipes/\(escapeSlugPath(ref.slug))",
            query: items.isEmpty ? nil : items
        )
        let (data, resp) = try await send(req)
        guard let http = resp as? HTTPURLResponse else {
            throw Error.malformed(reason: "non-HTTP response")
        }
        if http.statusCode == 404 {
            throw Error.notFound(slug: ref.slug)
        }
        guard (200..<300).contains(http.statusCode) else {
            throw Error.http(status: http.statusCode, body: String(data: data, encoding: .utf8) ?? "")
        }
        do {
            return try JSONDecoder().decode(HubRecipe.self, from: data)
        } catch {
            throw Error.malformed(reason: "\(error)")
        }
    }

    public func versions(slug: String) async throws -> [HubRecipeMeta] {
        let req = try makeRequest(
            method: "GET",
            path: "/v1/recipes/\(escapeSlugPath(slug))/versions"
        )
        let (data, resp) = try await send(req)
        guard let http = resp as? HTTPURLResponse else {
            throw Error.malformed(reason: "non-HTTP response")
        }
        if http.statusCode == 404 {
            throw Error.notFound(slug: slug)
        }
        guard (200..<300).contains(http.statusCode) else {
            throw Error.http(status: http.statusCode, body: String(data: data, encoding: .utf8) ?? "")
        }

        // Server returns [{version, publishedAt, sha256}]; lift each into a
        // HubRecipeMeta filling in the slug from the request.
        struct VersionEntry: Decodable {
            let version: Int
            let publishedAt: String?
            let sha256: String?
        }
        do {
            let raw = try JSONDecoder().decode([VersionEntry].self, from: data)
            return raw.map { entry in
                HubRecipeMeta(
                    slug: slug,
                    displayName: slug,
                    version: entry.version,
                    sha256: entry.sha256,
                    publishedAt: entry.publishedAt.flatMap(HubRecipeMeta.iso8601Date)
                )
            }
        } catch {
            throw Error.malformed(reason: "\(error)")
        }
    }

    public func publish(_ payload: HubPublishPayload) async throws -> HubPublishResult {
        guard let token, !token.isEmpty else {
            throw Error.missingToken
        }
        var req = try makeRequest(method: "POST", path: "/v1/recipes", authToken: token)
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        let encoder = JSONEncoder()
        // The server tolerates either `null` or omitted, but we mirror its
        // own schema (omitted) so the wire shape matches a hand-authored
        // payload too.
        encoder.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
        req.httpBody = try encoder.encode(payload)

        let (data, resp) = try await send(req)
        guard let http = resp as? HTTPURLResponse else {
            throw Error.malformed(reason: "non-HTTP response")
        }
        guard (200..<300).contains(http.statusCode) else {
            throw Error.http(status: http.statusCode, body: String(data: data, encoding: .utf8) ?? "")
        }
        do {
            return try JSONDecoder().decode(HubPublishResult.self, from: data)
        } catch {
            throw Error.malformed(reason: "\(error)")
        }
    }

    // MARK: - Internals

    private func makeRequest(
        method: String,
        path: String,
        query: [URLQueryItem]? = nil,
        authToken: String? = nil
    ) throws -> URLRequest {
        var comps = URLComponents()
        comps.scheme = baseURL.scheme
        comps.host = baseURL.host
        comps.port = baseURL.port
        let basePath = baseURL.path.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        let joined = basePath.isEmpty ? path : "/" + basePath + path
        comps.path = joined
        comps.queryItems = query
        guard let url = comps.url else {
            throw Error.malformed(reason: "could not assemble URL from \(baseURL) + \(path)")
        }
        var req = URLRequest(url: url)
        req.httpMethod = method
        req.setValue("forage/\(Forage.version)", forHTTPHeaderField: "User-Agent")
        if let authToken {
            req.setValue("Bearer \(authToken)", forHTTPHeaderField: "Authorization")
        }
        return req
    }

    private func send(_ request: URLRequest) async throws -> (Data, URLResponse) {
        do {
            return try await session.data(for: request)
        } catch {
            throw Error.transport(error)
        }
    }

    /// Path component encoding that leaves `/` intact (so `author/name`
    /// slugs become a single URL path segment) but URL-encodes other
    /// reserved characters.
    private func escapeSlugPath(_ slug: String) -> String {
        var allowed = CharacterSet.urlPathAllowed
        allowed.insert(charactersIn: "/")
        return slug.addingPercentEncoding(withAllowedCharacters: allowed) ?? slug
    }
}
