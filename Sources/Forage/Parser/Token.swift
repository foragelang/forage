import Foundation

/// Source location for diagnostics.
public struct SourceLoc: Hashable, Sendable {
    public var line: Int
    public var column: Int
    public init(line: Int, column: Int) { self.line = line; self.column = column }
}

public struct Token: Hashable, Sendable {
    public let kind: TokenKind
    public let lexeme: String
    public let loc: SourceLoc
    public init(kind: TokenKind, lexeme: String, loc: SourceLoc) {
        self.kind = kind; self.lexeme = lexeme; self.loc = loc
    }
}

public enum TokenKind: Hashable, Sendable {
    // Punctuation / brackets
    case lbrace, rbrace
    case lparen, rparen
    case lbracket, rbracket
    case comma, semicolon, colon, dot
    case question
    case qDot                  // ?.
    case wildcard              // [*]  (whole token; the lexer recognizes the trio)
    case pipe                  // |
    case arrow                 // ←   (left-arrow, binding)
    case caseArrow             // →   (right-arrow, case branch)
    case equal                 // =
    case gt                    // >
    case lt                    // <
    case bang                  // !

    // Path-expression heads
    case dollarRoot            // $   (just `$`, normally followed by .)
    case dollarInput           // $input
    case dollarVariable(String) // $<ident>

    // Literals
    case stringLit(String)
    case intLit(Int)
    case doubleLit(Double)
    case boolLit(Bool)
    case nullLit
    case dateLit(year: Int, month: Int, day: Int)

    // `hub://<slug>` token. Value is the raw slug after the prefix
    // (e.g. `sample-recipe` or `alice/awesome-recipe`).
    case hubURL(String)

    // Identifier (lowercased) and type-name (capitalized).
    // The parser disambiguates by context, so we use one identifier token here.
    case identifier(String)
    case typeName(String)

    // Keyword. The parser matches by lexeme; using one variant keeps the
    // token enum small and the lexer simple.
    case keyword(String)

    case eof
}
