import Foundation

/// A string template — `"prefix-{$.x}-suffix"` or `"price_{$weight | snake}"`.
/// The runtime renders these against the current scope, substituting each
/// interpolation with the JSON-stringified value the expression resolves to.
/// Interpolations are full extraction expressions, so they support pipelines
/// (`{$x | transform}`), function-call transforms (`{coalesce(a, b)}`), and
/// case-of branches in addition to bare paths.
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
    case interp(ExtractionExpr)
}
