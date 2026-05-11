import Testing
import Foundation
@testable import Forage

// MARK: - Stateful mock transport

/// Mock transport built for session-auth tests. Each registered handler is a
/// closure that gets to see every incoming request and the call counter for
/// its slot — enough to fake "first call 401, second call 200", "respond
/// only when the right Cookie header is present", etc.
actor SessionMockTransport: Transport {
    typealias Handler = @Sendable (URLRequest, Int) -> (Data, Int, [String: String])

    struct Slot {
        let matcher: @Sendable (URLRequest) -> Bool
        let handler: Handler
    }

    private(set) var slots: [Slot] = []
    private(set) var calls: [URLRequest] = []
    private(set) var slotHits: [Int] = []   // parallel to slots

    func register(
        urlSubstring: String,
        handler: @escaping Handler
    ) {
        slots.append(Slot(
            matcher: { ($0.url?.absoluteString ?? "").contains(urlSubstring) },
            handler: handler
        ))
        slotHits.append(0)
    }

    func register(
        match: @Sendable @escaping (URLRequest) -> Bool,
        handler: @escaping Handler
    ) {
        slots.append(Slot(matcher: match, handler: handler))
        slotHits.append(0)
    }

    func reset() {
        calls.removeAll()
        for i in slotHits.indices { slotHits[i] = 0 }
    }

    nonisolated func send(_ request: URLRequest) async throws -> (Data, HTTPURLResponse) {
        return try await self.handle(request)
    }

    private func handle(_ request: URLRequest) throws -> (Data, HTTPURLResponse) {
        calls.append(request)
        for (i, slot) in slots.enumerated() where slot.matcher(request) {
            let nth = slotHits[i]
            slotHits[i] = nth + 1
            let (data, status, headers) = slot.handler(request, nth)
            let response = HTTPURLResponse(
                url: request.url!,
                statusCode: status,
                httpVersion: "HTTP/1.1",
                headerFields: headers
            )!
            return (data, response)
        }
        throw NSError(domain: "SessionMockTransport", code: 404, userInfo: [
            NSLocalizedDescriptionKey: "no match for \(request.url?.absoluteString ?? "?")"
        ])
    }
}

// MARK: - Recipe builders

/// A formLogin recipe: posts username/password to /login, captures cookies,
/// then GETs /items which emits one record per item.
private func formLoginRecipe(maxReauthRetries: Int = 1, cache: TimeInterval? = nil) -> Recipe {
    Recipe(
        name: "form-login",
        engineKind: .http,
        types: [
            RecipeType(name: "Item", fields: [
                RecipeField(name: "id", type: .string, optional: false),
            ]),
        ],
        auth: .session(SessionAuth(
            kind: .formLogin(FormLogin(
                url: Template(literal: "https://example.com/login"),
                method: "POST",
                body: .form([
                    ("username", .path(.secret("username"))),
                    ("password", .path(.secret("password"))),
                ]),
                captureCookies: true
            )),
            maxReauthRetries: maxReauthRetries,
            cacheDuration: cache
        )),
        body: [
            .step(HTTPStep(
                name: "items",
                request: HTTPRequest(method: "GET", url: Template(literal: "https://example.com/items"))
            )),
            .forLoop(
                variable: "it",
                collection: .field(.variable("items"), "data"),
                body: [
                    .emit(Emission(
                        typeName: "Item",
                        bindings: [
                            FieldBinding(fieldName: "id", expr: .path(.field(.variable("it"), "id"))),
                        ]
                    ))
                ]
            ),
        ],
        secrets: ["username", "password"]
    )
}

/// A bearerLogin recipe: posts credentials to /token, extracts $.access_token,
/// then GETs /items with Authorization header.
private func bearerLoginRecipe(maxReauthRetries: Int = 1) -> Recipe {
    Recipe(
        name: "bearer-login",
        engineKind: .http,
        types: [
            RecipeType(name: "Item", fields: [
                RecipeField(name: "id", type: .string, optional: false),
            ]),
        ],
        auth: .session(SessionAuth(
            kind: .bearerLogin(BearerLogin(
                url: Template(literal: "https://example.com/token"),
                method: "POST",
                body: .jsonObject([
                    HTTPBodyKV(key: "client_id", value: .path(.secret("clientId"))),
                    HTTPBodyKV(key: "client_secret", value: .path(.secret("clientSecret"))),
                ]),
                tokenPath: .field(.current, "access_token")
            )),
            maxReauthRetries: maxReauthRetries
        )),
        body: [
            .step(HTTPStep(
                name: "items",
                request: HTTPRequest(method: "GET", url: Template(literal: "https://example.com/items"))
            )),
            .forLoop(
                variable: "it",
                collection: .field(.variable("items"), "data"),
                body: [
                    .emit(Emission(
                        typeName: "Item",
                        bindings: [
                            FieldBinding(fieldName: "id", expr: .path(.field(.variable("it"), "id"))),
                        ]
                    ))
                ]
            ),
        ],
        secrets: ["clientId", "clientSecret"]
    )
}

