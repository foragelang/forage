import Foundation

/// Wire-format value — decoded from an HTTP response body or constructed
/// from a literal in a recipe. Distinct from `TypedValue` (the recipe's
/// emit-output type): JSON has dicts and undifferentiated nulls;
/// TypedValue has typed nulls, named records, and the recipe's enums.
public indirect enum JSONValue: Hashable, Sendable {
    case null
    case bool(Bool)
    case int(Int)
    case double(Double)
    case string(String)
    case array([JSONValue])
    case object([String: JSONValue])
}

extension JSONValue {
    /// Decode from `Data` (typically an HTTP response body).
    public static func decode(_ data: Data) throws -> JSONValue {
        let any = try JSONSerialization.jsonObject(with: data, options: [.fragmentsAllowed])
        return JSONValue(fromAny: any)
    }

    public init(fromAny v: Any) {
        if v is NSNull { self = .null; return }
        if let b = v as? Bool { self = .bool(b); return }
        if let i = v as? Int { self = .int(i); return }
        if let d = v as? Double { self = .double(d); return }
        if let s = v as? String { self = .string(s); return }
        if let a = v as? [Any] { self = .array(a.map { JSONValue(fromAny: $0) }); return }
        if let o = v as? [String: Any] {
            var dict: [String: JSONValue] = [:]
            for (k, vv) in o { dict[k] = JSONValue(fromAny: vv) }
            self = .object(dict); return
        }
        if let n = v as? NSNumber {
            // Differentiate Bool from numeric NSNumber. CFBooleanGetTypeID lets us check.
            if CFGetTypeID(n) == CFBooleanGetTypeID() {
                self = .bool(n.boolValue); return
            }
            // Prefer Int when representable; else Double.
            if n.stringValue.contains(".") || n.stringValue.contains("e") {
                self = .double(n.doubleValue); return
            }
            self = .int(n.intValue); return
        }
        self = .null
    }

    /// Encode to a JSON-encodable Foundation Any.
    public var asAny: Any {
        switch self {
        case .null: return NSNull()
        case .bool(let b): return b
        case .int(let i): return i
        case .double(let d): return d
        case .string(let s): return s
        case .array(let a): return a.map(\.asAny)
        case .object(let o): return o.mapValues(\.asAny)
        }
    }

    /// Encode to UTF-8 JSON `Data`.
    public func encode() throws -> Data {
        try JSONSerialization.data(withJSONObject: asAny, options: [.fragmentsAllowed, .sortedKeys])
    }
}

extension JSONValue {
    /// Lift to TypedValue for emission. Object → record requires a typeName,
    /// which the runtime supplies; this helper is for primitive-level conversion.
    public func toTypedValue(typeName: String? = nil) -> TypedValue {
        switch self {
        case .null: return .null
        case .bool(let b): return .bool(b)
        case .int(let i): return .int(i)
        case .double(let d): return .double(d)
        case .string(let s): return .string(s)
        case .array(let a): return .array(a.map { $0.toTypedValue() })
        case .object(let o):
            // Without a type name we just pass through as a flat record with a
            // synthetic name. Recipes don't normally do this.
            let fields = o.mapValues { $0.toTypedValue() }
            return .record(ScrapedRecord(typeName: typeName ?? "_anonymous", fields: fields))
        }
    }
}
