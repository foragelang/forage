import Foundation

/// Lexer for `.forage` source. Produces a list of `Token`s with source
/// locations. Handles `//` line comments, `/* */` block comments,
/// strings with `\`-escapes, integer/double/bool/null/date literals, and
/// the multi-character operators `←`, `→`, `?.`, `[*]`.
public struct Lexer {

    public static let keywords: Set<String> = [
        "import",
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
        "records",          // used in expect blocks
        "count",            // used in expect blocks
        "typeName",         // expect blocks
        "initialURL",
        "loadMoreLabels",
        "extraLabels",
        "captureExtractions",
        "iterPath",
        "urlPattern",
        "withCookies",
        "as",
        "String", "Int", "Double", "Bool",
    ]

    public static let typeKeywords: Set<String> = ["String", "Int", "Double", "Bool"]

    public let source: String
    private var index: String.Index
    private var line: Int = 1
    private var column: Int = 1

    public init(source: String) {
        self.source = source
        self.index = source.startIndex
    }

    public mutating func tokenize() throws -> [Token] {
        var tokens: [Token] = []
        while !isEOF {
            skipWhitespaceAndComments()
            if isEOF { break }
            let startLoc = SourceLoc(line: line, column: column)
            let c = peek()

            if c == "{" { advance(); tokens.append(Token(kind: .lbrace, lexeme: "{", loc: startLoc)); continue }
            if c == "}" { advance(); tokens.append(Token(kind: .rbrace, lexeme: "}", loc: startLoc)); continue }
            if c == "(" { advance(); tokens.append(Token(kind: .lparen, lexeme: "(", loc: startLoc)); continue }
            if c == ")" { advance(); tokens.append(Token(kind: .rparen, lexeme: ")", loc: startLoc)); continue }
            if c == "[" {
                // Could be `[*]` or `[N]` or `[`
                if peek(offset: 1) == "*" && peek(offset: 2) == "]" {
                    advance(); advance(); advance()
                    tokens.append(Token(kind: .wildcard, lexeme: "[*]", loc: startLoc))
                    continue
                }
                advance()
                tokens.append(Token(kind: .lbracket, lexeme: "[", loc: startLoc))
                continue
            }
            if c == "]" { advance(); tokens.append(Token(kind: .rbracket, lexeme: "]", loc: startLoc)); continue }
            if c == "," { advance(); tokens.append(Token(kind: .comma, lexeme: ",", loc: startLoc)); continue }
            if c == ";" { advance(); tokens.append(Token(kind: .semicolon, lexeme: ";", loc: startLoc)); continue }
            if c == ":" { advance(); tokens.append(Token(kind: .colon, lexeme: ":", loc: startLoc)); continue }
            if c == "." { advance(); tokens.append(Token(kind: .dot, lexeme: ".", loc: startLoc)); continue }
            if c == "?" {
                if peek(offset: 1) == "." {
                    advance(); advance()
                    tokens.append(Token(kind: .qDot, lexeme: "?.", loc: startLoc))
                    continue
                }
                advance()
                tokens.append(Token(kind: .question, lexeme: "?", loc: startLoc))
                continue
            }
            if c == "|" { advance(); tokens.append(Token(kind: .pipe, lexeme: "|", loc: startLoc)); continue }
            if c == "=" { advance(); tokens.append(Token(kind: .equal, lexeme: "=", loc: startLoc)); continue }
            if c == ">" { advance(); tokens.append(Token(kind: .gt, lexeme: ">", loc: startLoc)); continue }
            if c == "<" { advance(); tokens.append(Token(kind: .lt, lexeme: "<", loc: startLoc)); continue }
            if c == "!" { advance(); tokens.append(Token(kind: .bang, lexeme: "!", loc: startLoc)); continue }
            if c == "←" { advance(); tokens.append(Token(kind: .arrow, lexeme: "←", loc: startLoc)); continue }
            if c == "→" { advance(); tokens.append(Token(kind: .caseArrow, lexeme: "→", loc: startLoc)); continue }

            if c == "\"" {
                let lit = try readStringLiteral(startLoc: startLoc)
                tokens.append(Token(kind: .stringLit(lit), lexeme: "\"\(lit)\"", loc: startLoc))
                continue
            }

            if c == "$" {
                advance() // consume $
                if isLetter(peek()) || peek() == "_" {
                    let name = readIdent()
                    if name == "input" {
                        tokens.append(Token(kind: .dollarInput, lexeme: "$input", loc: startLoc))
                    } else {
                        tokens.append(Token(kind: .dollarVariable(name), lexeme: "$\(name)", loc: startLoc))
                    }
                } else {
                    // Bare `$` — for current-value paths like `$.foo`
                    tokens.append(Token(kind: .dollarRoot, lexeme: "$", loc: startLoc))
                }
                continue
            }

            if isDigit(c) || (c == "-" && isDigit(peek(offset: 1))) {
                tokens.append(try readNumberOrDate(startLoc: startLoc))
                continue
            }

            if isLetter(c) || c == "_" {
                let name = readIdent()
                // `hub://...` — bare bone slug-literal token. Only triggered
                // when the identifier is exactly `hub` followed immediately
                // by `://`; otherwise `hub` stays an identifier (so a recipe
                // can name a variable `hub` if it really wants to).
                if name == "hub" && peek() == ":" && peek(offset: 1) == "/" && peek(offset: 2) == "/" {
                    advance(); advance(); advance() // consume `://`
                    var slug = ""
                    while !isEOF {
                        let ch = peek()
                        if isLetter(ch) || isDigit(ch) || ch == "-" || ch == "_" || ch == "/" {
                            slug.append(ch); advance()
                        } else {
                            break
                        }
                    }
                    tokens.append(Token(kind: .hubURL(slug), lexeme: "hub://\(slug)", loc: startLoc))
                    continue
                }
                if Lexer.keywords.contains(name) {
                    if name == "true" {
                        tokens.append(Token(kind: .boolLit(true), lexeme: name, loc: startLoc))
                    } else if name == "false" {
                        tokens.append(Token(kind: .boolLit(false), lexeme: name, loc: startLoc))
                    } else if name == "null" {
                        tokens.append(Token(kind: .nullLit, lexeme: name, loc: startLoc))
                    } else {
                        tokens.append(Token(kind: .keyword(name), lexeme: name, loc: startLoc))
                    }
                } else if let first = name.first, first.isUppercase {
                    tokens.append(Token(kind: .typeName(name), lexeme: name, loc: startLoc))
                } else {
                    tokens.append(Token(kind: .identifier(name), lexeme: name, loc: startLoc))
                }
                continue
            }

            throw LexError.unexpectedCharacter(c, loc: startLoc)
        }
        tokens.append(Token(kind: .eof, lexeme: "", loc: SourceLoc(line: line, column: column)))
        return tokens
    }