// MARK: - formLogin: cookie capture + thread-through

@Test
func formLoginCapturesCookieAndThreadsItIntoSubsequentRequests() async throws {
    let transport = SessionMockTransport()
    let itemsBody = try JSONSerialization.data(
        withJSONObject: ["data": [["id": "1"], ["id": "2"]]],
        options: [.fragmentsAllowed]
    )
    await transport.register(urlSubstring: "/login") { _, _ in
        // Server sets `session=abc123` on Set-Cookie.
        (Data("ok".utf8), 200, [
            "Set-Cookie": "session=abc123; Path=/; Domain=example.com",
            "Content-Type": "text/plain",
        ])
    }
    await transport.register(urlSubstring: "/items") { req, _ in
        // Items endpoint requires the cookie. If missing, return 401.
        let cookie = req.value(forHTTPHeaderField: "Cookie") ?? ""
        if cookie.contains("session=abc123") {
            return (itemsBody, 200, ["Content-Type": "application/json"])
        }
        return (Data("no cookie".utf8), 401, [:])
    }
    let client = HTTPClient(transport: transport, minRequestInterval: 0, maxRetries: 0)
    let runner = RecipeRunner(
        httpClient: client,
        secretResolver: DictionarySecretResolver(["username": "alice", "password": "hunter2"])
    )

    let result = try await runner.run(recipe: formLoginRecipe(), inputs: [:])
    #expect(result.report.stallReason == "completed")
    #expect(result.snapshot.records(of: "Item").count == 2)

    let calls = await transport.calls
    // Two requests: /login, /items.
    #expect(calls.count == 2)
    #expect(calls[0].url?.absoluteString == "https://example.com/login")
    #expect(calls[1].value(forHTTPHeaderField: "Cookie")?.contains("session=abc123") == true)
}

// MARK: - bearerLogin: token captured + Authorization injected

@Test
func bearerLoginInjectsAuthorizationHeader() async throws {
    let transport = SessionMockTransport()
    let tokenBody = try JSONSerialization.data(
        withJSONObject: ["access_token": "tok_xyz", "token_type": "Bearer"],
        options: [.fragmentsAllowed]
    )
    let itemsBody = try JSONSerialization.data(
        withJSONObject: ["data": [["id": "a"]]],
        options: [.fragmentsAllowed]
    )
    await transport.register(urlSubstring: "/token") { _, _ in
        (tokenBody, 200, ["Content-Type": "application/json"])
    }
    await transport.register(urlSubstring: "/items") { req, _ in
        let auth = req.value(forHTTPHeaderField: "Authorization") ?? ""
        if auth == "Bearer tok_xyz" {
            return (itemsBody, 200, ["Content-Type": "application/json"])
        }
        return (Data("bad auth".utf8), 401, [:])
    }
    let client = HTTPClient(transport: transport, minRequestInterval: 0, maxRetries: 0)
    let runner = RecipeRunner(
        httpClient: client,
        secretResolver: DictionarySecretResolver([
            "clientId": "cid", "clientSecret": "csec",
        ])
    )

    let result = try await runner.run(recipe: bearerLoginRecipe(), inputs: [:])
    #expect(result.report.stallReason == "completed")
    #expect(result.snapshot.records(of: "Item").count == 1)
}

// MARK: - 401 mid-run triggers exactly one re-auth + retry

