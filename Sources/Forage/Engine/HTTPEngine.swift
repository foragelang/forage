import Foundation
import CryptoKit

/// Executes an HTTP-engine `Recipe`. Walks the recipe's `body` (steps,
/// emissions, for-loops), maintains the runtime scope, accumulates emitted
/// records into a `Snapshot`.
///
/// Live progress is exposed via `progress` (an `HTTPProgress`) so consumers
/// can render phase / requests-sent / records-emitted without polling. The
/// engine drives all mutations as it runs.
public actor HTTPEngine {
    public let client: HTTPClient
    public let evaluator: ExtractionEvaluator

    public nonisolated let progress: HTTPProgress

    /// Resolves `$secret.<name>` references. Required if the recipe declares
    /// any secret references; nil otherwise.
    public let secretResolver: SecretResolver?
    /// Supplies MFA codes when the recipe declares `auth.session.requiresMFA: true`.
    public let mfaProvider: MFAProvider?
    /// Cache directory for persisted sessions. Defaults to
    /// `~/Library/Forage/Cache`. Tests override this to a tempdir.
    public let sessionCacheRoot: URL?
    /// Supplies the AES key for encrypted session caches. Defaults to a
    /// `KeychainSessionCacheKeyProvider`; tests pass an in-memory
    /// provider. Returning nil means `auth.session.cacheEncrypted: true`
    /// degrades gracefully to `chmod 600` plaintext.
    public let sessionCacheKeyProvider: SessionCacheKeyProvider

    /// In-flight session state (cookies / bearer token). Set by the login
    /// flow; cleared on re-auth.
    private var session: SessionState? = nil
    /// Re-auth budget for the current run. Reset to `maxReauthRetries` at
    /// start; decremented each time a 401/403 triggers re-auth.
    private var reauthBudget: Int = 0
    /// Resolved secrets for the current run, kept on the engine so the
    /// redactor can scrub diagnostic strings even after the scope has gone.
    private var resolvedSecrets: [String: String] = [:]

    public init(
        client: HTTPClient,
        evaluator: ExtractionEvaluator = ExtractionEvaluator(),
        progress: HTTPProgress? = nil,
        secretResolver: SecretResolver? = nil,
        mfaProvider: MFAProvider? = nil,
        sessionCacheRoot: URL? = nil,
        sessionCacheKeyProvider: SessionCacheKeyProvider = KeychainSessionCacheKeyProvider()
    ) {
        self.client = client
        self.evaluator = evaluator
        self.progress = progress ?? HTTPProgress()
        self.secretResolver = secretResolver
        self.mfaProvider = mfaProvider
        self.sessionCacheRoot = sessionCacheRoot
        self.sessionCacheKeyProvider = sessionCacheKeyProvider
    }

    /// Run a recipe with the given inputs. Returns a `RunResult` carrying
    /// the snapshot plus a `DiagnosticReport`. A successful run reports
    /// `stallReason == "completed"`; an interrupted run reports
    /// `stallReason == "failed: <description>"` and the snapshot reflects
    /// whatever the walker had emitted before the error. The HTTP engine
    /// has no `captures.match` concept, so `unmatchedCaptures`,
    /// `unfiredRules`, and `unhandledAffordances` are always empty.
    public func run(recipe: Recipe, inputs: [String: JSONValue]) async -> RunResult {
        precondition(recipe.engineKind == .http, "HTTPEngine requires recipe.engineKind == .http")

        // Reset per-run state. Engines are reused across runs in tests.
        self.session = nil
        self.reauthBudget = 0
        self.resolvedSecrets = [:]

        var scope = Scope(inputs: inputs, frames: [[:]], current: nil)
        var collector = EmissionCollector()

        // Pre-resolve every `$secret.<name>` the recipe references. Falls
        // through to the catch with a clear stallReason when a secret is
        // missing or the host didn't supply a resolver.
        do {
            self.resolvedSecrets = try await preResolveSecrets(recipe: recipe)
            scope = scope.withSecrets(self.resolvedSecrets)
        } catch let SecretError.notFound(name) {
            return runResult(
                stallReason: "auth-secret-missing: \(name)",
                snapshot: Snapshot(records: collector.records, observedAt: Date()),
                recipe: recipe,
                phaseAfter: .failed("auth-secret-missing: \(name)")
            )
        } catch {
            let redactor = SecretRedactor(from: scope)
            let stall = "failed: " + redactor.redact("\(error)")
            return runResult(
                stallReason: stall,
                snapshot: Snapshot(records: collector.records, observedAt: Date()),
                recipe: recipe,
                phaseAfter: .failed(stall)
            )
        }

        do {
            // Auth: prime step (htmlPrime) runs before the body and binds
            // variables into the top frame. staticHeader is applied per-request
            // inside `run(step:)`. session is also pre-run: log in (or load
            // a cached session) so the first regular step inherits cookies/headers.
            if case .htmlPrime(let stepName, let captures) = recipe.auth {
                await setPhase(.priming)
                scope = try await runHtmlPrime(recipe: recipe, stepName: stepName, captures: captures, scope: scope)
            }

            if case .session(let sauth) = recipe.auth {
                self.reauthBudget = sauth.maxReauthRetries
                await setPhase(.priming)
                try await bootstrapSession(recipe: recipe, sauth: sauth, scope: scope)
            }

            try await runStatements(recipe.body, recipe: recipe, scope: &scope, collector: &collector)

            // Persist the session if cache duration is set and we ran cleanly.
            if case .session(let sauth) = recipe.auth, let dur = sauth.cacheDuration, let st = self.session {
                persistSession(recipe: recipe, sauth: sauth, state: st, duration: dur)
            }

            await setPhase(.done)
            let snapshot = Snapshot(records: collector.records, observedAt: Date())
            return RunResult(
                snapshot: snapshot,
                report: DiagnosticReport(
                    stallReason: "completed",
                    unmetExpectations: ExpectationEvaluator.evaluate(recipe.expectations, against: snapshot)
                )
            )
        } catch let SessionAuthError.authFailed(detail) {
            let redactor = SecretRedactor(from: scope)
            let stall = "auth-failed: " + redactor.redact(detail)
            return runResult(
                stallReason: stall,
                snapshot: Snapshot(records: collector.records, observedAt: Date()),
                recipe: recipe,
                phaseAfter: .failed(stall)
            )
        } catch MFAError.cancelled {
            return runResult(
                stallReason: "auth-mfa-cancelled",
                snapshot: Snapshot(records: collector.records, observedAt: Date()),
                recipe: recipe,
                phaseAfter: .failed("auth-mfa-cancelled")
            )
        } catch {
            let stallReason: String
            if Self.isCancellation(error) {
                stallReason = "cancelled"
            } else {
                let redactor = SecretRedactor(from: scope)
                stallReason = "failed: " + redactor.redact("\(error)")
            }
            await setPhase(.failed(stallReason))
            let snapshot = Snapshot(records: collector.records, observedAt: Date())
            return RunResult(
                snapshot: snapshot,
                report: DiagnosticReport(
                    stallReason: stallReason,
                    unmetExpectations: ExpectationEvaluator.evaluate(recipe.expectations, against: snapshot)
                )
            )
        }
    }

    /// Compose a final `RunResult` and update progress in one go.
    private func runResult(
        stallReason: String,
        snapshot: Snapshot,
        recipe: Recipe,
        phaseAfter: HTTPProgress.Phase
    ) -> RunResult {
        // We can't `await` here (it's a local helper in the actor) — let the
        // caller mark the phase if needed. But run() is async so we hop now.
        Task { @MainActor [progress] in progress.setPhase(phaseAfter) }
        return RunResult(
            snapshot: snapshot,
            report: DiagnosticReport(
                stallReason: stallReason,
                unmetExpectations: ExpectationEvaluator.evaluate(recipe.expectations, against: snapshot)
            )
        )
    }

    /// Recognize task cancellation, however it arrived. `Task.cancel()` from
    /// the consumer throws `CancellationError` at the next `checkCancellation`
    /// site; in-flight `URLSession.data(for:)` calls surface the same signal
    /// as `URLError(.cancelled)`. Both should reduce to a "cancelled" stall
    /// reason rather than the generic "failed: …" envelope.
    private static func isCancellation(_ error: Error) -> Bool {
        if error is CancellationError { return true }
        if let urlError = error as? URLError, urlError.code == .cancelled { return true }
        return false
    }

    // MARK: - Progress helpers
    //
    // Hop to MainActor for every mutation; HTTPProgress is `@MainActor`.

    private func setPhase(_ phase: HTTPProgress.Phase) async {
        await MainActor.run {
            progress.setPhase(phase)
        }
    }

    private func noteRequest(_ url: String?) async {
        await MainActor.run { progress.noteRequestSent(url: url) }
    }

    private func noteEmission(count: Int) async {
        await MainActor.run { progress.setRecordsEmitted(count) }
    }

    /// Single send-point so `requestsSent` / `currentURL` are bumped once per
    /// real network call, regardless of which pagination strategy invoked it.
    ///
    /// If a session is active (cookies / bearer token), the credentials are
    /// injected into the request. A 401/403 triggers a single re-auth (per
    /// the remaining `reauthBudget`) and the original request retries; on a
    /// second 401/403, surfaces `auth-failed`.
    private func sendRequest(_ request: URLRequest, recipe: Recipe? = nil) async throws -> (Data, HTTPURLResponse) {
        let prepared = injectSessionCredentials(request)
        await noteRequest(prepared.url?.absoluteString)
        do {
            return try await client.send(prepared)
        } catch let HTTPClientError.badStatus(code, snippet) where (code == 401 || code == 403) {
            // Session expired mid-run. If we have a session-auth block and
            // budget left, drop the cached session and re-login once.
            if let recipe, case .session(let sauth) = recipe.auth, reauthBudget > 0 {
                reauthBudget -= 1
                // Evict any persisted cache: the credentials still work, but
                // the cached session doesn't.
                evictCachedSession(recipe: recipe, sauth: sauth)
                self.session = nil
                do {
                    try await performLogin(recipe: recipe, sauth: sauth, scope: Scope(inputs: [:], secrets: resolvedSecrets))
                } catch {
                    throw SessionAuthError.authFailed("re-auth login failed: \(error)")
                }
                let retried = injectSessionCredentials(request)
                await noteRequest(retried.url?.absoluteString)
                do {
                    return try await client.send(retried)
                } catch let HTTPClientError.badStatus(retriedCode, retriedSnippet) where (retriedCode == 401 || retriedCode == 403) {
                    throw SessionAuthError.authFailed("HTTP \(retriedCode) after re-auth\(retriedSnippet.map { ": \($0)" } ?? "")")
                }
            }
            // Either no session-auth, or budget exhausted: bubble up.
            if recipe != nil {
                throw SessionAuthError.authFailed("HTTP \(code)\(snippet.map { ": \($0)" } ?? "")")
            }
            throw HTTPClientError.badStatus(code: code, snippet: snippet)
        }
    }

    /// Apply current session state (cookies / Authorization header) to a
    /// pending request. No-op when no session is active. Recipe-defined
    /// headers always win — they're set later in `buildRequest`.
    private func injectSessionCredentials(_ request: URLRequest) -> URLRequest {
        guard let st = session else { return request }
        var r = request
        switch st.payload {
        case .cookies(let cookies):
            // Filter to cookies whose Domain matches the target host (when set).
            // RFC 6265 domain match: trailing-substring matches with leading dot
            // stripped. For tests against `example.com` we keep this loose: if
            // Domain is nil OR the host ends in (Domain stripped of leading dot),
            // include the cookie.
            let host = r.url?.host ?? ""
            let applicable = cookies.filter { c in
                guard let d = c.domain, !d.isEmpty else { return true }
                let bare = d.hasPrefix(".") ? String(d.dropFirst()) : d
                return host == bare || host.hasSuffix("." + bare)
            }
            if !applicable.isEmpty {
                // Merge with any existing Cookie header on the request.
                let existing = r.value(forHTTPHeaderField: "Cookie")
                let ours = SessionCookie.headerValue(applicable)
                let merged = [existing, ours].compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: "; ")
                r.setValue(merged, forHTTPHeaderField: "Cookie")
            }
        case .bearer(let token, let headerName, let headerPrefix):
            // Don't clobber a recipe-supplied auth header — the engine sets
            // the session header before recipe headers in `buildRequest`,
            // and recipe headers can override. But for cookie+bearer parity,
            // we just unconditionally set it here.
            r.setValue("\(headerPrefix)\(token)", forHTTPHeaderField: headerName)
        }
        return r
    }

    // MARK: - Session auth: bootstrap / login / persistence

    /// Establish a `session` before the body runs. Tries the on-disk cache
    /// first; falls back to running the login flow. The login result is
    /// stored on the engine so subsequent step requests inherit cookies /
    /// the bearer token.
    private func bootstrapSession(recipe: Recipe, sauth: SessionAuth, scope: Scope) async throws {
        // Try cache.
        if let dur = sauth.cacheDuration {
            if let cached = loadCachedSession(recipe: recipe, sauth: sauth) {
                if !cached.isExpired(maxAge: dur) {
                    self.session = cached
                    return
                }
                // Expired — drop it.
                evictCachedSession(recipe: recipe, sauth: sauth)
            }
        }

        // No usable cache: run the login flow now.
        try await performLogin(recipe: recipe, sauth: sauth, scope: scope)
    }

    /// Drive the login flow for the recipe's session-auth kind. Updates
    /// `self.session` on success; throws `SessionAuthError.authFailed` on
    /// failure.
    private func performLogin(recipe: Recipe, sauth: SessionAuth, scope: Scope) async throws {
        switch sauth.kind {
        case .formLogin(let f):
            try await runFormLogin(f, sauth: sauth, scope: scope)
        case .bearerLogin(let b):
            try await runBearerLogin(b, sauth: sauth, scope: scope)
        case .cookiePersist(let c):
            try await runCookiePersist(c, scope: scope)
        }
    }

    private func runFormLogin(_ f: FormLogin, sauth: SessionAuth, scope: Scope) async throws {
        // Build the login request. If MFA is required, ask the provider for
        // a code and inject it as `<mfaFieldName>: <code>` into the body.
        var loginBody = f.body
        if sauth.requiresMFA {
            guard let mfa = mfaProvider else {
                throw SessionAuthError.authFailed("recipe declares requiresMFA: true but no MFAProvider was supplied")
            }
            let code = try await mfa.mfaCode()
            loginBody = addMFAField(to: loginBody, name: sauth.mfaFieldName, code: code)
        }
        let request = try buildRawRequest(
            url: f.url, method: f.method, body: loginBody, headers: [], recipe: nil, scope: scope
        )
        await noteRequest(request.url?.absoluteString)
        let (_, response) = try await sendLogin(request)
        if !(200..<300).contains(response.statusCode) {
            throw SessionAuthError.authFailed("login HTTP \(response.statusCode)")
        }
        // Capture cookies from `Set-Cookie` headers. URLResponse exposes
        // multi-value headers via `allHeaderFields` (merged with comma-join),
        // which corrupts the `Set-Cookie` payload (cookies legitimately
        // contain commas). Use HTTPCookieStorage's helper to parse properly.
        let cookies: [SessionCookie] = {
            guard f.captureCookies else { return [] }
            if let url = response.url {
                let httpCookies = HTTPCookie.cookies(
                    withResponseHeaderFields: (response.allHeaderFields as? [String: String]) ?? [:],
                    for: url
                )
                if !httpCookies.isEmpty {
                    return httpCookies.map { SessionCookie(name: $0.name, value: $0.value, domain: $0.domain, path: $0.path) }
                }
            }
            // Fallback: parse raw `Set-Cookie` strings if URL is nil. We don't
            // have multi-value access on HTTPURLResponse pre-iOS 13/macOS 10.15
            // so try `Set-Cookie` directly.
            let raw = (response.allHeaderFields["Set-Cookie"] as? String).map { [$0] } ?? []
            return SessionCookie.parseSetCookieHeaders(raw)
        }()
        if cookies.isEmpty && f.captureCookies {
            // No cookies returned from a form login is usually a problem.
            // Surface as auth-failed so the user can see the recipe expects
            // cookies but the server didn't set any.
            throw SessionAuthError.authFailed("form login succeeded but server returned no Set-Cookie")
        }
        self.session = SessionState(payload: .cookies(cookies))
    }

    private func runBearerLogin(_ b: BearerLogin, sauth: SessionAuth, scope: Scope) async throws {
        var loginBody = b.body
        if sauth.requiresMFA {
            guard let mfa = mfaProvider else {
                throw SessionAuthError.authFailed("recipe declares requiresMFA: true but no MFAProvider was supplied")
            }
            let code = try await mfa.mfaCode()
            loginBody = addMFAField(to: loginBody, name: sauth.mfaFieldName, code: code)
        }
        let request = try buildRawRequest(
            url: b.url, method: b.method, body: loginBody, headers: [], recipe: nil, scope: scope
        )
        await noteRequest(request.url?.absoluteString)
        let (data, response) = try await sendLogin(request)
        if !(200..<300).contains(response.statusCode) {
            throw SessionAuthError.authFailed("login HTTP \(response.statusCode)")
        }
        // Extract the token via `tokenPath` from the response body (JSON).
        let parsed = try JSONValue.decode(data)
        let bodyScope = scope.withCurrent(parsed)
        guard case .string(let token) = try PathResolver.resolve(b.tokenPath, in: bodyScope), !token.isEmpty else {
            throw SessionAuthError.authFailed("bearer login: tokenPath did not resolve to a non-empty string")
        }
        self.session = SessionState(payload: .bearer(
            token: token, headerName: b.headerName, headerPrefix: b.headerPrefix
        ))
    }

    private func runCookiePersist(_ c: CookiePersist, scope: Scope) async throws {
        let path = try TemplateRenderer.render(c.sourcePath, in: scope)
        let url = URL(fileURLWithPath: path)
        let cookies: [SessionCookie]
        switch c.format {
        case .json:
            let data = try Data(contentsOf: url)
            cookies = try JSONDecoder().decode([SessionCookie].self, from: data)
        case .netscape:
            // Netscape `cookies.txt`: tab-separated, comment lines start with #.
            // Columns: domain, includeSubdomains, path, secure, expiry, name, value.
            let text = try String(contentsOf: url, encoding: .utf8)
            var parsed: [SessionCookie] = []
            for line in text.split(separator: "\n") {
                let l = line.trimmingCharacters(in: .whitespaces)
                if l.isEmpty || l.hasPrefix("#") { continue }
                let cols = l.split(separator: "\t", omittingEmptySubsequences: false)
                guard cols.count >= 7 else { continue }
                let domain = String(cols[0])
                let p = String(cols[2])
                let name = String(cols[5])
                let value = String(cols[6])
                parsed.append(SessionCookie(name: name, value: value, domain: domain, path: p))
            }
            cookies = parsed
        }
        if cookies.isEmpty {
            throw SessionAuthError.authFailed("cookiePersist: no cookies parsed from \(path)")
        }
        self.session = SessionState(payload: .cookies(cookies))
    }

    /// Send a login request without the usual session injection (we're
    /// establishing the session, not consuming it). The HTTPClient still
    /// applies its rate-limit + 5xx/429 retries.
    ///
    /// 4xx login failures (401/403 for bad credentials) are mapped to
    /// `SessionAuthError.authFailed` so the outer `run` catch surfaces them
    /// with the correct `auth-failed:` prefix instead of the generic
    /// `failed: …` envelope.
    private func sendLogin(_ request: URLRequest) async throws -> (Data, HTTPURLResponse) {
        do {
            return try await client.send(request)
        } catch let HTTPClientError.badStatus(code, snippet) {
            throw SessionAuthError.authFailed("login HTTP \(code)\(snippet.map { ": \($0)" } ?? "")")
        }
    }

    private func addMFAField(to body: HTTPBody, name: String, code: String) -> HTTPBody {
        switch body {
        case .jsonObject(var kvs):
            kvs.removeAll(where: { $0.key == name })
            kvs.append(HTTPBodyKV(key: name, value: .literal(.string(code))))
            return .jsonObject(kvs)
        case .form(var pairs):
            pairs.removeAll(where: { $0.0 == name })
            pairs.append((name, .literal(.string(code))))
            return .form(pairs)
        case .raw(let t):
            // For raw bodies we can't safely splice in another field — leave it.
            // Recipe authors using raw bodies are responsible for templating
            // {$secret.mfaCode} or similar themselves.
            return .raw(t)
        }
    }

    /// Build a URLRequest from primitive pieces (used by the login flow,
    /// where we don't have an `HTTPStep` to feed `buildRequest`).
    private func buildRawRequest(
        url: Template,
        method: String,
        body: HTTPBody?,
        headers: [(String, Template)],
        recipe: Recipe?,
        scope: Scope
    ) throws -> URLRequest {
        let template = HTTPRequest(method: method, url: url, headers: headers, body: body)
        // We can reuse buildRequest's body/header/pagination handling by
        // wrapping the recipe-less call site: it expects a `Recipe`, but the
        // only thing it needs from it is `recipe.auth`. Build a dummy one
        // with no auth so staticHeader doesn't fire on the login.
        let dummy = recipe ?? Recipe(name: "<login>", engineKind: .http)
        return try buildRequest(template, recipe: dummy, scope: scope, paginationOverride: nil)
    }

    // MARK: - Session cache

    private func loadCachedSession(recipe: Recipe, sauth: SessionAuth) -> SessionState? {
        let fp = CredentialFingerprint.compute(secrets: resolvedSecrets)
        let url = SessionCache.cacheFile(for: recipe.name, fingerprint: fp, root: sessionCacheRoot)
        let key = sauth.cacheEncrypted ? symmetricKeyForCache() : nil
        return SessionCache.read(at: url, encryptionKey: key)
    }

    private func persistSession(recipe: Recipe, sauth: SessionAuth, state: SessionState, duration: TimeInterval) {
        _ = duration  // Captured in state's createdAt + caller-supplied duration on read.
        let fp = CredentialFingerprint.compute(secrets: resolvedSecrets)
        let url = SessionCache.cacheFile(for: recipe.name, fingerprint: fp, root: sessionCacheRoot)
        let key = sauth.cacheEncrypted ? symmetricKeyForCache() : nil
        do {
            try SessionCache.write(state, to: url, encryptionKey: key)
        } catch {
            // Cache-write failures are non-fatal: the run already finished
            // successfully; we just won't have a cache next time.
        }
    }

    private func evictCachedSession(recipe: Recipe, sauth: SessionAuth) {
        let fp = CredentialFingerprint.compute(secrets: resolvedSecrets)
        let url = SessionCache.cacheFile(for: recipe.name, fingerprint: fp, root: sessionCacheRoot)
        SessionCache.evict(at: url)
    }

    /// Per-machine AES key for `auth.session.cacheEncrypted: true`.
    /// Delegates to the configured `SessionCacheKeyProvider`. The
    /// default `KeychainSessionCacheKeyProvider` generates and stores a
    /// 256-bit key in the macOS Keychain on first use; tests pass an
    /// `InMemorySessionCacheKeyProvider`. A nil return (no Keychain
    /// access, key generation failed) falls back to plain-but-chmod-600,
    /// matching the pre-M7 behavior.
    private func symmetricKeyForCache() -> CryptoKit.SymmetricKey? {
        sessionCacheKeyProvider.key()
    }

    // MARK: - Secret resolution

    /// Walk the recipe and resolve every referenced `$secret.<name>` exactly
    /// once. The resolver call is `async throws` so we hop here at run start
    /// and freeze the result into the scope.
    private func preResolveSecrets(recipe: Recipe) async throws -> [String: String] {
        // Collect names: prefer the declared list when present (catches typos
        // earlier), else fall back to traversing the AST. The validator already
        // warned on undeclared references, so we collect both for safety.
        var names = Set(recipe.secrets)
        names.formUnion(allReferencedSecrets(recipe: recipe))
        guard !names.isEmpty else { return [:] }
        guard let resolver = secretResolver else {
            // No resolver but secrets are referenced: surface as auth-secret-missing.
            let first = names.sorted().first ?? "?"
            throw SecretError.notFound(first)
        }
        var out: [String: String] = [:]
        for name in names.sorted() {
            out[name] = try await resolver.resolve(name)
        }
        return out
    }

    // MARK: - Statement walker

    private func runStatements(
        _ statements: [Statement],
        recipe: Recipe,
        scope: inout Scope,
        collector: inout EmissionCollector
    ) async throws {
        // htmlPrime consumed its named step during the priming phase; skip it
        // here so the walker doesn't fire the same request twice.
        let primeStepName: String? = {
            if case .htmlPrime(let name, _) = recipe.auth { return name }
            return nil
        }()
        for statement in statements {
            if let primeStepName,
               case .step(let s) = statement, s.name == primeStepName {
                continue
            }
            try await runStatement(statement, recipe: recipe, scope: &scope, collector: &collector)
        }
    }

    private func runStatement(
        _ statement: Statement,
        recipe: Recipe,
        scope: inout Scope,
        collector: inout EmissionCollector
    ) async throws {
        switch statement {
        case .step(let step):
            await setPhase(.stepping(name: step.name))
            let result = try await runStep(step, recipe: recipe, scope: scope)
            scope = scope.with(step.name, result)

        case .emit(let emission):
            let record = try evaluator.emit(emission, in: scope)
            collector.append(record)
            await noteEmission(count: collector.records.count)

        case .forLoop(let varName, let collection, let body):
            let listValue = try evaluator.evaluateToJSON(collection, in: scope)
            let items: [JSONValue]
            switch listValue {
            case .array(let xs): items = xs
            case .null: items = []
            default: items = [listValue]
            }
            for item in items {
                var inner = scope.with(varName, item).withCurrent(item)
                try await runStatements(body, recipe: recipe, scope: &inner, collector: &collector)
                // The inner scope's variable bindings drop on each iteration;
                // step results from inside the loop don't leak out either.
                // (We restore by not propagating `inner` back.)
            }
        }
    }

    // MARK: - Step execution

    private func runStep(_ step: HTTPStep, recipe: Recipe, scope: Scope) async throws -> JSONValue {
        try Task.checkCancellation()
        if let pagination = step.pagination {
            return try await runPaginated(step: step, recipe: recipe, scope: scope, pagination: pagination)
        }
        let request = try buildRequest(step.request, recipe: recipe, scope: scope, paginationOverride: nil)
        let (data, response) = try await sendRequest(request, recipe: recipe)
        let ct = response.value(forHTTPHeaderField: "Content-Type")
        return JSONValue.decodeBody(data, contentType: ct)
    }

    private func runPaginated(
        step: HTTPStep,
        recipe: Recipe,
        scope: Scope,
        pagination: Pagination
    ) async throws -> JSONValue {
        switch pagination {
        case .pageWithTotal(let itemsPath, let totalPath, let pageParam, let pageSize, let zeroIndexed):
            return try await runPageWithTotal(
                step: step, recipe: recipe, scope: scope,
                itemsPath: itemsPath, totalPath: totalPath,
                pageParam: pageParam, pageSize: pageSize, zeroIndexed: zeroIndexed
            )
        case .untilEmpty(let itemsPath, let pageParam, let zeroIndexed):
            return try await runUntilEmpty(
                step: step, recipe: recipe, scope: scope,
                itemsPath: itemsPath, pageParam: pageParam, zeroIndexed: zeroIndexed
            )
        case .cursor(let itemsPath, let cursorPath, let cursorParam):
            return try await runCursor(
                step: step, recipe: recipe, scope: scope,
                itemsPath: itemsPath, cursorPath: cursorPath, cursorParam: cursorParam
            )
        }
    }

    private func runPageWithTotal(
        step: HTTPStep, recipe: Recipe, scope: Scope,
        itemsPath: PathExpr, totalPath: PathExpr,
        pageParam: String, pageSize: Int, zeroIndexed: Bool
    ) async throws -> JSONValue {
        var collected: [JSONValue] = []
        var page = zeroIndexed ? 0 : 1
        var humanPage = 1
        while true {
            try Task.checkCancellation()
            await setPhase(.paginating(name: step.name, page: humanPage))
            let pageOverride = PaginationOverride(param: pageParam, value: .int(page))
            let request = try buildRequest(step.request, recipe: recipe, scope: scope, paginationOverride: pageOverride)
            let (data, _) = try await sendRequest(request, recipe: recipe)
            let response = try JSONValue.decode(data)

            let pageScope = scope.withCurrent(response)
            let items = try PathResolver.resolve(itemsPath, in: pageScope)
            let total = try PathResolver.resolve(totalPath, in: pageScope)
            let totalCount: Int = {
                if case .int(let n) = total { return n }
                if case .double(let d) = total { return Int(d) }
                return 0
            }()

            if case .array(let xs) = items {
                collected.append(contentsOf: xs)
                if collected.count >= totalCount || xs.isEmpty { break }
            } else {
                break
            }

            page += 1
            humanPage += 1
            if page > 200 { break }   // safety cap
        }
        return .array(collected)
    }

    private func runUntilEmpty(
        step: HTTPStep, recipe: Recipe, scope: Scope,
        itemsPath: PathExpr, pageParam: String, zeroIndexed: Bool
    ) async throws -> JSONValue {
        var collected: [JSONValue] = []
        var page = zeroIndexed ? 0 : 1
        var humanPage = 1
        while true {
            try Task.checkCancellation()
            await setPhase(.paginating(name: step.name, page: humanPage))
            let pageOverride = PaginationOverride(param: pageParam, value: .int(page))
            let request = try buildRequest(step.request, recipe: recipe, scope: scope, paginationOverride: pageOverride)
            let (data, _) = try await sendRequest(request, recipe: recipe)
            let response = try JSONValue.decode(data)
            let items = try PathResolver.resolve(itemsPath, in: scope.withCurrent(response))
            if case .array(let xs) = items, !xs.isEmpty {
                collected.append(contentsOf: xs)
            } else {
                break
            }
            page += 1
            humanPage += 1
            if page > 500 { break }
        }
        return .array(collected)
    }

    private func runCursor(
        step: HTTPStep, recipe: Recipe, scope: Scope,
        itemsPath: PathExpr, cursorPath: PathExpr, cursorParam: String
    ) async throws -> JSONValue {
        var collected: [JSONValue] = []
        var cursor: JSONValue = .null
        var iter = 0
        while true {
            try Task.checkCancellation()
            await setPhase(.paginating(name: step.name, page: iter + 1))
            let cursorString: String? = {
                if case .string(let s) = cursor, !s.isEmpty { return s }
                return nil
            }()
            let override = cursorString.map { PaginationOverride(param: cursorParam, value: .string($0)) }
            let request = try buildRequest(step.request, recipe: recipe, scope: scope, paginationOverride: override)
            let (data, _) = try await sendRequest(request, recipe: recipe)
            let response = try JSONValue.decode(data)
            let pageScope = scope.withCurrent(response)
            let items = try PathResolver.resolve(itemsPath, in: pageScope)
            if case .array(let xs) = items { collected.append(contentsOf: xs) }
            cursor = (try? PathResolver.resolve(cursorPath, in: pageScope)) ?? .null
            if case .null = cursor { break }
            if case .string(let s) = cursor, s.isEmpty { break }
            iter += 1
            if iter > 500 { break }
        }
        return .array(collected)
    }

    // MARK: - htmlPrime (auth)

    private func runHtmlPrime(
        recipe: Recipe,
        stepName: String,
        captures: [HtmlPrimeVar],
        scope: Scope
    ) async throws -> Scope {
        try Task.checkCancellation()
        guard let primeStmt = recipe.body.first(where: {
            if case .step(let s) = $0, s.name == stepName { return true }
            return false
        }), case .step(let primeStep) = primeStmt else {
            throw EngineError.htmlPrimeStepNotFound(stepName)
        }
        let request = try buildRequest(primeStep.request, recipe: recipe, scope: scope, paginationOverride: nil)
        let (data, _) = try await sendRequest(request)
        guard let html = String(data: data, encoding: .utf8) else {
            throw EngineError.htmlPrimeNotText
        }

        var newScope = scope
        for capture in captures {
            let regex = try NSRegularExpression(pattern: capture.regexPattern)
            guard let match = regex.firstMatch(in: html, range: NSRange(html.startIndex..., in: html)),
                  capture.groupIndex < match.numberOfRanges,
                  let range = Range(match.range(at: capture.groupIndex), in: html)
            else {
                throw EngineError.htmlPrimeRegexNoMatch(varName: capture.varName, pattern: capture.regexPattern)
            }
            let value = String(html[range])
            newScope = newScope.with(capture.varName, .string(value))
        }
        return newScope
    }

    // MARK: - Request building

    private struct PaginationOverride {
        let param: String
        let value: JSONValue
    }

    private func buildRequest(
        _ template: HTTPRequest,
        recipe: Recipe,
        scope: Scope,
        paginationOverride: PaginationOverride?
    ) throws -> URLRequest {
        var urlString = try TemplateRenderer.render(template.url, in: scope)
        // For pageWithTotal/untilEmpty, the page param goes in the body. For
        // cursor, the cursor goes in the body too (or query — recipe author's
        // choice; default is body for consistency).
        var request = URLRequest(url: URL(string: urlString)!)
        request.httpMethod = template.method

        // Apply auth.staticHeader before recipe headers (recipe headers can override).
        if case .staticHeader(let name, let value) = recipe.auth {
            let rendered = try TemplateRenderer.render(value, in: scope)
            request.setValue(rendered, forHTTPHeaderField: name)
        }
        for (k, v) in template.headers {
            let rendered = try TemplateRenderer.render(v, in: scope)
            request.setValue(rendered, forHTTPHeaderField: k)
        }

        // Body
        if let body = template.body {
            switch body {
            case .jsonObject(var kvs):
                if let p = paginationOverride {
                    kvs = upserted(kvs, key: p.param, value: .literal(p.value))
                }
                let json = try renderJSONBody(kvs, scope: scope)
                let data = try JSONSerialization.data(withJSONObject: json, options: [.fragmentsAllowed])
                request.httpBody = data
                if request.value(forHTTPHeaderField: "Content-Type") == nil {
                    request.setValue("application/json", forHTTPHeaderField: "Content-Type")
                }
            case .form(var pairs):
                if let p = paginationOverride {
                    pairs = upsertedForm(pairs, key: p.param, value: .literal(p.value))
                }
                var rendered: [(String, String)] = []
                for (k, v) in pairs {
                    let any = try renderBodyValue(v, scope: scope)
                    rendered.append((k, Self.stringifyAny(any)))
                }
                request.httpBody = BodyEncoding.formEncoded(rendered)
                if request.value(forHTTPHeaderField: "Content-Type") == nil {
                    request.setValue("application/x-www-form-urlencoded", forHTTPHeaderField: "Content-Type")
                }
            case .raw(let t):
                request.httpBody = try TemplateRenderer.render(t, in: scope).data(using: .utf8)
            }
        }

        _ = urlString
        return request
    }

    private func renderJSONBody(_ kvs: [HTTPBodyKV], scope: Scope) throws -> [String: Any] {
        var dict: [String: Any] = [:]
        for kv in kvs {
            dict[kv.key] = try renderBodyValue(kv.value, scope: scope)
        }
        return dict
    }

    private func renderBodyValue(_ bv: BodyValue, scope: Scope) throws -> Any {
        switch bv {
        case .templateString(let t):
            return try TemplateRenderer.render(t, in: scope)
        case .literal(let j):
            return j.asAny
        case .path(let p):
            let v = try PathResolver.resolve(p, in: scope)
            return v.asAny
        case .object(let kvs):
            var dict: [String: Any] = [:]
            for kv in kvs { dict[kv.key] = try renderBodyValue(kv.value, scope: scope) }
            return dict
        case .array(let xs):
            return try xs.map { try renderBodyValue($0, scope: scope) }
        case .caseOf(let scrutinee, let branches):
            let v = try PathResolver.resolve(scrutinee, in: scope)
            guard case .string(let label) = v else { throw EngineError.caseScrutineeNotEnumLabel }
            for branch in branches where branch.label == label {
                return try renderBodyValue(branch.value, scope: scope)
            }
            throw EngineError.caseNoMatchingBranch(label: label, available: branches.map(\.label))
        }
    }

    private func upserted(_ kvs: [HTTPBodyKV], key: String, value: BodyValue) -> [HTTPBodyKV] {
        var out = kvs.filter { $0.key != key }
        out.append(HTTPBodyKV(key: key, value: value))
        return out
    }

    private func upsertedForm(_ pairs: [(String, BodyValue)], key: String, value: BodyValue) -> [(String, BodyValue)] {
        var out = pairs.filter { $0.0 != key }
        out.append((key, value))
        return out
    }

    static func stringifyAny(_ any: Any) -> String {
        if let s = any as? String { return s }
        if let i = any as? Int { return String(i) }
        if let d = any as? Double {
            if d == d.rounded() && abs(d) < 1e15 { return String(Int(d)) }
            return String(d)
        }
        if let b = any as? Bool { return String(b) }
        if any is NSNull { return "" }
        return "\(any)"
    }
}

