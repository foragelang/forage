import Foundation

/// One step in a recipe's HTTP graph. The step is a single request shape;
/// iteration over collections + pagination loops are wrapped around it by
/// the surrounding `Statement.forLoop` / `step.pagination`.
public struct HTTPStep: Hashable, Sendable {
    public let name: String
    public let request: HTTPRequest
    public let pagination: Pagination?

    public init(name: String, request: HTTPRequest, pagination: Pagination? = nil) {
        self.name = name
        self.request = request
        self.pagination = pagination
    }
}

public struct HTTPRequest: Hashable, Sendable {
    public let method: String
    public let url: Template
    public let headers: [(String, Template)]
    public let body: HTTPBody?

    public init(method: String, url: Template, headers: [(String, Template)] = [], body: HTTPBody? = nil) {
        self.method = method
        self.url = url
        self.headers = headers
        self.body = body
    }

    // Hashable: tuples in arrays aren't auto-Hashable, do it manually.
    public func hash(into hasher: inout Hasher) {
        hasher.combine(method)
        hasher.combine(url)
        for (k, v) in headers { hasher.combine(k); hasher.combine(v) }
        hasher.combine(body)
    }
    public static func == (lhs: Self, rhs: Self) -> Bool {
        guard lhs.method == rhs.method, lhs.url == rhs.url, lhs.body == rhs.body else { return false }
        guard lhs.headers.count == rhs.headers.count else { return false }
        for (a, b) in zip(lhs.headers, rhs.headers) where a.0 != b.0 || a.1 != b.1 { return false }
        return true
    }
}

public indirect enum HTTPBody: Hashable, Sendable {
    /// JSON-encoded body. The runtime renders `BodyValue` against the scope.
    case jsonObject([HTTPBodyKV])
    /// `application/x-www-form-urlencoded` body. Keys may contain brackets
    /// (`wizard_data[retailer_id]`); values are templates.
    case form([(String, Template)])
    /// Raw text body, rendered from a template.
    case raw(Template)

    public func hash(into hasher: inout Hasher) {
        switch self {
        case .jsonObject(let kvs):
            hasher.combine(0); hasher.combine(kvs)
        case .form(let kvs):
            hasher.combine(1)
            for (k, v) in kvs { hasher.combine(k); hasher.combine(v) }
        case .raw(let t):
            hasher.combine(2); hasher.combine(t)
        }
    }
    public static func == (lhs: Self, rhs: Self) -> Bool {
        switch (lhs, rhs) {
        case (.jsonObject(let a), .jsonObject(let b)): return a == b
        case (.form(let a), .form(let b)):
            guard a.count == b.count else { return false }
            for (x, y) in zip(a, b) where x.0 != y.0 || x.1 != y.1 { return false }
            return true
        case (.raw(let a), .raw(let b)): return a == b
        default: return false
        }
    }
}

/// Key-value entry in a JSON object body.
public struct HTTPBodyKV: Hashable, Sendable {
    public let key: String
    public let value: BodyValue
    public init(key: String, value: BodyValue) {
        self.key = key; self.value = value
    }
}

/// Value position in a JSON body: scalar (templated string / number / bool),
/// nested object, array. `templateString` is for "{$x}" interpolation;
/// `literal` is for numeric/boolean/null constants.
public indirect enum BodyValue: Hashable, Sendable {
    case templateString(Template)
    case literal(JSONValue)
    case path(PathExpr)                   // `key: $input.x` — substitute the resolved value
    case object([HTTPBodyKV])
    case array([BodyValue])
    /// `case $x of { A → ...; B → ... }` inside a body — selects one BodyValue
    /// based on the iteration variable's enum value.
    case caseOf(scrutinee: PathExpr, branches: [(label: String, value: BodyValue)])

    public func hash(into hasher: inout Hasher) {
        switch self {
        case .templateString(let t): hasher.combine(0); hasher.combine(t)
        case .literal(let j):        hasher.combine(1); hasher.combine(j)
        case .path(let p):           hasher.combine(2); hasher.combine(p)
        case .object(let kvs):       hasher.combine(3); hasher.combine(kvs)
        case .array(let xs):         hasher.combine(4); hasher.combine(xs)
        case .caseOf(let s, let bs):
            hasher.combine(5); hasher.combine(s)
            for (l, v) in bs { hasher.combine(l); hasher.combine(v) }
        }
    }
    public static func == (lhs: Self, rhs: Self) -> Bool {
        switch (lhs, rhs) {
        case (.templateString(let a), .templateString(let b)): return a == b
        case (.literal(let a), .literal(let b)): return a == b
        case (.path(let a), .path(let b)): return a == b
        case (.object(let a), .object(let b)): return a == b
        case (.array(let a), .array(let b)): return a == b
        case (.caseOf(let s1, let b1), .caseOf(let s2, let b2)):
            guard s1 == s2, b1.count == b2.count else { return false }
            for (x, y) in zip(b1, b2) where x.label != y.label || x.value != y.value { return false }
            return true
        default: return false
        }
    }
}
