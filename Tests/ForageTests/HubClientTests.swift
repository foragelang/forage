import Testing
import Foundation
@testable import Forage

// Hub tests share a process-wide URLProtocol stub, so they run serially.
// `.serialized` makes Swift Testing one-at-a-time within this suite — the
// global stub registry is only mutated by one test at a time.
@Suite("HubClient", .serialized)
struct HubClientSuite {

    @Test
    func healthReturnsTrueOnOK() async throws {
        let session = makeMockSession()
        MockHubProtocol.register(.init(
            match: { $0.url?.path == "/v1/health" },
            body: #"{"status":"ok"}"#.data(using: .utf8)!,
            status: 200,
            headers: ["Content-Type": "application/json"]
        ))

        let client = HubClient(baseURL: testBase, session: session)
        let ok = try await client.health()
        #expect(ok == true)

        let observed = MockHubProtocol.requests()
        #expect(observed.count == 1)
        #expect(observed[0].url?.path == "/v1/health")
        #expect(observed[0].value(forHTTPHeaderField: "User-Agent")?.hasPrefix("forage/") == true)
    }

    @Test
    func listDecodesItems() async throws {
        let session = makeMockSession()
        let body = """
        {"items":[{"slug":"foo","author":null,"displayName":"Foo","summary":"hi","tags":["a"],"platform":null,"version":1,"sha256":"abc","createdAt":"2026-05-10T12:00:00Z","updatedAt":"2026-05-10T12:00:00Z"}],"nextCursor":null}
        """.data(using: .utf8)!
        MockHubProtocol.register(.init(
            match: { $0.url?.path == "/v1/recipes" },
            body: body,
            status: 200,
            headers: ["Content-Type": "application/json"]
        ))

        let client = HubClient(baseURL: testBase, session: session)
        let (items, next) = try await client.list(limit: 25)
        #expect(items.count == 1)
        #expect(items[0].slug == "foo")
        #expect(items[0].displayName == "Foo")
        #expect(items[0].tags == ["a"])
        #expect(items[0].version == 1)
        #expect(items[0].publishedAt != nil)
        #expect(next == nil)

        let observed = MockHubProtocol.requests()
        #expect(observed.count == 1)
        let queryItems = URLComponents(url: observed[0].url!, resolvingAgainstBaseURL: false)?.queryItems
        #expect(queryItems?.contains(where: { $0.name == "limit" && $0.value == "25" }) == true)
    }

    @Test
    func getDecodesRecipe() async throws {
        let session = makeMockSession()
        let body = """
        {"slug":"bar","author":"alice","displayName":"Bar","summary":"","tags":[],"platform":null,"version":3,"sha256":"deadbeef","createdAt":"2026-05-10T11:00:00Z","updatedAt":"2026-05-10T11:00:00Z","body":"recipe \\"bar\\" { engine http }"}
        """.data(using: .utf8)!
        MockHubProtocol.register(.init(
            match: { $0.url?.path == "/v1/recipes/bar" },
            body: body,
            status: 200,
            headers: ["Content-Type": "application/json"]
        ))

        let client = HubClient(baseURL: testBase, session: session)
        let recipe = try await client.get(HubRecipeRef(slug: "bar"))
        #expect(recipe.meta.slug == "bar")
        #expect(recipe.meta.author == "alice")
        #expect(recipe.meta.version == 3)
        #expect(recipe.body.contains("engine http"))
    }

    @Test
    func getEncodesAuthorSlashName() async throws {
        let session = makeMockSession()
        let body = """
        {"slug":"alice/awesome","author":"alice","displayName":"Awesome","summary":null,"tags":[],"platform":null,"version":1,"sha256":"x","createdAt":"2026-05-10T11:00:00Z","updatedAt":"2026-05-10T11:00:00Z","body":"recipe \\"awesome\\" { engine http }"}
        """.data(using: .utf8)!
        MockHubProtocol.register(.init(
            match: { $0.url?.path == "/v1/recipes/alice/awesome" },
            body: body,
            status: 200,
            headers: ["Content-Type": "application/json"]
        ))
        let client = HubClient(baseURL: testBase, session: session)
        let recipe = try await client.get(HubRecipeRef(slug: "alice/awesome"))
        #expect(recipe.meta.slug == "alice/awesome")

        let observed = MockHubProtocol.requests()
        #expect(observed.count == 1)
        #expect(observed[0].url?.path == "/v1/recipes/alice/awesome")
    }

