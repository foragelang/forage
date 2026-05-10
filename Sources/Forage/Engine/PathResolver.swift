import Foundation

/// Evaluates `PathExpr` against a `Scope` to produce one or more `JSONValue`s.
///
/// A wildcard (`[*]`) widens the result to a list — the resolver returns a
/// `.array` whose elements are the wildcard expansion. The runtime decides
/// per call site whether to consume the result as a scalar or as a sequence.
public enum PathResolver {

    public static func resolve(_ expr: PathExpr, in scope: Scope) throws -> JSONValue {
        switch expr {
        case .current:
            guard let v = scope.current else { throw ScopeError.noCurrentValue }
            return v
        case .input:
            // Bare `$input` (without trailing field) — return the inputs as an object
            return .object(scope.inputs)
        case .variable(let name):
            guard let v = scope.variable(name) else { throw ScopeError.undefinedVariable(name) }
            return v
        case .field(let base, let name):
            let v = try resolve(base, in: scope)
            return field(v, name) ?? .null
        case .optField(let base, let name):
            // Returns null for missing/null parent; never throws.
            let v = (try? resolve(base, in: scope)) ?? .null
            return field(v, name) ?? .null
        case .index(let base, let i):
            let v = try resolve(base, in: scope)
            return arrayIndex(v, i) ?? .null
        case .wildcard(let base):
            let v = try resolve(base, in: scope)
            return arrayWidened(v)
        }
    }

    // MARK: - Field / index helpers (handle null gracefully)

    private static func field(_ v: JSONValue, _ name: String) -> JSONValue? {
        // First-segment-after-input shortcut: PathExpr `.input` returns
        // an object built from `Scope.inputs` so a `.field(.input, "x")`
        // chain just looks like ordinary object access.
        if case .object(let o) = v {
            return o[name]
        }
        return nil
    }

    private static func arrayIndex(_ v: JSONValue, _ i: Int) -> JSONValue? {
        if case .array(let a) = v, i >= 0, i < a.count { return a[i] }
        return nil
    }

    /// Widens to an array. If `v` is already an array, returns as-is.
    /// Otherwise wraps in a 1-element array (ergonomic for "iterate this
    /// thing whether it's a scalar or a list" recipes).
    private static func arrayWidened(_ v: JSONValue) -> JSONValue {
        if case .array = v { return v }
        if case .null = v { return .array([]) }
        return .array([v])
    }
}