// MARK: - Emission collector

public struct EmissionCollector {
    public var records: [ScrapedRecord] = []
    public init() {}
    public mutating func append(_ r: ScrapedRecord) { records.append(r) }
}

// MARK: - Errors

public enum EngineError: Error, CustomStringConvertible {
    case htmlPrimeStepNotFound(String)
    case htmlPrimeNotText
    case htmlPrimeRegexNoMatch(varName: String, pattern: String)
    case caseScrutineeNotEnumLabel
    case caseNoMatchingBranch(label: String, available: [String])

    public var description: String {
        switch self {
        case .htmlPrimeStepNotFound(let n): return "auth.htmlPrime references unknown step '\(n)'"
        case .htmlPrimeNotText: return "auth.htmlPrime: response body wasn't UTF-8 text"
        case .htmlPrimeRegexNoMatch(let v, let p): return "auth.htmlPrime: regex didn't match for $\(v) (pattern: \(p))"
        case .caseScrutineeNotEnumLabel: return "case-of: scrutinee didn't resolve to an enum label string"
        case .caseNoMatchingBranch(let l, let a): return "case-of: no branch matched '\(l)' (available: \(a.joined(separator: ", ")))"
        }
    }
}

/// Errors raised by the session-auth runtime. The engine catches these and
/// surfaces a friendly `stallReason` envelope; the underlying error keeps
/// any secret values redacted.
public enum SessionAuthError: Error, CustomStringConvertible {
    case authFailed(String)

