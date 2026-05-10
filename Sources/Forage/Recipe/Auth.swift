import Foundation

/// HTTP-engine authentication strategies. Each named variant grows the
/// runtime's auth vocabulary by deliberate Swift extension; recipes pick.
public enum AuthStrategy: Hashable, Sendable {
    /// A static header on every subsequent request, value rendered from
    /// the recipe scope (typically the consumer-supplied `$input.storeId`).
    case staticHeader(name: String, value: Template)

    /// "Prime" the session by fetching an HTML page and (a) capturing whatever
    /// cookies the server sets, (b) extracting one or more values from the
    /// page body via regex. The captured values become scope variables for
    /// subsequent steps. Used by Leafbridge / WordPress-AJAX style platforms.
    ///
    /// `stepName` references a step in the same recipe whose response is HTML
    /// and that runs *before* any auth-needing step. The runtime serializes
    /// the auth step before the regular step graph runs.
    case htmlPrime(stepName: String, capturedVars: [HtmlPrimeVar])
}

/// Variable captured from an HTML prime step's body via regex group.
public struct HtmlPrimeVar: Hashable, Sendable {
    public let varName: String      // `$ajaxNonce` etc.
    public let regexPattern: String
    public let groupIndex: Int      // 1-based capture group

    public init(varName: String, regexPattern: String, groupIndex: Int) {
        self.varName = varName
        self.regexPattern = regexPattern
        self.groupIndex = groupIndex
    }
}
