import Foundation

/// Evaluates an `ExtractionExpr` against the current scope to produce a
/// `TypedValue` (the recipe's emit-output type). Recursively handles
/// pipelines, case-of branches, map-to-record blocks, and function-call
/// transforms.
public struct ExtractionEvaluator: Sendable {

    public let transforms: TransformImpls

    public init(transforms: TransformImpls = TransformImpls()) {
        self.transforms = transforms
    }

    public func evaluate(_ expr: ExtractionExpr, in scope: Scope) throws -> TypedValue {
        let json = try evaluateToJSON(expr, in: scope)
        return Self.lift(json)
    }

    /// Internal: most expressions yield JSON-shaped values; we lift to
    /// TypedValue at the binding site.
    public func evaluateToJSON(_ expr: ExtractionExpr, in scope: Scope) throws -> JSONValue {
        switch expr {
        case .path(let p):
            return try PathResolver.resolve(p, in: scope)

        case .literal(let v):
            return v

        case .template(let t):
            return .string(try TemplateRenderer.render(t, in: scope))

        case .pipe(let inner, let calls):
            var v = try evaluateToJSON(inner, in: scope)
            for call in calls {
                let args = try call.args.map { try evaluateToJSON($0, in: scope) }
                v = try transforms.apply(call.name, value: v, args: args)
            }
            return v

        case .caseOf(let scrutinee, let branches):
            let label = try resolveEnumLabel(scrutinee, in: scope)
            for branch in branches {
                if branch.label == label {
                    return try evaluateToJSON(branch.expr, in: scope)
                }
            }
            throw EvaluationError.noMatchingCaseBranch(label: label, available: branches.map(\.label))

        case .mapTo(let path, let emission):
            let listValue = try PathResolver.resolve(path, in: scope)
            guard case .array(let items) = listValue else {
                if case .null = listValue { return .array([]) }
                throw EvaluationError.expectedListForMap
            }
            var out: [JSONValue] = []
            for item in items {
                let itemScope = scope.withCurrent(item)
                let record = try emit(emission, in: itemScope)
                out.append(.object(Self.recordToJSON(record)))
            }
            return .array(out)

        case .call(let name, let args):
            // Function-call form: applies `name` as a transform with no piped
            // input (treated as null) and the args as transform args. Most
            // commonly used for `coalesce(a, b)` style multi-arg transforms.
            let evaluatedArgs = try args.map { try evaluateToJSON($0, in: scope) }
            // Convention: first arg is treated as the "value" for piped-style
            // transforms that also accept multiple args (coalesce, default).
            let value = evaluatedArgs.first ?? .null
            let rest = Array(evaluatedArgs.dropFirst())
            return try transforms.apply(name, value: value, args: rest)
        }
    }

    /// Construct a `ScrapedRecord` from an `Emission` by evaluating each
    /// binding in the current scope.
    public func emit(_ emission: Emission, in scope: Scope) throws -> ScrapedRecord {
        var fields: [String: TypedValue] = [:]
        for binding in emission.bindings {
            let v = try evaluate(binding.expr, in: scope)
            fields[binding.fieldName] = v
        }
        return ScrapedRecord(typeName: emission.typeName, fields: fields)
    }

    // MARK: - Helpers

    /// Read a path expression that should resolve to a string-shaped enum
    /// label (e.g. `$menu` whose binding is `.string("RECREATIONAL")`).
    private func resolveEnumLabel(_ path: PathExpr, in scope: Scope) throws -> String {
        let v = try PathResolver.resolve(path, in: scope)
        guard case .string(let s) = v else {
            throw EvaluationError.expectedEnumLabel(got: v)
        }
        return s
    }

    /// Lift a JSON-shaped value to a TypedValue. Object → flat record under
    /// a synthetic name (we never lift unnamed objects to records in normal
    /// flows; map-to-record uses the explicit Emission path).
    public static func lift(_ json: JSONValue) -> TypedValue {
        switch json {
        case .null: return .null
        case .bool(let b): return .bool(b)
        case .int(let i): return .int(i)
        case .double(let d): return .double(d)
        case .string(let s): return .string(s)
        case .array(let a): return .array(a.map { lift($0) })
        case .object(let o):
            // Special case: an object containing a `_typeName` synthetic key
            // is treated as a record. Map-to-record bakes this into the JSON.
            if case .string(let name)? = o["_typeName"] {
                var fields = o
                fields.removeValue(forKey: "_typeName")
                return .record(ScrapedRecord(typeName: name, fields: fields.mapValues { lift($0) }))
            }
            // Plain object: lift to anonymous record.
            return .record(ScrapedRecord(typeName: "_anonymous", fields: o.mapValues { lift($0) }))
        }
    }

    /// Reverse of `lift` for the map-to-record path: convert a record to a
    /// JSON object so it can sit inside an array we hand back through the
    /// pipeline.
    public static func recordToJSON(_ record: ScrapedRecord) -> [String: JSONValue] {
        var out: [String: JSONValue] = ["_typeName": .string(record.typeName)]
        for (k, v) in record.fields {
            out[k] = typedToJSON(v)
        }
        return out
    }

    public static func typedToJSON(_ v: TypedValue) -> JSONValue {
        switch v {
        case .null: return .null
        case .bool(let b): return .bool(b)
        case .int(let i): return .int(i)
        case .double(let d): return .double(d)
        case .string(let s): return .string(s)
        case .array(let xs): return .array(xs.map { typedToJSON($0) })
        case .record(let r): return .object(recordToJSON(r))
        }
    }
}

public enum EvaluationError: Error, CustomStringConvertible {
    case noMatchingCaseBranch(label: String, available: [String])
    case expectedEnumLabel(got: JSONValue)
    case expectedListForMap

    public var description: String {
        switch self {
        case .noMatchingCaseBranch(let l, let a):
            return "case-of: no branch matched '\(l)' (available: \(a.joined(separator: ", ")))"
        case .expectedEnumLabel(let v):
            return "case-of: scrutinee must resolve to a string enum label, got \(v)"
        case .expectedListForMap:
            return "map-to-record: path must resolve to a list"
        }
    }
}
