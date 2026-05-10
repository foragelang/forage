import Foundation

/// A path expression — `$.x.y?.z[*]`, `$input.storeId`, `$cat.id`, etc.
/// The runtime evaluates these against the current `Scope` to produce a
/// `JSONValue` (or a list of values when `[*]` widens).
public indirect enum PathExpr: Hashable, Sendable {
    /// `$` — the current value at the binding site (e.g. inside an emit
    /// inside a `for` loop, `$` is the loop's current value).
    case current
    /// `$input` — the recipe's input scope.
    case input
    /// `$<name>` — anything else introduced by a `for` binding or step result.
    case variable(String)
    /// `<base>.<field>`
    case field(PathExpr, String)
    /// `<base>?.<field>` — yields null on missing/null parent
    case optField(PathExpr, String)
    /// `<base>[N]`
    case index(PathExpr, Int)
    /// `<base>[*]` — wildcard, broadens to a list
    case wildcard(PathExpr)
}
