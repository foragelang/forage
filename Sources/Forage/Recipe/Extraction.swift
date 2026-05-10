import Foundation

/// An emit block — `emit Product { name ← $.name; brand ← $.brand?.name }`.
/// Produces one `ScrapedRecord` per execution; the runtime accumulates them
/// into the final `Snapshot`.
public struct Emission: Hashable, Sendable {
    public let typeName: String
    public let bindings: [FieldBinding]

    public init(typeName: String, bindings: [FieldBinding]) {
        self.typeName = typeName
        self.bindings = bindings
    }
}

public struct FieldBinding: Hashable, Sendable {
    public let fieldName: String
    public let expr: ExtractionExpr

    public init(fieldName: String, expr: ExtractionExpr) {
        self.fieldName = fieldName
        self.expr = expr
    }
}

/// Right-hand side of a field binding. The runtime evaluates these against
/// the current scope to produce a `TypedValue`.
public indirect enum ExtractionExpr: Hashable, Sendable {
    /// A bare path: `$.x.y?.z`
    case path(PathExpr)
    /// `<expr> | <transform> | <transform>` — left-to-right pipeline
    case pipe(ExtractionExpr, [TransformCall])
    /// `case $x of { A → expr; B → expr }` — switch on the scrutinee's enum value
    case caseOf(scrutinee: PathExpr, branches: [(label: String, expr: ExtractionExpr)])
    /// `<expr> | map(<emit>)` — map a list to a list of typed records
    case mapTo(PathExpr, emission: Emission)
    /// Inline literal — `"sweed"`, `42`, `true`
    case literal(JSONValue)
    /// Template string with interpolations — `"{$.id}:{$weight}"`
    case template(Template)
    /// Function-call-shaped transform with multiple args, e.g. `coalesce(a, b)`
    /// or `normalizeOzToGrams($variant.unitSize?.unitAbbr)`.
    case call(name: String, args: [ExtractionExpr])

    public func hash(into hasher: inout Hasher) {
        switch self {
        case .path(let p):           hasher.combine(0); hasher.combine(p)
        case .pipe(let e, let ts):   hasher.combine(1); hasher.combine(e); hasher.combine(ts)
        case .caseOf(let s, let bs):
            hasher.combine(2); hasher.combine(s)
            for (l, e) in bs { hasher.combine(l); hasher.combine(e) }
        case .mapTo(let p, let e):   hasher.combine(3); hasher.combine(p); hasher.combine(e)
        case .literal(let v):        hasher.combine(4); hasher.combine(v)
        case .template(let t):       hasher.combine(5); hasher.combine(t)
        case .call(let n, let xs):   hasher.combine(6); hasher.combine(n); hasher.combine(xs)
        }
    }
    public static func == (lhs: Self, rhs: Self) -> Bool {
        switch (lhs, rhs) {
        case (.path(let a), .path(let b)): return a == b
        case (.pipe(let e1, let t1), .pipe(let e2, let t2)): return e1 == e2 && t1 == t2
        case (.caseOf(let s1, let b1), .caseOf(let s2, let b2)):
            guard s1 == s2, b1.count == b2.count else { return false }
            for (x, y) in zip(b1, b2) where x.label != y.label || x.expr != y.expr { return false }
            return true
        case (.mapTo(let p1, let e1), .mapTo(let p2, let e2)): return p1 == p2 && e1 == e2
        case (.literal(let a), .literal(let b)): return a == b
        case (.template(let a), .template(let b)): return a == b
        case (.call(let n1, let a1), .call(let n2, let a2)): return n1 == n2 && a1 == a2
        default: return false
        }
    }
}

public struct TransformCall: Hashable, Sendable {
    public let name: String
    /// Optional positional args. Most transforms take none (`titleCase`,
    /// `parseSize`); some take args (`coalesce(a, b)`, `default("X")`).
    public let args: [ExtractionExpr]

    public init(name: String, args: [ExtractionExpr] = []) {
        self.name = name
        self.args = args
    }
}
