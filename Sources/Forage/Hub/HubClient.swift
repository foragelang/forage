import Foundation
#if canImport(FoundationNetworking)
import FoundationNetworking
#endif

/// Docker-style reference to a recipe in the hub. Parsed from the form
/// written after `import` in a recipe source file. Resolution rules mirror
/// Docker's image-reference grammar:
///
/// - `sweed` → official namespace on the default hub. Resolves to
///   `<defaultHub>/forage/sweed`.
/// - `alice/zen-leaf` → `alice`'s namespace on the default hub.
/// - `hub.example.com/team/scraper` → custom hub.
/// - `localhost:5000/me/test` → local hub on port 5000.
///
/// Detection rule: the first slash-separated component is treated as a
/// **registry hostname** if it contains `.`, contains `:`, or equals
/// `localhost` (case-insensitive). Otherwise it's a namespace, or — with no
/// slash — the bare name in the default namespace.
public struct HubRecipeRef: Sendable, Hashable, Codable {
    /// Original textual reference as written in the recipe source.
    public let raw: String
    public let version: Int?

    /// `nil` means "default hub" (whatever the `HubClient.baseURL` is).
    public let registry: String?
    /// `nil` means "default namespace" (only valid when `registry == nil`).
    public let namespace: String?
    public let name: String

    public init(
        raw: String,
        version: Int? = nil,
        registry: String? = nil,
        namespace: String? = nil,
        name: String
    ) {
        self.raw = raw
        self.version = version
        self.registry = registry
        self.namespace = namespace
        self.name = name
    }

    /// Parse a Docker-style reference. Throws on malformed input.
    public init(parsing raw: String, version: Int? = nil) throws {
        let trimmed = raw.trimmingCharacters(in: .whitespaces)
        if trimmed.isEmpty { throw ParseError.empty }

        let segments = trimmed.split(separator: "/", omittingEmptySubsequences: false).map(String.init)
        if segments.contains(where: { $0.isEmpty || $0 == "." || $0 == ".." }) {
            throw ParseError.invalidSegment(trimmed)
        }

        // Detect registry: first segment contains `.`, contains `:`, or is `localhost`.
        var rest = segments
        var registry: String? = nil
        if let first = rest.first, Self.looksLikeRegistry(first) {
            try Self.validateRegistry(first)
            registry = first
            rest.removeFirst()
        }

        // Validate the remaining segments — namespace and/or name.
        for s in rest {
            try Self.validateNameSegment(s)
        }

        switch rest.count {
        case 0:
            throw ParseError.missingName
        case 1:
            // Bare `name`. Only legal with no explicit registry — a registry
            // prefix needs at least `<registry>/<name>` (a registry without
            // a name is meaningless).
            if registry != nil {
                throw ParseError.missingName
            }
            self.init(raw: trimmed, version: version, registry: nil, namespace: nil, name: rest[0])
        case 2:
            self.init(raw: trimmed, version: version, registry: registry, namespace: rest[0], name: rest[1])
        default:
            throw ParseError.tooManySegments
        }
    }

    /// Effective namespace: explicit, or `forage` (the default).
    public var effectiveNamespace: String { namespace ?? "forage" }

    /// Path under `/v1/recipes/...`. Always `<namespace>/<name>`.
    public var slugPath: String { "\(effectiveNamespace)/\(name)" }

    /// Resolution rule: the registry segment determines the base URL. For
    /// the default registry (`registry == nil`), the caller's configured
    /// `HubClient.baseURL` wins. For `localhost[:port]`, we use `http://`.
    /// For other hosts we assume `https://`.
    public func resolveBaseURL(defaultBase: URL) -> URL {
        guard let r = registry else { return defaultBase }
        let lower = r.lowercased()
        let scheme = lower.hasPrefix("localhost") ? "http" : "https"
        // URL(string:) handles the optional port — `localhost:5000` becomes
        // host=localhost, port=5000.
        return URL(string: "\(scheme)://\(r)")!
    }

    public enum ParseError: Swift.Error, Equatable, CustomStringConvertible {
        case empty
        case invalidSegment(String)
        case invalidName(String)
        case invalidRegistry(String)
        case missingName
        case tooManySegments

