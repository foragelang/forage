import Foundation

/// A string template — `"prefix-{$.x}-suffix"`. The runtime renders these
/// against the current scope (substituting each interpolation with the
/// JSON-stringified value the path resolves to).
public struct Template: Hashable, Sendable {
    public let parts: [TemplatePart]

    public init(parts: [TemplatePart]) {
        self.parts = parts
    }

    /// Construct from a literal string with no interpolations.
    public init(literal: String) {
        self.parts = [.literal(literal)]
    }
}

public enum TemplatePart: Hashable, Sendable {
    case literal(String)
    case interp(PathExpr)
}