    public var description: String {
        switch self {
        case .authFailed(let detail): return detail
        }
    }
}

// MARK: - Secret-reference traversal (engine-local copy)

/// Walk the recipe and gather every `$secret.<name>` referenced anywhere.
/// Mirrors `Validator.collectReferencedSecrets` so the engine doesn't depend
/// on the validator at run time.
fileprivate func allReferencedSecrets(recipe: Recipe) -> Set<String> {
    var out = Set<String>()
    if case .session(let s) = recipe.auth {
        switch s.kind {
        case .formLogin(let f):
            out.formUnion(secretsInTemplate(f.url))
            out.formUnion(secretsInBody(f.body))
        case .bearerLogin(let b):
            out.formUnion(secretsInTemplate(b.url))
            out.formUnion(secretsInBody(b.body))
            out.formUnion(b.tokenPath.referencedSecrets)
        case .cookiePersist(let c):
            out.formUnion(secretsInTemplate(c.sourcePath))
        }
    }
    if case .staticHeader(_, let v) = recipe.auth {
        out.formUnion(secretsInTemplate(v))
    }
    for stmt in recipe.body { collectSecretsInStatement(stmt, into: &out) }
    return out
}

fileprivate func collectSecretsInStatement(_ stmt: Statement, into out: inout Set<String>) {
    switch stmt {
    case .step(let s):
        out.formUnion(secretsInTemplate(s.request.url))
        for (_, hv) in s.request.headers { out.formUnion(secretsInTemplate(hv)) }
        if let body = s.request.body { out.formUnion(secretsInBody(body)) }
    case .emit(let em):
        for b in em.bindings { out.formUnion(secretsInExpr(b.expr)) }
    case .forLoop(_, let coll, let body):
        // Post-M8: forLoop.collection is an ExtractionExpr (pipelines can
        // drive iteration). Walk it via secretsInExpr instead of the
        // PathExpr-only `.referencedSecrets` extension.
        out.formUnion(secretsInExpr(coll))
        for s in body { collectSecretsInStatement(s, into: &out) }
    }
}

