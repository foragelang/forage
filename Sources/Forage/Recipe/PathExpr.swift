import Foundation

/// A path expression — `$.x.y?.z[*]`, `$input.storeId`, `$cat.id`, `$secret.password`, etc.
/// The runtime evaluates these against the current `Scope` to produce a
/// `JSONValue` (or a list of values when `[*]` widens).
public indirect enum PathExpr: Hashable, Sendable {
    /// `$` — the current value at the binding site (e.g. inside an emit
    /// inside a `for` loop, `$` is the loop's current value).
    case current
    /// `$input` — the recipe's input scope.
    case input
    /// `$secret.<name>` — resolved at run time via the host's `SecretResolver`
    /// (env var on the CLI, Keychain in the Toolkit). Recipe text never
    /// contains the resolved value; the engine redacts it from diagnostics.
    case secret(String)
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

    /// All `$secret.<name>` references this expression mentions, transitively.
    public var referencedSecrets: Set<String> {
        switch self {
        case .secret(let n): return [n]
        case .current, .input, .variable: return []
        case .field(let inner, _), .optField(let inner, _), .index(let inner, _), .wildcard(let inner):
            return inner.referencedSecrets
        }
    }
}