    // MARK: - Helpers

    private var isEOF: Bool { index >= source.endIndex }

    private func peek(offset: Int = 0) -> Character {
        var i = index
        var k = offset
        while k > 0 && i < source.endIndex {
            i = source.index(after: i); k -= 1
        }
        return i < source.endIndex ? source[i] : "\0"
    }

    private mutating func advance() {
        guard index < source.endIndex else { return }
        let c = source[index]
        if c == "\n" { line += 1; column = 1 } else { column += 1 }
        index = source.index(after: index)
    }

    private mutating func skipWhitespaceAndComments() {
        while !isEOF {
            let c = peek()
            if c == " " || c == "\t" || c == "\n" || c == "\r" {
                advance()
            } else if c == "/" && peek(offset: 1) == "/" {
                while !isEOF && peek() != "\n" { advance() }
            } else if c == "/" && peek(offset: 1) == "*" {
                advance(); advance()
                while !isEOF && !(peek() == "*" && peek(offset: 1) == "/") { advance() }
                if !isEOF { advance(); advance() }
            } else {
                break
            }
        }
    }

    private mutating func readIdent() -> String {
        var name = ""
        while !isEOF && (isLetter(peek()) || isDigit(peek()) || peek() == "_") {
            name.append(peek()); advance()
        }
        return name
    }

