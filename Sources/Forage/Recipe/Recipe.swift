import Foundation

/// A parsed recipe — produced by the parser (Phase C), consumed by the
/// runtime (this phase). The structure mirrors the `.forage` file layout:
/// declarations at the top, followed by an ordered body of statements
/// (steps, emits, for-loops). Browser-engine recipes also carry a
/// `BrowserConfig` with their navigation / dismissal / pagination plan.
public struct Recipe: Hashable, Sendable {
    public let name: String
    public let engineKind: EngineKind
    public let types: [RecipeType]
    public let enums: [RecipeEnum]
    public let inputs: [InputDecl]
    public let auth: AuthStrategy?
    public let body: [Statement]
    public let browser: BrowserConfig?
    public let expectations: [Expectation]
    /// Top-level `import hub://<slug>` directives, in source order. These
    /// are unresolved pointers — `RecipeImporter.flatten` is the runtime
    /// pass that fetches and merges the imported recipes into a single
    /// flattened `Recipe`.
    public let imports: [HubRecipeRef]

    public init(
        name: String,
        engineKind: EngineKind,
        types: [RecipeType] = [],
        enums: [RecipeEnum] = [],
        inputs: [InputDecl] = [],
        auth: AuthStrategy? = nil,
        body: [Statement] = [],
        browser: BrowserConfig? = nil,
        expectations: [Expectation] = [],
        imports: [HubRecipeRef] = []
    ) {
        self.name = name
        self.engineKind = engineKind
        self.types = types
        self.enums = enums
        self.inputs = inputs
        self.auth = auth
        self.body = body
        self.browser = browser
        self.expectations = expectations
        self.imports = imports
    }

    public func type(_ name: String) -> RecipeType? {
        types.first(where: { $0.name == name })
    }
    public func recipeEnum(_ name: String) -> RecipeEnum? {
        enums.first(where: { $0.name == name })
    }
    public func input(_ name: String) -> InputDecl? {
        inputs.first(where: { $0.name == name })
    }
}

public enum EngineKind: String, Hashable, Sendable {
    case http
    case browser
}

/// One body statement. Recipes mix steps, emissions, and for-loops at any
/// level of nesting.
public indirect enum Statement: Hashable, Sendable {
    case step(HTTPStep)
    case emit(Emission)
    case forLoop(variable: String, collection: PathExpr, body: [Statement])
}

/// Recipe author-declared expectation, e.g.
/// `expect { records.where(typeName == "Product").count >= 100 }`.
/// Phase D's validator checks expression well-formedness; the runtime
/// checks actual records against expectations after a run finishes.
public struct Expectation: Hashable, Sendable {
    public let kind: ExpectationKind
    public init(_ kind: ExpectationKind) { self.kind = kind }
}

public enum ExpectationKind: Hashable, Sendable {
    /// `records.where(typeName == "X").count <op> N`
    case recordCount(typeName: String, op: ComparisonOp, value: Int)
}

public enum ComparisonOp: String, Hashable, Sendable {
    case ge = ">="
    case gt = ">"
    case le = "<="
    case lt = "<"
    case eq = "=="
    case ne = "!="
}