        public var description: String {
            switch self {
            case .empty: return "import ref: empty"
            case .invalidSegment(let s): return "import ref: invalid segment '\(s)'"
            case .invalidName(let n): return "import ref: invalid name '\(n)' (expected ^[a-z0-9][a-z0-9-]{1,63}$)"
            case .invalidRegistry(let r): return "import ref: invalid registry '\(r)'"
            case .missingName: return "import ref: missing name component"
            case .tooManySegments: return "import ref: too many '/'-separated segments (expected at most <registry>/<namespace>/<name>)"
            }
        }
    }

    // MARK: - Internal helpers

    static func looksLikeRegistry(_ s: String) -> Bool {
        s.contains(".") || s.contains(":") || s.lowercased() == "localhost"
    }

    static func validateRegistry(_ s: String) throws {
        // Allow letters, digits, `-`, `.`, `_`, and a single `:port` suffix
        // where port is digits.
        let parts = s.split(separator: ":", maxSplits: 1, omittingEmptySubsequences: false).map(String.init)
        let host = parts[0]
        guard !host.isEmpty else { throw ParseError.invalidRegistry(s) }
        for c in host {
            let ok = c.isLetter || c.isNumber || c == "-" || c == "." || c == "_"
            if !ok { throw ParseError.invalidRegistry(s) }
        }
        if parts.count == 2 {
            let port = parts[1]
            if port.isEmpty || port.contains(where: { !$0.isNumber }) {
                throw ParseError.invalidRegistry(s)
            }
        }
    }

    static func validateNameSegment(_ s: String) throws {
        // Same shape the Worker enforces: ^[a-z0-9][a-z0-9-]{1,63}$.
        guard let first = s.first, (first.isLowercase && first.isLetter) || first.isNumber else {
            throw ParseError.invalidName(s)
        }
        for c in s {
            let ok = (c.isLowercase && c.isLetter) || c.isNumber || c == "-"
            if !ok { throw ParseError.invalidName(s) }
        }
        guard s.count >= 2, s.count <= 64 else { throw ParseError.invalidName(s) }
    }
}

/// Lightweight metadata returned by `list` / `versions`. Mirrors the hub's
/// `ListingItem` shape.
public struct HubRecipeMeta: Sendable, Hashable, Codable {
    /// `<namespace>/<name>` slug.
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

/// Request body for `publish`. Mirrors `hub-api`'s `PublishRequest`. The
/// `slug` field is `<namespace>/<name>`; the server splits it back on `/`
/// for storage.
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
        let req = try makeRequest(method: "GET", path: "/v1/health", overrideBase: nil)
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

        let req = try makeRequest(method: "GET", path: "/v1/recipes", query: items, overrideBase: nil)
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
            path: "/v1/recipes/\(escapeSlugPath(ref.slugPath))",
            query: items.isEmpty ? nil : items,
            overrideBase: ref.resolveBaseURL(defaultBase: baseURL)
        )
        let (data, resp) = try await send(req)
        guard let http = resp as? HTTPURLResponse else {
            throw Error.malformed(reason: "non-HTTP response")
        }
        if http.statusCode == 404 {
            throw Error.notFound(slug: ref.slugPath)
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

    /// Versions endpoint takes the same `<namespace>/<name>` slug shape.
    public func versions(slug: String) async throws -> [HubRecipeMeta] {
        let req = try makeRequest(
            method: "GET",
            path: "/v1/recipes/\(escapeSlugPath(slug))/versions",
            overrideBase: nil
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
        var req = try makeRequest(method: "POST", path: "/v1/recipes", authToken: token, overrideBase: nil)
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
        authToken: String? = nil,
        overrideBase: URL?
    ) throws -> URLRequest {
        let base = overrideBase ?? baseURL
        var comps = URLComponents()
        comps.scheme = base.scheme
        comps.host = base.host
        comps.port = base.port
        let basePath = base.path.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        let joined = basePath.isEmpty ? path : "/" + basePath + path
        comps.path = joined
        comps.queryItems = query
        guard let url = comps.url else {
            throw Error.malformed(reason: "could not assemble URL from \(base) + \(path)")
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

    /// Path component encoding that leaves `/` intact (so `<namespace>/<name>`
    /// becomes a single URL path segment) but URL-encodes other reserved
    /// characters.
    private func escapeSlugPath(_ slug: String) -> String {
        var allowed = CharacterSet.urlPathAllowed
        allowed.insert(charactersIn: "/")
        return slug.addingPercentEncoding(withAllowedCharacters: allowed) ?? slug
    }
}