    @Test
    func getForwardsVersionQueryParam() async throws {
        let session = makeMockSession()
        let body = """
        {"slug":"baz","author":null,"displayName":"Baz","summary":null,"tags":[],"platform":null,"version":2,"sha256":"x","createdAt":"2026-05-10T11:00:00Z","updatedAt":"2026-05-10T11:00:00Z","body":"recipe \\"baz\\" {}"}
        """.data(using: .utf8)!
        MockHubProtocol.register(.init(
            match: { $0.url?.path == "/v1/recipes/baz" },
            body: body,
            status: 200,
            headers: ["Content-Type": "application/json"]
        ))

        let client = HubClient(baseURL: testBase, session: session)
        _ = try await client.get(HubRecipeRef(slug: "baz", version: 2))
        let observed = MockHubProtocol.requests()
        let q = URLComponents(url: observed[0].url!, resolvingAgainstBaseURL: false)?.queryItems
        #expect(q?.contains(where: { $0.name == "version" && $0.value == "2" }) == true)
    }

    @Test
    func getThrowsNotFoundOn404() async throws {
        let session = makeMockSession()
        MockHubProtocol.register(.init(
            match: { _ in true },
            body: #"{"error":"not_found","message":"unknown slug"}"#.data(using: .utf8)!,
            status: 404,
            headers: ["Content-Type": "application/json"]
        ))
        let client = HubClient(baseURL: testBase, session: session)
        do {
            _ = try await client.get(HubRecipeRef(slug: "ghost"))
            Issue.record("expected notFound")
        } catch let e as HubClient.Error {
            if case .notFound(let slug) = e {
                #expect(slug == "ghost")
            } else {
                Issue.record("expected .notFound, got \(e)")
            }
        }
    }

    @Test
    func publishRequiresToken() async throws {
        let client = HubClient(baseURL: testBase, session: makeMockSession())
        do {
            _ = try await client.publish(HubPublishPayload(slug: "x", displayName: "X", body: "x"))
            Issue.record("expected missingToken")
        } catch let e as HubClient.Error {
            if case .missingToken = e {} else {
                Issue.record("expected .missingToken, got \(e)")
            }
        }
    }

    @Test
    func publishPostsExpectedShape() async throws {
        let session = makeMockSession()
        MockHubProtocol.register(.init(
            match: { $0.url?.path == "/v1/recipes" && $0.httpMethod == "POST" },
            body: #"{"slug":"foo","version":1,"sha256":"deadbeef"}"#.data(using: .utf8)!,
            status: 201,
            headers: ["Content-Type": "application/json"]
        ))

        let client = HubClient(baseURL: testBase, token: "test-token", session: session)
        let result = try await client.publish(
            HubPublishPayload(
                slug: "foo",
                author: "alice",
                displayName: "Foo",
                summary: "a thing",
                tags: ["dispensary"],
                body: "recipe \"foo\" {}",
                fixtures: nil,
                snapshot: nil
            )
        )

        #expect(result.slug == "foo")
        #expect(result.version == 1)
        #expect(result.sha256 == "deadbeef")

        let observed = MockHubProtocol.requests()
        #expect(observed.count == 1)
        let r = observed[0]
        #expect(r.httpMethod == "POST")
        #expect(r.value(forHTTPHeaderField: "Authorization") == "Bearer test-token")
        #expect(r.value(forHTTPHeaderField: "Content-Type") == "application/json")
        // URLSession may deliver the body via httpBody or httpBodyStream
        // depending on the SDK version — handle both.
        let body: Data
        if let direct = r.httpBody {
            body = direct
        } else if let stream = r.httpBodyStream {
            body = readStream(stream)
        } else {
            body = Data()
        }
        let json = try JSONSerialization.jsonObject(with: body) as! [String: Any]
        #expect(json["slug"] as? String == "foo")
        #expect(json["author"] as? String == "alice")
        #expect(json["displayName"] as? String == "Foo")
        #expect(json["summary"] as? String == "a thing")
        #expect((json["tags"] as? [String])?.first == "dispensary")
        #expect((json["body"] as? String)?.contains("recipe \"foo\"") == true)
    }

