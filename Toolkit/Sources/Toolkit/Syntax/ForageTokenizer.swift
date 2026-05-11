import Foundation
import AppKit

/// Lightweight syntax highlighter for the Forage source editor. Independent
/// of `Forage.Lexer` — that tokenizer is built to power a parser, throwing on
/// any malformed input. The editor needs to highlight half-typed source
/// without throwing, so we scan tolerantly here and just classify spans.
///
/// Emits `Token` values with UTF-16 ranges (the same coordinate space
/// `NSTextStorage` uses), so consumers can call
/// `addAttribute(_:value:range:)` directly without converting indices.
struct ForageTokenizer {
    /// Token categories the editor cares about. The set is intentionally
    /// small — finer distinctions (operator vs. punctuation, recipe-decl
    /// keyword vs. body keyword) don't read differently to a human eye.
    enum Kind: Hashable, Sendable {
        case keyword
        case typeName
        case string
        case comment
        case number
        case dollar
        case op
        case punctuation
        case identifier
    }

    struct Token: Hashable, Sendable {
        let kind: Kind
        /// UTF-16 range, suitable for `NSAttributedString`.
        let range: NSRange
    }

    static let keywords: Set<String> = [
        "recipe", "engine", "http", "browser", "type", "enum", "input",
        "step", "method", "url", "headers", "body", "json", "form", "raw",
        "auth", "staticHeader", "htmlPrime", "extract", "regex", "groups",
        "paginate", "pageWithTotal", "untilEmpty", "cursor",
        "items", "total", "pageParam", "pageSize", "cursorPath", "cursorParam",
        "for", "in", "emit", "case", "of", "let", "where", "expect",
        "true", "false", "null",
        "observe", "browserPaginate", "scroll", "replay", "ageGate",
        "autoFill", "warmupClicks", "navigate", "until",
        "noProgressFor", "maxIterations", "iterationDelay", "seedFilter",
        "replayOverride", "captures", "match", "dismissals",
        "dob", "reloadAfter", "reloadAfterSubmit",
        "name", "value", "stepName", "nonceVar", "ajaxUrlVar",
        "pageZeroIndexed",
        "records", "count", "typeName",
        "initialURL",
        "loadMoreLabels", "extraLabels", "captureExtractions",
        "iterPath", "urlPattern", "withCookies", "as",
        "import",
    ]

    /// Capital-leading identifiers we want to highlight as type names. The
    /// recipe grammar treats them as type tokens; visually they read like
    /// types.
    private static let typeKeywords: Set<String> = ["String", "Int", "Double", "Bool"]

