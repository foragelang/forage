import Foundation

/// Custom Codable conformance for `TypedValue` / `ScrapedRecord` / `Snapshot`.
/// JSON is used as the wire format; the recipe-author-facing snapshot file
/// can be either YAML or JSON, but the encoder/decoder works on JSON
/// internally and a YAML pass converts at I/O time.
///
/// Encoding shape:
///   Snapshot:
///     { "observedAt": ISO8601 string, "records": [Record, …] }
///   ScrapedRecord:
///     { "_typeName": "Foo", "fields": { … } }
///   TypedValue:
///     null / bool / int / double / string / array / nested record object

extension TypedValue: Codable {
    public func encode(to encoder: Encoder) throws {
        var c = encoder.singleValueContainer()
        switch self {
        case .null:           try c.encodeNil()
        case .bool(let b):    try c.encode(b)
        case .int(let i):     try c.encode(i)
        case .double(let d):  try c.encode(d)
        case .string(let s):  try c.encode(s)
        case .array(let a):   try c.encode(a)
        case .record(let r):  try c.encode(r)
        }
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.singleValueContainer()
        if c.decodeNil() { self = .null; return }
        if let b = try? c.decode(Bool.self) { self = .bool(b); return }
        if let i = try? c.decode(Int.self) { self = .int(i); return }
        if let d = try? c.decode(Double.self) { self = .double(d); return }
        if let s = try? c.decode(String.self) { self = .string(s); return }
        if let a = try? c.decode([TypedValue].self) { self = .array(a); return }
        if let r = try? c.decode(ScrapedRecord.self) { self = .record(r); return }
        throw DecodingError.dataCorruptedError(in: c, debugDescription: "Unrecognized TypedValue")
    }
}

extension ScrapedRecord: Codable {
    enum CodingKeys: String, CodingKey {
        case typeName = "_typeName"
        case fields
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        let typeName = try c.decode(String.self, forKey: .typeName)
        let fields = try c.decode([String: TypedValue].self, forKey: .fields)
        self.init(typeName: typeName, fields: fields)
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(typeName, forKey: .typeName)
        try c.encode(fields, forKey: .fields)
    }
}

extension Snapshot: Codable {
    enum CodingKeys: String, CodingKey {
        case observedAt
        case records
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        let when = try c.decode(Date.self, forKey: .observedAt)
        let records = try c.decode([ScrapedRecord].self, forKey: .records)
        self.init(records: records, observedAt: when)
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(observedAt, forKey: .observedAt)
        try c.encode(records, forKey: .records)
    }
}

/// JSON helpers for snapshot round-tripping. We use JSON (not YAML) for
/// fixture and snapshot files for now — minimal dependencies, trivial
/// hand-edit. A YAML output mode can layer on later.
public enum SnapshotIO {
    public static func encode(_ snapshot: Snapshot, pretty: Bool = true) throws -> Data {
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        encoder.outputFormatting = pretty
            ? [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes]
            : [.sortedKeys, .withoutEscapingSlashes]
        return try encoder.encode(snapshot)
    }

    public static func decode(_ data: Data) throws -> Snapshot {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        return try decoder.decode(Snapshot.self, from: data)
    }
}