    @Test
    func surfacesHTTPErrors() async throws {
        let session = makeMockSession()
        MockHubProtocol.register(.init(
            match: { _ in true },
            body: #"{"error":"server","message":"boom"}"#.data(using: .utf8)!,
            status: 500,
            headers: ["Content-Type": "application/json"]
        ))
        let client = HubClient(baseURL: testBase, session: session)
        do {
            _ = try await client.health()
            Issue.record("expected http error")
        } catch let e as HubClient.Error {
            if case .http(let status, _) = e {
                #expect(status == 500)
            } else {
                Issue.record("expected .http, got \(e)")
            }
        }
    }

    @Test
    func hubRecipeRefParsesValidSlugs() {
        #expect(HubRecipeRef.parse("foo") == HubRecipeRef(slug: "foo"))
        #expect(HubRecipeRef.parse("foo-bar") == HubRecipeRef(slug: "foo-bar"))
        #expect(HubRecipeRef.parse("alice/awesome-recipe") == HubRecipeRef(slug: "alice/awesome-recipe"))
        #expect(HubRecipeRef.parse("") == nil)
        #expect(HubRecipeRef.parse("a/b/c") == nil)
        #expect(HubRecipeRef.parse("UPPER") != nil) // letters allowed (server lowercases its own check)
    }

    // MARK: - RecipeImporter

    @Test
    func importerFlattensSingleImport() async throws {
        let session = makeMockSession()
        let importedRecipe = """
        recipe "common" {
            engine http
            type Item { id: String }
        }
        """
        registerHubRecipeStub(slug: "common", body: importedRecipe)

        let client = HubClient(baseURL: testBase, session: session)
        let importer = RecipeImporter(client: client)

        let rootSrc = """
        import hub://common

        recipe "uses-common" {
            engine http
            type Local { id: String }
            emit Local { id ← "x" }
        }
        """
        let root = try Parser.parse(source: rootSrc)
        let flat = try await importer.flatten(root)

        #expect(flat.imports.isEmpty)
        let typeNames = Set(flat.types.map(\.name))
        #expect(typeNames == ["Local", "Item"])
    }

    @Test
    func importerLetsRootOverrideImportedType() async throws {
        let session = makeMockSession()
        let imported = """
        recipe "lib" {
            engine http
            type Conflict { id: String }
        }
        """
        registerHubRecipeStub(slug: "lib", body: imported)

        let client = HubClient(baseURL: testBase, session: session)
        let importer = RecipeImporter(client: client)

        let rootOK = try Parser.parse(source: """
        import hub://lib

        recipe "r" {
            engine http
            type Conflict { id: String; extra: String? }
            emit Conflict { id ← "1" }
        }
        """)
        let flat = try await importer.flatten(rootOK)
        #expect(flat.types.first(where: { $0.name == "Conflict" })?.fields.count == 2)
    }

    @Test
    func importerRaisesWhenHubFetchFails() async throws {
        let session = makeMockSession()
        MockHubProtocol.register(.init(
            match: { _ in true },
            body: Data(),
            status: 404,
            headers: [:]
        ))
        let client = HubClient(baseURL: testBase, session: session)
        let importer = RecipeImporter(client: client)

        let root = try Parser.parse(source: """
        import hub://does-not-exist

        recipe "r" { engine http }
        """)
        do {
            _ = try await importer.flatten(root)
            Issue.record("expected error")
        } catch let e as RecipeImporter.Error {
            if case .hub = e {} else { Issue.record("expected .hub error, got \(e)") }
        }
    }

