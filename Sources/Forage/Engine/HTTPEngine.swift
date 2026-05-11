import Foundation

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

    public init(
        client: HTTPClient,
        evaluator: ExtractionEvaluator = ExtractionEvaluator(),
        progress: HTTPProgress? = nil
    ) {
        self.client = client
        self.evaluator = evaluator
        self.progress = progress ?? HTTPProgress()
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

        var scope = Scope(inputs: inputs, frames: [[:]], current: nil)
        var collector = EmissionCollector()

        do {
            // Auth: prime step (htmlPrime) runs before the body and binds
            // variables into the top frame. staticHeader is applied per-request
            // inside `run(step:)`.
            if case .htmlPrime(let stepName, let captures) = recipe.auth {
                await setPhase(.priming)
                scope = try await runHtmlPrime(recipe: recipe, stepName: stepName, captures: captures, scope: scope)
            }

            try await runStatements(recipe.body, recipe: recipe, scope: &scope, collector: &collector)

            await setPhase(.done)
            let snapshot = Snapshot(records: collector.records, observedAt: Date())
            return RunResult(
                snapshot: snapshot,
                report: DiagnosticReport(
                    stallReason: "completed",
                    unmetExpectations: ExpectationEvaluator.evaluate(recipe.expectations, against: snapshot)
                )
            )
        } catch {
            let stallReason: String = Self.isCancellation(error) ? "cancelled" : "failed: \(error)"
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
    private func sendRequest(_ request: URLRequest) async throws -> (Data, HTTPURLResponse) {
        await noteRequest(request.url?.absoluteString)
        return try await client.send(request)
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
            let listValue = try PathResolver.resolve(collection, in: scope)
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
        let (data, _) = try await sendRequest(request)
        return try JSONValue.decode(data)
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
            let (data, _) = try await sendRequest(request)
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
            let (data, _) = try await sendRequest(request)
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
            let (data, _) = try await sendRequest(request)
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