    private mutating func readStringLiteral(startLoc: SourceLoc) throws -> String {
        advance() // consume opening "
        var s = ""
        while !isEOF && peek() != "\"" {
            let c = peek()
            if c == "\\" {
                advance()
                let esc = peek()
                switch esc {
                case "\"": s.append("\""); advance()
                case "\\": s.append("\\"); advance()
                case "n": s.append("\n"); advance()
                case "t": s.append("\t"); advance()
                case "r": s.append("\r"); advance()
                default:
                    s.append(esc); advance()
                }
            } else {
                s.append(c); advance()
            }
        }
        if isEOF {
            throw LexError.unterminatedString(loc: startLoc)
        }
        advance() // consume closing "
        return s
    }

    private mutating func readNumberOrDate(startLoc: SourceLoc) throws -> Token {
        var raw = ""
        if peek() == "-" { raw.append("-"); advance() }
        while !isEOF && isDigit(peek()) { raw.append(peek()); advance() }
        // Check for date pattern YYYY-MM-DD where leading number is the year
        if !isEOF && peek() == "-" && raw.count >= 1 && Int(raw) != nil {
            // Look ahead — must match exact 4-2-2 digits-dash-digits-dash-digits
            // Save state, try to parse as date.
            let saved = (index, line, column)
            advance() // consume first -
            var monthStr = ""
            while !isEOF && isDigit(peek()) { monthStr.append(peek()); advance() }
            if peek() == "-" {
                advance()
                var dayStr = ""
                while !isEOF && isDigit(peek()) { dayStr.append(peek()); advance() }
                if let y = Int(raw), let m = Int(monthStr), let d = Int(dayStr),
                   monthStr.count == 2, dayStr.count == 2, raw.count == 4 {
                    return Token(kind: .dateLit(year: y, month: m, day: d), lexeme: "\(raw)-\(monthStr)-\(dayStr)", loc: startLoc)
                }
            }
            // Roll back
            (index, line, column) = saved
        }
        // Decimal?
        if !isEOF && peek() == "." && isDigit(peek(offset: 1)) {
            raw.append(".")
            advance()
            while !isEOF && isDigit(peek()) { raw.append(peek()); advance() }
            guard let d = Double(raw) else { throw LexError.invalidNumber(raw, loc: startLoc) }
            return Token(kind: .doubleLit(d), lexeme: raw, loc: startLoc)
        }
        guard let i = Int(raw) else { throw LexError.invalidNumber(raw, loc: startLoc) }
        return Token(kind: .intLit(i), lexeme: raw, loc: startLoc)
    }

    private func isDigit(_ c: Character) -> Bool { c >= "0" && c <= "9" }
    private func isLetter(_ c: Character) -> Bool {
        (c >= "a" && c <= "z") || (c >= "A" && c <= "Z")
    }
}

public enum LexError: Error, CustomStringConvertible {
    case unexpectedCharacter(Character, loc: SourceLoc)
    case unterminatedString(loc: SourceLoc)
    case invalidNumber(String, loc: SourceLoc)

    public var description: String {
        switch self {
        case .unexpectedCharacter(let c, let l):
            return "lexer: unexpected character '\(c)' at \(l.line):\(l.column)"
        case .unterminatedString(let l):
            return "lexer: unterminated string at \(l.line):\(l.column)"
        case .invalidNumber(let s, let l):
            return "lexer: invalid number '\(s)' at \(l.line):\(l.column)"
        }
    }
}