    @Test
    func importerDetectsCycle() async throws {
        let session = makeMockSession()
        registerHubRecipeStub(slug: "a", body: """
        import hub://b

        recipe "a" { engine http }
        """)
        registerHubRecipeStub(slug: "b", body: """
        import hub://a

        recipe "b" { engine http }
        """)
        let client = HubClient(baseURL: testBase, session: session)
        let importer = RecipeImporter(client: client)

        let root = try Parser.parse(source: """
        import hub://a

        recipe "root" { engine http }
        """)
        do {
            _ = try await importer.flatten(root)
            Issue.record("expected cycle error")
        } catch let e as RecipeImporter.Error {
            if case .cycle = e {} else { Issue.record("expected .cycle, got \(e)") }
        }
    }
}

// MARK: - URLProtocol-based stub session — shared with RecipeImporterTests

/// Stubs `URLSession` for tests. Register a handler with `MockHubProtocol.register`;
/// each request is matched in order, the first matching closure provides the
/// (data, status, headers) tuple, and the request is appended to `requests` for
/// post-assertions.
final class MockHubProtocol: URLProtocol, @unchecked Sendable {
    struct Stub: Sendable {
        let match: @Sendable (URLRequest) -> Bool
        let body: Data
        let status: Int
        let headers: [String: String]
    }

    // Process-wide test state. Suite-level `.serialized` keeps mutation
    // sequential; the lock is belt-and-suspenders.
    private static let lock = NSLock()
    nonisolated(unsafe) private static var stubs: [Stub] = []
    nonisolated(unsafe) private static var observed: [URLRequest] = []

    static func reset() {
        lock.lock(); defer { lock.unlock() }
        stubs.removeAll(); observed.removeAll()
    }

    static func register(_ stub: Stub) {
        lock.lock(); defer { lock.unlock() }
        stubs.append(stub)
    }

    static func requests() -> [URLRequest] {
        lock.lock(); defer { lock.unlock() }
        return observed
    }

    override class func canInit(with _: URLRequest) -> Bool { true }
    override class func canonicalRequest(for r: URLRequest) -> URLRequest { r }

    override func startLoading() {
        Self.lock.lock()
        Self.observed.append(request)
        let match = Self.stubs.first(where: { $0.match(request) })
        Self.lock.unlock()

        guard let match else {
            client?.urlProtocol(self, didFailWithError: NSError(domain: "MockHub", code: 404,
                userInfo: [NSLocalizedDescriptionKey: "no stub for \(request.url?.absoluteString ?? "?")"]))
            return
        }

        let response = HTTPURLResponse(
            url: request.url!,
            statusCode: match.status,
            httpVersion: "HTTP/1.1",
            headerFields: match.headers
        )!
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: match.body)
        client?.urlProtocolDidFinishLoading(self)
    }

    override func stopLoading() {}
}

func makeMockSession() -> URLSession {
    MockHubProtocol.reset()
    let cfg = URLSessionConfiguration.ephemeral
    cfg.protocolClasses = [MockHubProtocol.self]
    return URLSession(configuration: cfg)
}

let testBase = URL(string: "https://hub.test")!

// MARK: - Helpers

func readStream(_ stream: InputStream) -> Data {
    stream.open()
    defer { stream.close() }
    var data = Data()
    let bufSize = 4096
    var buf = [UInt8](repeating: 0, count: bufSize)
    while stream.hasBytesAvailable {
        let read = stream.read(&buf, maxLength: bufSize)
        if read <= 0 { break }
        data.append(buf, count: read)
    }
    return data
}

/// Convenience for tests that need to serve a single recipe body via the
/// MockHubProtocol stub. Crafts a `RecipeDetailResponse`-shaped JSON
/// payload (metadata + `body`) and registers a 200 OK on
/// `/v1/recipes/<slug>`.
func registerHubRecipeStub(slug: String, body source: String) {
    let payload: [String: Any] = [
        "slug": slug,
        "author": NSNull(),
        "displayName": slug,
        "summary": NSNull(),
        "tags": [],
        "platform": NSNull(),
        "version": 1,
        "sha256": "abc",
        "createdAt": "2026-05-10T11:00:00Z",
        "updatedAt": "2026-05-10T11:00:00Z",
        "body": source,
    ]
    let data = try! JSONSerialization.data(withJSONObject: payload)
    MockHubProtocol.register(.init(
        match: { $0.url?.path == "/v1/recipes/\(slug)" },
        body: data,
        status: 200,
        headers: ["Content-Type": "application/json"]
    ))
}
