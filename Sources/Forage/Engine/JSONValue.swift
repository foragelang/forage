import Foundation
import SwiftSoup

/// Runtime value flowing through extraction. Decoded from an HTTP response
/// body, constructed from a recipe literal, or produced by a transform.
/// Distinct from `TypedValue` (the recipe's emit-output type): JSON has
/// dicts and undifferentiated nulls; TypedValue has typed nulls, named
/// records, and the recipe's enums.
///
/// The `.node` case wraps a parsed HTML/XML element so the same path-and-
/// pipe extraction works against server-rendered pages alongside JSON
/// responses. Nodes are produced by the `parseHtml` transform (or by an
/// HTTP response with `Content-Type: text/html` once an explicit pipe
/// is applied). Nodes don't survive a JSON encode/decode round-trip —
/// they serialize as their outerHTML string, and decoders never produce
/// `.node` because the wire format doesn't carry the discriminator.
public indirect enum JSONValue: Hashable, @unchecked Sendable {
    case null
    case bool(Bool)
    case int(Int)
    case double(Double)
    case string(String)
    case array([JSONValue])
    case object([String: JSONValue])
    case node(HTMLNode)
}

/// A parsed HTML/XML element queryable by CSS selectors. Wraps a
/// `SwiftSoup.Element` reference; equality and hashing go through the
/// element's outerHTML so two nodes with structurally identical markup
/// compare equal even if they came from different parses.
public struct HTMLNode: Hashable, @unchecked Sendable {
    public let element: SwiftSoup.Element

    public init(_ element: SwiftSoup.Element) {
        self.element = element
    }

    public static func == (lhs: HTMLNode, rhs: HTMLNode) -> Bool {
        let l = (try? lhs.element.outerHtml()) ?? ""
        let r = (try? rhs.element.outerHtml()) ?? ""
        return l == r
    }

    public func hash(into hasher: inout Hasher) {
        hasher.combine((try? element.outerHtml()) ?? "")
    }
}

extension JSONValue: Codable {
    public func encode(to encoder: Encoder) throws {
        var c = encoder.singleValueContainer()
        switch self {
        case .null:           try c.encodeNil()
        case .bool(let b):    try c.encode(b)
        case .int(let i):     try c.encode(i)
        case .double(let d):  try c.encode(d)
        case .string(let s):  try c.encode(s)
        case .array(let a):   try c.encode(a)
        case .object(let o):  try c.encode(o)
        case .node(let n):    try c.encode((try? n.element.outerHtml()) ?? "")
        }
    }

    /// **Round-trip caveats:**
    ///
    /// 1. `.double(N.0)` decodes back as `.int(N)`. JSON has one numeric
    ///    tower and the decoder tries `Int` before `Double`, so a
    ///    whole-valued `Double` (e.g. `39.0` written by `JSONEncoder`)
    ///    comes back as `.int(39)`. Recipes don't care — downstream
    ///    coercions handle both — but any caller relying on the `.int`
    ///    vs `.double` distinction surviving an encode/decode (e.g.
    ///    `meta.inputs` round-trip via `ArchiveMeta`) needs to plan
    ///    around this.
    /// 2. `.node` encodes as its outerHTML string. Decoding never
    ///    produces `.node` — the wire format has no discriminator for
    ///    "this string is parsed HTML." Nodes are transient runtime
    ///    values; if you need them downstream, re-parse via
    ///    `parseHtml`.
    public init(from decoder: Decoder) throws {
        let c = try decoder.singleValueContainer()
        if c.decodeNil() { self = .null; return }
        if let b = try? c.decode(Bool.self) { self = .bool(b); return }
        if let i = try? c.decode(Int.self) { self = .int(i); return }
        if let d = try? c.decode(Double.self) { self = .double(d); return }
        if let s = try? c.decode(String.self) { self = .string(s); return }
        if let a = try? c.decode([JSONValue].self) { self = .array(a); return }
        if let o = try? c.decode([String: JSONValue].self) { self = .object(o); return }
        throw DecodingError.dataCorruptedError(in: c, debugDescription: "Unrecognized JSONValue")
    }
}

extension JSONValue {
    /// Decode from `Data` (typically an HTTP response body) as JSON.
    /// Throws on malformed JSON — callers that want a graceful fallback
    /// for non-JSON bodies should use `decodeBody(_:contentType:)`.
    public static func decode(_ data: Data) throws -> JSONValue {
        let any = try JSONSerialization.jsonObject(with: data, options: [.fragmentsAllowed])
        return JSONValue(fromAny: any)
    }

    /// Decode an HTTP response body using its Content-Type as a hint.
    /// JSON bodies parse normally; HTML/XML/text bodies fall back to
    /// `.string(body)` so the recipe can pipe through `parseHtml` or
    /// other transforms. Binary or undecodable bodies produce
    /// `.string("")` rather than throwing.
    public static func decodeBody(_ data: Data, contentType: String?) -> JSONValue {
        let ct = contentType?.lowercased() ?? ""
        // Explicit HTML/XML/text: hand back as a string for the recipe
        // to parse with `parseHtml`. Don't even try JSON.
        if ct.contains("html") || ct.contains("xml") || ct.hasPrefix("text/") {
            return .string(String(data: data, encoding: .utf8) ?? "")
        }
        // Try JSON; fall back to string on parse failure (some servers
        // ship JSON without a proper Content-Type).
        if let parsed = try? JSONValue.decode(data) { return parsed }
        return .string(String(data: data, encoding: .utf8) ?? "")
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

    /// Encode to a JSON-encodable Foundation Any. `.node` values
    /// project to their outerHTML so they survive serialization paths
    /// that go through `asAny → JSONSerialization`.
    public var asAny: Any {
        switch self {
        case .null: return NSNull()
        case .bool(let b): return b
        case .int(let i): return i
        case .double(let d): return d
        case .string(let s): return s
        case .array(let a): return a.map(\.asAny)
        case .object(let o): return o.mapValues(\.asAny)
        case .node(let n): return (try? n.element.outerHtml()) ?? ""
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
    /// `.node` values lift to their outerHTML string — recipes shouldn't
    /// emit raw nodes (the snapshot doesn't carry DOM state) and the string
    /// is the most useful fallback if a recipe accidentally does.
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
        case .node(let n):
            return .string((try? n.element.outerHtml()) ?? "")
        }
    }
}