@Test
func midRunUnauthorizedTriggersOneReauthThenSucceeds() async throws {
    let transport = SessionMockTransport()
    let itemsBody = try JSONSerialization.data(
        withJSONObject: ["data": [["id": "x"]]],
        options: [.fragmentsAllowed]
    )
    // Login returns a different cookie each time so we can confirm the
    // second login happened.
    await transport.register(urlSubstring: "/login") { _, n in
        let cookie = (n == 0) ? "session=first" : "session=second"
        return (Data("ok".utf8), 200, ["Set-Cookie": "\(cookie); Path=/; Domain=example.com"])
    }
    await transport.register(urlSubstring: "/items") { req, n in
        let cookie = req.value(forHTTPHeaderField: "Cookie") ?? ""
        if n == 0 {
            // First /items call returns 401 even though we have a cookie:
            // simulates a session that just expired upstream.
            return (Data("expired".utf8), 401, [:])
        }
        // Second /items call should carry the second cookie.
        if cookie.contains("session=second") {
            return (itemsBody, 200, ["Content-Type": "application/json"])
        }
        return (Data("still bad".utf8), 401, [:])
    }
    let client = HTTPClient(transport: transport, minRequestInterval: 0, maxRetries: 0)
    let runner = RecipeRunner(
        httpClient: client,
        secretResolver: DictionarySecretResolver(["username": "a", "password": "b"])
    )

    let result = try await runner.run(recipe: formLoginRecipe(maxReauthRetries: 1), inputs: [:])
    #expect(result.report.stallReason == "completed")
    #expect(result.snapshot.records(of: "Item").count == 1)
    let calls = await transport.calls
    // login, items(401), login, items(200) = 4
    #expect(calls.count == 4)
    #expect(calls[0].url?.absoluteString == "https://example.com/login")
    #expect(calls[1].url?.absoluteString == "https://example.com/items")
    #expect(calls[2].url?.absoluteString == "https://example.com/login")
    #expect(calls[3].url?.absoluteString == "https://example.com/items")
}

// MARK: - Two 401s in a row → auth-failed

@Test
func twoUnauthorizedRespsesSurfaceAuthFailed() async throws {
    let transport = SessionMockTransport()
    await transport.register(urlSubstring: "/login") { _, _ in
        (Data("ok".utf8), 200, ["Set-Cookie": "session=x; Path=/"])
    }
    await transport.register(urlSubstring: "/items") { _, _ in
        // Always 401.
        (Data("nope".utf8), 401, [:])
    }
    let client = HTTPClient(transport: transport, minRequestInterval: 0, maxRetries: 0)
    let runner = RecipeRunner(
        httpClient: client,
        secretResolver: DictionarySecretResolver(["username": "a", "password": "b"])
    )

    let result = try await runner.run(recipe: formLoginRecipe(maxReauthRetries: 1), inputs: [:])
    #expect(result.report.stallReason.hasPrefix("auth-failed:"))
    #expect(result.snapshot.records.isEmpty)
}

// MARK: - Bad credentials at login → auth-failed

@Test
func badCredentialsAtLoginSurfaceAuthFailed() async throws {
    let transport = SessionMockTransport()
    await transport.register(urlSubstring: "/login") { _, _ in
        // 401 right on the login endpoint.
        (Data("invalid".utf8), 401, [:])
    }
    let client = HTTPClient(transport: transport, minRequestInterval: 0, maxRetries: 0)
    let runner = RecipeRunner(
        httpClient: client,
        secretResolver: DictionarySecretResolver(["username": "a", "password": "wrong"])
    )

    let result = try await runner.run(recipe: formLoginRecipe(), inputs: [:])
    #expect(result.report.stallReason.hasPrefix("auth-failed:"))
}

// MARK: - MFA hook fires and the code reaches the login body