    /// Scan `source` (a Swift `String`) and return tokens with UTF-16
    /// ranges. Anything unclassified is omitted; the editor falls back to
    /// the default foreground color for those spans.
    static func tokenize(_ source: String) -> [Token] {
        var tokens: [Token] = []
        let scalars = Array(source.unicodeScalars)
        var i = 0
        var utf16Offset = 0

        // Cached UTF-16 lengths per code-point so we don't recount per match.
        // Most code-points are 1 UTF-16 unit, but `←` / `→` are also 1, and
        // emoji are 2.
        func u16Width(_ s: Unicode.Scalar) -> Int { s.utf16.count }

        func emit(_ kind: Kind, _ startU16: Int, _ endU16: Int) {
            guard endU16 > startU16 else { return }
            tokens.append(Token(kind: kind, range: NSRange(location: startU16, length: endU16 - startU16)))
        }

        while i < scalars.count {
            let c = scalars[i]
            let startU16 = utf16Offset

            // Line comment
            if c == "/" && i + 1 < scalars.count && scalars[i + 1] == "/" {
                var j = i
                var w = utf16Offset
                while j < scalars.count && scalars[j] != "\n" {
                    w += u16Width(scalars[j])
                    j += 1
                }
                emit(.comment, startU16, w)
                i = j
                utf16Offset = w
                continue
            }
            // Block comment
            if c == "/" && i + 1 < scalars.count && scalars[i + 1] == "*" {
                var j = i + 2
                var w = utf16Offset + 2
                while j < scalars.count {
                    if scalars[j] == "*" && j + 1 < scalars.count && scalars[j + 1] == "/" {
                        w += 2
                        j += 2
                        break
                    }
                    w += u16Width(scalars[j])
                    j += 1
                }
                emit(.comment, startU16, w)
                i = j
                utf16Offset = w
                continue
            }
            // String literal — tolerate unterminated by scanning to newline / EOF.
            if c == "\"" {
                var j = i + 1
                var w = utf16Offset + 1
                while j < scalars.count {
                    let s = scalars[j]
                    if s == "\\" && j + 1 < scalars.count {
                        w += u16Width(s) + u16Width(scalars[j + 1])
                        j += 2
                        continue
                    }
                    if s == "\"" {
                        w += 1
                        j += 1
                        break
                    }
                    if s == "\n" { break }
                    w += u16Width(s)
                    j += 1
                }
                emit(.string, startU16, w)
                i = j
                utf16Offset = w
                continue
            }
            // Number — int or double, signed-via-context (we don't fold `-`).
            if isDigit(c) {
                var j = i
                var w = utf16Offset
                while j < scalars.count, isDigit(scalars[j]) {
                    w += 1; j += 1
                }
                if j < scalars.count, scalars[j] == ".",
                   j + 1 < scalars.count, isDigit(scalars[j + 1]) {
                    w += 1; j += 1
                    while j < scalars.count, isDigit(scalars[j]) { w += 1; j += 1 }
                }
                emit(.number, startU16, w)
                i = j
                utf16Offset = w
                continue
            }
            // $-prefixed path heads
            if c == "$" {
                var j = i + 1
                var w = utf16Offset + 1
                while j < scalars.count, isIdentCont(scalars[j]) {
                    w += u16Width(scalars[j])
                    j += 1
                }
                emit(.dollar, startU16, w)
                i = j
                utf16Offset = w
                continue
            }
            // Identifier / keyword / type-name
            if isIdentStart(c) {
                var j = i
                var w = utf16Offset
                while j < scalars.count, isIdentCont(scalars[j]) {
                    w += u16Width(scalars[j])
                    j += 1
                }
                let lex = String(String.UnicodeScalarView(scalars[i..<j]))
                let kind: Kind
                if keywords.contains(lex) {
                    kind = .keyword
                } else if typeKeywords.contains(lex) {
                    kind = .typeName
                } else if let first = lex.first, first.isUppercase {
                    kind = .typeName
                } else {
                    kind = .identifier
                }
                emit(kind, startU16, w)
                i = j
                utf16Offset = w
                continue
            }
            // Operators we want to color
            if c == "←" || c == "→" || c == "|" || c == "?" {
                let width = u16Width(c)
                emit(.op, startU16, startU16 + width)
                i += 1
                utf16Offset += width
                continue
            }
            // Punctuation we want to dim (kept neutral so the editor doesn't
            // turn into a christmas tree)
            if "{}[]().,:;".unicodeScalars.contains(c) {
                emit(.punctuation, startU16, startU16 + 1)
                i += 1
                utf16Offset += 1
                continue
            }

            // Default: advance one scalar, no token.
            utf16Offset += u16Width(c)
            i += 1
        }
        return tokens
    }

    private static func isDigit(_ s: Unicode.Scalar) -> Bool {
        s >= "0" && s <= "9"
    }

    private static func isIdentStart(_ s: Unicode.Scalar) -> Bool {
        (s >= "A" && s <= "Z") || (s >= "a" && s <= "z") || s == "_"
    }

    private static func isIdentCont(_ s: Unicode.Scalar) -> Bool {
        isIdentStart(s) || isDigit(s)
    }
}

extension ForageTokenizer.Kind {
    /// Editor color for this token kind. Tuned for the default macOS light
    /// theme; the same hex values read fine on dark, but a follow-up could
    /// fork these per `NSAppearance`.
    var color: NSColor {
        switch self {
        case .keyword:      return NSColor(red: 0.10, green: 0.30, blue: 0.85, alpha: 1)
        case .typeName:     return NSColor(red: 0.20, green: 0.45, blue: 0.70, alpha: 1)
        case .string:       return NSColor(red: 0.15, green: 0.55, blue: 0.20, alpha: 1)
        case .comment:      return NSColor(white: 0.45, alpha: 1)
        case .number:       return NSColor(red: 0.85, green: 0.45, blue: 0.10, alpha: 1)
        case .dollar:       return NSColor(red: 0.55, green: 0.20, blue: 0.70, alpha: 1)
        case .op:           return NSColor(red: 0.90, green: 0.25, blue: 0.50, alpha: 1)
        case .punctuation:  return NSColor(white: 0.40, alpha: 1)
        case .identifier:   return NSColor.textColor
        }
    }

    var isBold: Bool {
        switch self {
        case .keyword: return true
        default: return false
        }
    }
}
