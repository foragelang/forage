import Foundation

/// A recipe-declared type. Recipes ship their own type catalog; forage
/// doesn't pre-define `Product` / `Variant` / etc.
public struct RecipeType: Hashable, Sendable {
    public let name: String
    public let fields: [RecipeField]

    public init(name: String, fields: [RecipeField]) {
        self.name = name
        self.fields = fields
    }

    public func field(_ name: String) -> RecipeField? {
        fields.first(where: { $0.name == name })
    }
}

public struct RecipeField: Hashable, Sendable {
    public let name: String
    public let type: FieldType
    /// True when the field is declared as `name: Type?` rather than `name: Type`.
    /// Required fields cause a validation error if a recipe doesn't bind them
    /// in an emit block (or binds them to `null`).
    public let optional: Bool

    public init(name: String, type: FieldType, optional: Bool) {
        self.name = name
        self.type = type
        self.optional = optional
    }
}

/// Recipe field types. References to other recipe-declared types and enums
/// are by name; resolved at validation time, not in the parser.
public indirect enum FieldType: Hashable, Sendable {
    case string
    case int
    case double
    case bool
    case array(FieldType)
    case record(String)    // typeName
    case enumRef(String)   // enum name
}

/// A recipe-declared enum, e.g. `enum MenuType { RECREATIONAL, MEDICAL }`.
public struct RecipeEnum: Hashable, Sendable {
    public let name: String
    public let variants: [String]

    public init(name: String, variants: [String]) {
        self.name = name
        self.variants = variants
    }
}

/// A consumer-supplied input declaration. `weed-prices` for example provides
/// `storeId` for Sweed recipes; the runtime validates the supplied inputs
/// against these decls before running.
public struct InputDecl: Hashable, Sendable {
    public let name: String
    public let type: FieldType
    public let optional: Bool

    public init(name: String, type: FieldType, optional: Bool = false) {
        self.name = name
        self.type = type
        self.optional = optional
    }
}