@Test
func mfaProviderIsCalledAndCodeIsAppendedToLoginBody() async throws {
    // Recipe declares requiresMFA: true. Engine must call the provider and
    // include the resulting code in the form body.
    let recipe = Recipe(
        name: "mfa-login",
        engineKind: .http,
        types: [RecipeType(name: "Item", fields: [RecipeField(name: "id", type: .string, optional: false)])],
        auth: .session(SessionAuth(
            kind: .formLogin(FormLogin(
                url: Template(literal: "https://example.com/login"),
                body: .form([
                    ("username", .path(.secret("username"))),
                    ("password", .path(.secret("password"))),
                ])
            )),
            requiresMFA: true
        )),
        body: [
            .step(HTTPStep(
                name: "items",
                request: HTTPRequest(method: "GET", url: Template(literal: "https://example.com/items"))
            )),
        ],
        secrets: ["username", "password"]
    )

    let transport = SessionMockTransport()
    let itemsBody = try JSONSerialization.data(withJSONObject: ["data": []], options: [.fragmentsAllowed])
    await transport.register(urlSubstring: "/login") { req, _ in
        let body = req.httpBody.flatMap { String(data: $0, encoding: .utf8) } ?? ""
        // Body must include the mfa code at the default field name "code".
        if body.contains("code=123456") {
            return (Data("ok".utf8), 200, ["Set-Cookie": "session=ok; Path=/"])
        }
        return (Data("missing code".utf8), 401, [:])
    }
    await transport.register(urlSubstring: "/items") { _, _ in
        (itemsBody, 200, ["Content-Type": "application/json"])
    }
    let client = HTTPClient(transport: transport, minRequestInterval: 0, maxRetries: 0)
    let runner = RecipeRunner(
        httpClient: client,
        secretResolver: DictionarySecretResolver(["username": "u", "password": "p"]),
        mfaProvider: StaticMFAProvider(code: "123456")
    )

    let result = try await runner.run(recipe: recipe, inputs: [:])
    #expect(result.report.stallReason == "completed")
}

// MARK: - Secret resolution: dictionary, env, missing

@Test
func dictionarySecretResolverResolves() async throws {
    let r = DictionarySecretResolver(["x": "v"])
    let v = try await r.resolve("x")
    #expect(v == "v")
    await #expect(throws: (any Error).self) {
        _ = try await r.resolve("missing")
    }
}

@Test
func missingSecretSurfacesAuthSecretMissingStall() async throws {
    let transport = SessionMockTransport()
    // No /login handler — we shouldn't even get that far.
    let client = HTTPClient(transport: transport, minRequestInterval: 0, maxRetries: 0)
    let runner = RecipeRunner(
        httpClient: client,
        secretResolver: DictionarySecretResolver(["password": "p"]) // no "username"
    )
    let result = try await runner.run(recipe: formLoginRecipe(), inputs: [:])
    #expect(result.report.stallReason.hasPrefix("auth-secret-missing:"))
}

@Test
func recipeReferencingSecretsWithoutResolverFailsWithSecretMissing() async throws {
    let transport = SessionMockTransport()
    let client = HTTPClient(transport: transport, minRequestInterval: 0, maxRetries: 0)
    // No secretResolver at all.
    let runner = RecipeRunner(httpClient: client)
    let result = try await runner.run(recipe: formLoginRecipe(), inputs: [:])
    #expect(result.report.stallReason.hasPrefix("auth-secret-missing:"))
}

// MARK: - Cache: write + read + expiry

@Test
func sessionCacheIsWrittenWithRestrictivePermissions() async throws {
    let tmpRoot = URL(fileURLWithPath: NSTemporaryDirectory())
        .appendingPathComponent("forage-session-cache-\(UUID().uuidString)", isDirectory: true)
    defer { try? FileManager.default.removeItem(at: tmpRoot) }

    let transport = SessionMockTransport()
    let itemsBody = try JSONSerialization.data(withJSONObject: ["data": [["id": "1"]]], options: [.fragmentsAllowed])
    await transport.register(urlSubstring: "/login") { _, _ in
        (Data("ok".utf8), 200, ["Set-Cookie": "session=cached; Path=/; Domain=example.com"])
    }
    await transport.register(urlSubstring: "/items") { _, _ in
        (itemsBody, 200, ["Content-Type": "application/json"])
    }
    let client = HTTPClient(transport: transport, minRequestInterval: 0, maxRetries: 0)
    let runner = RecipeRunner(
        httpClient: client,
        secretResolver: DictionarySecretResolver(["username": "u", "password": "p"]),
        sessionCacheRoot: tmpRoot
    )

    _ = try await runner.run(recipe: formLoginRecipe(cache: 3600), inputs: [:])

    // Locate the cache file. Filename is the credential fingerprint;
    // directory is `sessions/<recipe-name>/`.
    let dir = tmpRoot.appendingPathComponent("sessions").appendingPathComponent("form-login")
    let urls = try FileManager.default.contentsOfDirectory(at: dir, includingPropertiesForKeys: nil)
    let file = urls.first(where: { $0.pathExtension == "json" })
    #expect(file != nil)

    let attrs = try FileManager.default.attributesOfItem(atPath: file!.path)
    let perms = (attrs[.posixPermissions] as? NSNumber)?.intValue ?? 0
    // 0o600 == 384
    #expect(perms == 0o600)
}

