import Foundation

/// `Snapshot` is the only fixed wrapper forage owns. The records inside it
/// are typed by *recipe-declared* types — forage doesn't hardcode anything
/// about Dispensaries, Products, or any other domain.
///
/// A recipe declares its own types (e.g. `type Product { name: String; … }`)
/// in the recipe file. The runtime collects records emitted under those
/// type names into the snapshot. Downstream consumers (weed-prices, future
/// non-cannabis consumers, etc.) read records by `typeName` and translate
/// to their own storage. Two recipes for two different domains can ship in
/// the same forage runtime without sharing any types.
///
/// Keeping forage domain-agnostic means:
/// - The catalog grows by recipes adding new types, not by us patching the engine.
/// - A consumer that doesn't know a recipe's types just gets unrecognized
///   records (which it can ignore, log, or surface as opaque).
/// - The DSL grammar gets a type-declaration construct (handled by the parser),
///   not a fixed schema we ship.
///
/// Consumers wire this up by knowing — out of band — which recipes emit
/// which types and how those map into their own storage. e.g. weed-prices
/// expects recipes to emit `Product` / `Variant` / `PriceObservation`
/// records shaped like its SQLite schema, and refuses recipes that don't.

// MARK: - ScrapedRecord

/// One emitted record. `typeName` is the recipe-declared type; `fields` are
/// the field-name → typed-value bindings from the recipe's extraction.
public struct ScrapedRecord: Hashable, Sendable {
    public let typeName: String
    public let fields: [String: TypedValue]

    public init(typeName: String, fields: [String: TypedValue]) {
        self.typeName = typeName
        self.fields = fields
    }

    /// Convenience: read a field by name with optional fall-through if missing.
    public subscript(_ field: String) -> TypedValue? {
        fields[field]
    }
}

// MARK: - TypedValue

/// Field value in a `ScrapedRecord`. JSON-shaped primitives plus nested
/// records (when a recipe-declared type embeds children — e.g. a Product
/// type might have a `variants: [Variant]` field). Recipes can also choose
/// to keep relationships flat (Variants as separate top-level records keyed
/// by externalId) — both shapes are expressible.
public indirect enum TypedValue: Hashable, Sendable {
    case null
    case bool(Bool)
    case int(Int)
    case double(Double)
    case string(String)
    case array([TypedValue])
    case record(ScrapedRecord)
}

// MARK: - Snapshot

/// The full output of one recipe run. The runtime hands this to the
/// downstream consumer; the consumer interprets records by `typeName`.
public struct Snapshot: Hashable, Sendable {
    public let records: [ScrapedRecord]
    public let observedAt: Date

    public init(records: [ScrapedRecord] = [], observedAt: Date = Date()) {
        self.records = records
        self.observedAt = observedAt
    }

    /// Records filtered to a specific recipe-declared type.
    public func records(of typeName: String) -> [ScrapedRecord] {
        records.filter { $0.typeName == typeName }
    }
}
