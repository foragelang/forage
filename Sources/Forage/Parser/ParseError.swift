import Foundation

public enum ParseError: Error, CustomStringConvertible {
    case unexpected(Token, expected: String)
    case unknownDeclaration(String, loc: SourceLoc)
    case unknownPaginationStrategy(String, loc: SourceLoc)
    case unknownAuthStrategy(String, loc: SourceLoc)
    case unknownFieldType(String, loc: SourceLoc)
    case unsupportedConstruct(String, loc: SourceLoc)
    case missingRequiredField(String, container: String, loc: SourceLoc)

    public var description: String {
        switch self {
        case .unexpected(let tok, let expected):
            return "parser: expected \(expected) at \(tok.loc.line):\(tok.loc.column), got \(tok.kind) (lexeme '\(tok.lexeme)')"
        case .unknownDeclaration(let n, let l):
            return "parser: unknown top-level declaration '\(n)' at \(l.line):\(l.column)"
        case .unknownPaginationStrategy(let n, let l):
            return "parser: unknown pagination strategy '\(n)' at \(l.line):\(l.column)"
        case .unknownAuthStrategy(let n, let l):
            return "parser: unknown auth strategy '\(n)' at \(l.line):\(l.column)"
        case .unknownFieldType(let n, let l):
            return "parser: unknown field type '\(n)' at \(l.line):\(l.column)"
        case .unsupportedConstruct(let m, let l):
            return "parser: \(m) at \(l.line):\(l.column)"
        case .missingRequiredField(let n, let c, let l):
            return "parser: missing required field '\(n)' in \(c) at \(l.line):\(l.column)"
        }
    }
}