@Test
func sessionCacheHitSkipsLoginOnSubsequentRun() async throws {
    let tmpRoot = URL(fileURLWithPath: NSTemporaryDirectory())
        .appendingPathComponent("forage-session-cache-\(UUID().uuidString)", isDirectory: true)
    defer { try? FileManager.default.removeItem(at: tmpRoot) }

    let transport = SessionMockTransport()
    let itemsBody = try JSONSerialization.data(withJSONObject: ["data": [["id": "1"]]], options: [.fragmentsAllowed])
    await transport.register(urlSubstring: "/login") { _, _ in
        (Data("ok".utf8), 200, ["Set-Cookie": "session=cached; Path=/; Domain=example.com"])
    }
    await transport.register(urlSubstring: "/items") { _, _ in
        (itemsBody, 200, ["Content-Type": "application/json"])
    }
    let client = HTTPClient(transport: transport, minRequestInterval: 0, maxRetries: 0)
    let runner = RecipeRunner(
        httpClient: client,
        secretResolver: DictionarySecretResolver(["username": "u", "password": "p"]),
        sessionCacheRoot: tmpRoot
    )

    // First run: should hit /login once + /items once.
    _ = try await runner.run(recipe: formLoginRecipe(cache: 3600), inputs: [:])
    let firstCalls = await transport.calls.count
    #expect(firstCalls == 2)

    // Second run: cache is fresh, /login should not be hit.
    let secondResult = try await runner.run(recipe: formLoginRecipe(cache: 3600), inputs: [:])
    let secondCallsTotal = await transport.calls.count
    #expect(secondResult.report.stallReason == "completed")
    // Second run only added /items, not /login.
    #expect(secondCallsTotal == 3)
    let calls = await transport.calls
    #expect(calls[2].url?.absoluteString == "https://example.com/items")
}

@Test
func expiredSessionCacheCausesRelogin() async throws {
    let tmpRoot = URL(fileURLWithPath: NSTemporaryDirectory())
        .appendingPathComponent("forage-session-cache-\(UUID().uuidString)", isDirectory: true)
    try FileManager.default.createDirectory(at: tmpRoot, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: tmpRoot) }

    // Pre-seed a stale cache directly to avoid timing-sensitive test scaffolding.
    let recipe = formLoginRecipe(cache: 60)
    let fp = CredentialFingerprint.compute(secrets: ["username": "u", "password": "p"])
    let cacheURL = SessionCache.cacheFile(for: recipe.name, fingerprint: fp, root: tmpRoot)
    let stale = SessionState(
        createdAt: Date().addingTimeInterval(-3600),  // 1h old, max age 60s → expired
        payload: .cookies([SessionCookie(name: "session", value: "stale", domain: "example.com", path: "/")])
    )
    try SessionCache.write(stale, to: cacheURL)

    let transport = SessionMockTransport()
    let itemsBody = try JSONSerialization.data(withJSONObject: ["data": [["id": "ok"]]], options: [.fragmentsAllowed])
    await transport.register(urlSubstring: "/login") { _, _ in
        (Data("ok".utf8), 200, ["Set-Cookie": "session=fresh; Path=/; Domain=example.com"])
    }
    await transport.register(urlSubstring: "/items") { _, _ in
        (itemsBody, 200, ["Content-Type": "application/json"])
    }
    let client = HTTPClient(transport: transport, minRequestInterval: 0, maxRetries: 0)
    let runner = RecipeRunner(
        httpClient: client,
        secretResolver: DictionarySecretResolver(["username": "u", "password": "p"]),
        sessionCacheRoot: tmpRoot
    )

    let result = try await runner.run(recipe: recipe, inputs: [:])
    #expect(result.report.stallReason == "completed")
    // Login should have been hit because cache was stale.
    let calls = await transport.calls
    #expect(calls.count == 2)
    #expect(calls[0].url?.absoluteString == "https://example.com/login")
}

// MARK: - cookiePersist loads cookies from a JSON file