fileprivate func secretsInTemplate(_ t: Template) -> Set<String> {
    var out = Set<String>()
    for part in t.parts {
        if case .interp(let expr) = part {
            out.formUnion(secretsInExpr(expr))
        }
    }
    return out
}

fileprivate func secretsInBody(_ body: HTTPBody) -> Set<String> {
    var out = Set<String>()
    switch body {
    case .jsonObject(let kvs):
        for kv in kvs { out.formUnion(secretsInBodyValue(kv.value)) }
    case .form(let kvs):
        for (_, v) in kvs { out.formUnion(secretsInBodyValue(v)) }
    case .raw(let t):
        out.formUnion(secretsInTemplate(t))
    }
    return out
}

fileprivate func secretsInBodyValue(_ bv: BodyValue) -> Set<String> {
    switch bv {
    case .templateString(let t): return secretsInTemplate(t)
    case .literal: return []
    case .path(let p): return p.referencedSecrets
    case .object(let kvs):
        var out = Set<String>()
        for kv in kvs { out.formUnion(secretsInBodyValue(kv.value)) }
        return out
    case .array(let xs):
        var out = Set<String>()
        for v in xs { out.formUnion(secretsInBodyValue(v)) }
        return out
    case .caseOf(let scrutinee, let branches):
        var out = scrutinee.referencedSecrets
        for (_, v) in branches { out.formUnion(secretsInBodyValue(v)) }
        return out
    }
}

fileprivate func secretsInExpr(_ expr: ExtractionExpr) -> Set<String> {
    switch expr {
    case .path(let p): return p.referencedSecrets
    case .pipe(let inner, let calls):
        var out = secretsInExpr(inner)
        for c in calls { for a in c.args { out.formUnion(secretsInExpr(a)) } }
        return out
    case .caseOf(let scrutinee, let branches):
        var out = scrutinee.referencedSecrets
        for (_, e) in branches { out.formUnion(secretsInExpr(e)) }
        return out
    case .mapTo(let p, _): return p.referencedSecrets
    case .literal: return []
    case .template(let t): return secretsInTemplate(t)
    case .call(_, let args):
        var out = Set<String>()
        for a in args { out.formUnion(secretsInExpr(a)) }
        return out
    }
}