@Test
func cookiePersistLoadsCookiesFromJSONFile() async throws {
    let tmp = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("cookies-\(UUID().uuidString).json")
    let cookies: [SessionCookie] = [
        SessionCookie(name: "session", value: "from-disk", domain: "example.com", path: "/"),
    ]
    let data = try JSONEncoder().encode(cookies)
    try data.write(to: tmp)
    defer { try? FileManager.default.removeItem(at: tmp) }

    let recipe = Recipe(
        name: "cookie-persist",
        engineKind: .http,
        types: [RecipeType(name: "Item", fields: [RecipeField(name: "id", type: .string, optional: false)])],
        auth: .session(SessionAuth(
            kind: .cookiePersist(CookiePersist(
                sourcePath: Template(literal: tmp.path),
                format: .json
            ))
        )),
        body: [
            .step(HTTPStep(
                name: "items",
                request: HTTPRequest(method: "GET", url: Template(literal: "https://example.com/items"))
            )),
            .forLoop(
                variable: "it",
                collection: .field(.variable("items"), "data"),
                body: [
                    .emit(Emission(
                        typeName: "Item",
                        bindings: [
                            FieldBinding(fieldName: "id", expr: .path(.field(.variable("it"), "id"))),
                        ]
                    ))
                ]
            ),
        ]
    )

    let transport = SessionMockTransport()
    let itemsBody = try JSONSerialization.data(withJSONObject: ["data": [["id": "a"]]], options: [.fragmentsAllowed])
    await transport.register(urlSubstring: "/items") { req, _ in
        let cookie = req.value(forHTTPHeaderField: "Cookie") ?? ""
        if cookie.contains("session=from-disk") {
            return (itemsBody, 200, ["Content-Type": "application/json"])
        }
        return (Data("no cookie".utf8), 401, [:])
    }
    let client = HTTPClient(transport: transport, minRequestInterval: 0, maxRetries: 0)
    let runner = RecipeRunner(httpClient: client)

    let result = try await runner.run(recipe: recipe, inputs: [:])
    #expect(result.report.stallReason == "completed")
    #expect(result.snapshot.records(of: "Item").count == 1)
}

// MARK: - Parser surface

@Test
func parserAcceptsAuthSessionFormLogin() throws {
    let src = """
    recipe "p" {
        engine http
        secret username
        secret password
        type Item { id: String }
        auth.session.formLogin {
            url: "https://example.com/login"
            method: "POST"
            body.form {
                "username": $secret.username
                "password": $secret.password
            }
            captureCookies: true
            maxReauthRetries: 2
            cache: 3600
        }
        step items { method "GET"; url "https://example.com/items" }
        for $it in $items {
            emit Item { id ← $it.id }
        }
    }
    """
    let recipe = try Parser.parse(source: src)
    guard case .session(let s) = recipe.auth else { Issue.record("expected session auth"); return }
    #expect(s.maxReauthRetries == 2)
    #expect(s.cacheDuration == 3600)
    if case .formLogin(let f) = s.kind {
        // captureCookies parsed
        #expect(f.captureCookies == true)
        #expect(f.method == "POST")
    } else {
        Issue.record("expected formLogin variant")
    }
    #expect(recipe.secrets == ["username", "password"])
}

@Test
func parserAcceptsAuthSessionBearerLogin() throws {
    let src = """
    recipe "p" {
        engine http
        secret clientId
        secret clientSecret
        type Item { id: String }
        auth.session.bearerLogin {
            url: "https://example.com/token"
            body.json {
                client_id: $secret.clientId
                client_secret: $secret.clientSecret
            }
            tokenPath: $.access_token
            headerPrefix: "Bearer "
        }
        step items { method "GET"; url "https://example.com/items" }
        for $it in $items {
            emit Item { id ← $it.id }
        }
    }
    """
    let recipe = try Parser.parse(source: src)
    guard case .session(let s) = recipe.auth else { Issue.record("expected session auth"); return }
    if case .bearerLogin(let b) = s.kind {
        #expect(b.headerName == "Authorization")
        #expect(b.headerPrefix == "Bearer ")
    } else {
        Issue.record("expected bearerLogin variant")
    }
}

@Test
func parserAcceptsAuthSessionCookiePersist() throws {
    let src = """
    recipe "p" {
        engine http
        secret cookieFile
        type Item { id: String }
        auth.session.cookiePersist {
            sourcePath: "{$secret.cookieFile}"
            format: netscape
        }
        step items { method "GET"; url "https://example.com/items" }
        for $it in $items {
            emit Item { id ← $it.id }
        }
    }
    """
    let recipe = try Parser.parse(source: src)
    guard case .session(let s) = recipe.auth else { Issue.record("expected session auth"); return }
    if case .cookiePersist(let c) = s.kind {
        #expect(c.format == .netscape)
    } else {
        Issue.record("expected cookiePersist variant")
    }
}

// MARK: - Validator

@Test
func validatorWarnsOnUndeclaredSecretReference() {
    let recipe = Recipe(
        name: "v",
        engineKind: .http,
        types: [RecipeType(name: "Item", fields: [RecipeField(name: "id", type: .string, optional: false)])],
        auth: .session(SessionAuth(
            kind: .formLogin(FormLogin(
                url: Template(literal: "https://example.com/login"),
                body: .form([
                    ("username", .path(.secret("typoeduser"))),
                ])
            ))
        )),
        body: [
            .step(HTTPStep(
                name: "items",
                request: HTTPRequest(method: "GET", url: Template(literal: "https://example.com/items"))
            )),
        ]
        // intentionally no `secrets:` declaration
    )
    let issues = Validator.validate(recipe)
    let warnings = issues.warnings.map(\.message)
    #expect(warnings.contains(where: { $0.contains("typoeduser") && $0.contains("not declared") }))
}

@Test
func validatorWarnsOnUnusedSecretDeclaration() {
    let recipe = Recipe(
        name: "v",
        engineKind: .http,
        types: [RecipeType(name: "Item", fields: [RecipeField(name: "id", type: .string, optional: false)])],
        body: [
            .step(HTTPStep(
                name: "items",
                request: HTTPRequest(method: "GET", url: Template(literal: "https://example.com/items"))
            )),
        ],
        secrets: ["unused"]
    )
    let issues = Validator.validate(recipe)
    let warnings = issues.warnings.map(\.message)
    #expect(warnings.contains(where: { $0.contains("unused") && $0.contains("never referenced") }))
}

// MARK: - Diagnostic redaction

@Test
func resolvedSecretValuesAreRedactedFromStallReason() async throws {
    // Build a recipe whose first request throws a non-cancellation error
    // with the secret value mentioned in the underlying error description.
    let recipe = Recipe(
        name: "redact",
        engineKind: .http,
        types: [RecipeType(name: "Item", fields: [RecipeField(name: "id", type: .string, optional: false)])],
        auth: .session(SessionAuth(
            kind: .formLogin(FormLogin(
                url: Template(literal: "https://example.com/login"),
                body: .form([("password", .path(.secret("password")))])
            ))
        )),
        body: [
            .step(HTTPStep(
                name: "items",
                request: HTTPRequest(method: "GET", url: Template(literal: "https://example.com/items"))
            )),
        ],
        secrets: ["password"]
    )
    let transport = SessionMockTransport()
    // Login succeeds. /items throws a non-401 NSError whose description
    // accidentally includes the literal secret string.
    let veryDistinctSecret = "S3CR3T_REDACT_ME_PLEASE"
    let cookieJSON = try JSONSerialization.data(withJSONObject: ["data": [["id": "x"]]], options: [.fragmentsAllowed])
    _ = cookieJSON
    await transport.register(urlSubstring: "/login") { _, _ in
        (Data("ok".utf8), 200, ["Set-Cookie": "session=ok; Path=/"])
    }
    await transport.register(urlSubstring: "/items") { _, _ in
        // 500 with the secret in the snippet — HTTPClient will surface it as
        // `HTTPClientError.badStatus(code: 500, snippet: "...\(secret)...")`
        // which the engine's catch-all picks up.
        (Data("server error: \(veryDistinctSecret)".utf8), 500, [:])
    }
    let client = HTTPClient(transport: transport, minRequestInterval: 0, maxRetries: 0)
    let runner = RecipeRunner(
        httpClient: client,
        secretResolver: DictionarySecretResolver(["password": veryDistinctSecret])
    )

    let result = try await runner.run(recipe: recipe, inputs: [:])
    #expect(result.report.stallReason.contains(veryDistinctSecret) == false,
            "stallReason should not contain the resolved secret value verbatim")
    #expect(result.report.stallReason.contains("<redacted>"))
}
