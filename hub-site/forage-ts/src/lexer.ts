// Lexer for `.forage` source. Mirrors Swift's `Lexer` 1:1.

export interface SourceLoc {
    line: number
    column: number
}

export type TokenKind =
    | { tag: 'lbrace' }
    | { tag: 'rbrace' }
    | { tag: 'lparen' }
    | { tag: 'rparen' }
    | { tag: 'lbracket' }
    | { tag: 'rbracket' }
    | { tag: 'comma' }
    | { tag: 'semicolon' }
    | { tag: 'colon' }
    | { tag: 'dot' }
    | { tag: 'question' }
    | { tag: 'qDot' }
    | { tag: 'wildcard' }
    | { tag: 'pipe' }
    | { tag: 'arrow' }
    | { tag: 'caseArrow' }
    | { tag: 'equal' }
    | { tag: 'gt' }
    | { tag: 'lt' }
    | { tag: 'bang' }
    | { tag: 'dollarRoot' }
    | { tag: 'dollarInput' }
    | { tag: 'dollarSecret' }
    | { tag: 'dollarVariable'; name: string }
    | { tag: 'stringLit'; value: string }
    | { tag: 'intLit'; value: number }
    | { tag: 'doubleLit'; value: number }
    | { tag: 'boolLit'; value: boolean }
    | { tag: 'nullLit' }
    | { tag: 'dateLit'; year: number; month: number; day: number }
    | { tag: 'identifier'; name: string }
    | { tag: 'typeName'; name: string }
    | { tag: 'keyword'; name: string }
    | { tag: 'eof' }

export interface Token {
    kind: TokenKind
    lexeme: string
    loc: SourceLoc
}

export const KEYWORDS: ReadonlySet<string> = new Set([
    'recipe', 'engine', 'http', 'browser', 'type', 'enum', 'input',
    'step', 'method', 'url', 'headers', 'body', 'json', 'form', 'raw',
    'auth', 'staticHeader', 'htmlPrime', 'extract', 'regex', 'groups',
    'paginate', 'pageWithTotal', 'untilEmpty', 'cursor',
    'items', 'total', 'pageParam', 'pageSize', 'cursorPath', 'cursorParam',
    'for', 'in', 'emit', 'case', 'of', 'let', 'where', 'expect',
    'true', 'false', 'null',
    'observe', 'browserPaginate', 'scroll', 'replay', 'ageGate',
    'autoFill', 'warmupClicks', 'navigate', 'until',
    'noProgressFor', 'maxIterations', 'iterationDelay', 'seedFilter',
    'replayOverride', 'captures', 'match', 'dismissals',
    'dob', 'reloadAfter', 'reloadAfterSubmit',
    'name', 'value', 'stepName', 'nonceVar', 'ajaxUrlVar',
    'pageZeroIndexed',
    'records', 'count', 'typeName',
    'initialURL', 'loadMoreLabels', 'extraLabels', 'captureExtractions',
    'iterPath', 'urlPattern', 'withCookies', 'as',
    'String', 'Int', 'Double', 'Bool',
    // Session auth (M7)
    'secret',
    'session', 'formLogin', 'bearerLogin', 'cookiePersist',
    'captureCookies', 'maxReauthRetries', 'cache', 'cacheEncrypted',
    'requiresMFA', 'mfaFieldName',
    'tokenPath', 'headerName', 'headerPrefix',
    'sourcePath', 'format',
    // Interactive bootstrap (M10)
    'interactive', 'bootstrapURL', 'cookieDomains', 'sessionExpiredPattern',
])

export class LexError extends Error {
    constructor(public readonly loc: SourceLoc, message: string) {
        super(`lexer: ${message} at ${loc.line}:${loc.column}`)
        this.name = 'LexError'
    }
}

export class Lexer {
    private index = 0
    private line = 1
    private column = 1

    constructor(private readonly source: string) {}

    tokenize(): Token[] {
        const tokens: Token[] = []
        while (!this.isEOF()) {
            this.skipWhitespaceAndComments()
            if (this.isEOF()) break

            const startLoc = this.loc()
            const c = this.peek()

            if (c === '{') { this.advance(); tokens.push({ kind: { tag: 'lbrace' }, lexeme: '{', loc: startLoc }); continue }
            if (c === '}') { this.advance(); tokens.push({ kind: { tag: 'rbrace' }, lexeme: '}', loc: startLoc }); continue }
            if (c === '(') { this.advance(); tokens.push({ kind: { tag: 'lparen' }, lexeme: '(', loc: startLoc }); continue }
            if (c === ')') { this.advance(); tokens.push({ kind: { tag: 'rparen' }, lexeme: ')', loc: startLoc }); continue }
            if (c === '[') {
                if (this.peek(1) === '*' && this.peek(2) === ']') {
                    this.advance(); this.advance(); this.advance()
                    tokens.push({ kind: { tag: 'wildcard' }, lexeme: '[*]', loc: startLoc })
                    continue
                }
                this.advance()
                tokens.push({ kind: { tag: 'lbracket' }, lexeme: '[', loc: startLoc })
                continue
            }
            if (c === ']') { this.advance(); tokens.push({ kind: { tag: 'rbracket' }, lexeme: ']', loc: startLoc }); continue }
            if (c === ',') { this.advance(); tokens.push({ kind: { tag: 'comma' }, lexeme: ',', loc: startLoc }); continue }
            if (c === ';') { this.advance(); tokens.push({ kind: { tag: 'semicolon' }, lexeme: ';', loc: startLoc }); continue }
            if (c === ':') { this.advance(); tokens.push({ kind: { tag: 'colon' }, lexeme: ':', loc: startLoc }); continue }
            if (c === '.') { this.advance(); tokens.push({ kind: { tag: 'dot' }, lexeme: '.', loc: startLoc }); continue }
            if (c === '?') {
                if (this.peek(1) === '.') {
                    this.advance(); this.advance()
                    tokens.push({ kind: { tag: 'qDot' }, lexeme: '?.', loc: startLoc })
                    continue
                }
                this.advance()
                tokens.push({ kind: { tag: 'question' }, lexeme: '?', loc: startLoc })
                continue
            }
            if (c === '|') { this.advance(); tokens.push({ kind: { tag: 'pipe' }, lexeme: '|', loc: startLoc }); continue }
            if (c === '=') { this.advance(); tokens.push({ kind: { tag: 'equal' }, lexeme: '=', loc: startLoc }); continue }
            if (c === '>') { this.advance(); tokens.push({ kind: { tag: 'gt' }, lexeme: '>', loc: startLoc }); continue }
            if (c === '<') { this.advance(); tokens.push({ kind: { tag: 'lt' }, lexeme: '<', loc: startLoc }); continue }
            if (c === '!') { this.advance(); tokens.push({ kind: { tag: 'bang' }, lexeme: '!', loc: startLoc }); continue }
            if (c === '←') { this.advance(); tokens.push({ kind: { tag: 'arrow' }, lexeme: '←', loc: startLoc }); continue }
            if (c === '→') { this.advance(); tokens.push({ kind: { tag: 'caseArrow' }, lexeme: '→', loc: startLoc }); continue }

            if (c === '"') {
                const value = this.readStringLiteral(startLoc)
                tokens.push({ kind: { tag: 'stringLit', value }, lexeme: `"${value}"`, loc: startLoc })
                continue
            }

            if (c === '$') {
                this.advance()
                const next = this.peek()
                if (isLetter(next) || next === '_') {
                    const name = this.readIdent()
                    if (name === 'input') {
                        tokens.push({ kind: { tag: 'dollarInput' }, lexeme: '$input', loc: startLoc })
                    } else if (name === 'secret') {
                        tokens.push({ kind: { tag: 'dollarSecret' }, lexeme: '$secret', loc: startLoc })
                    } else {
                        tokens.push({ kind: { tag: 'dollarVariable', name }, lexeme: `$${name}`, loc: startLoc })
                    }
                } else {
                    tokens.push({ kind: { tag: 'dollarRoot' }, lexeme: '$', loc: startLoc })
                }
                continue
            }

            if (isDigit(c) || (c === '-' && isDigit(this.peek(1)))) {
                tokens.push(this.readNumberOrDate(startLoc))
                continue
            }

            if (isLetter(c) || c === '_') {
                const name = this.readIdent()
                if (KEYWORDS.has(name)) {
                    if (name === 'true') {
                        tokens.push({ kind: { tag: 'boolLit', value: true }, lexeme: name, loc: startLoc })
                    } else if (name === 'false') {
                        tokens.push({ kind: { tag: 'boolLit', value: false }, lexeme: name, loc: startLoc })
                    } else if (name === 'null') {
                        tokens.push({ kind: { tag: 'nullLit' }, lexeme: name, loc: startLoc })
                    } else {
                        tokens.push({ kind: { tag: 'keyword', name }, lexeme: name, loc: startLoc })
                    }
                } else if (name.length > 0 && name[0] === name[0].toUpperCase() && /[A-Z]/.test(name[0])) {
                    tokens.push({ kind: { tag: 'typeName', name }, lexeme: name, loc: startLoc })
                } else {
                    tokens.push({ kind: { tag: 'identifier', name }, lexeme: name, loc: startLoc })
                }
                continue
            }

            throw new LexError(startLoc, `unexpected character '${c}'`)
        }
        tokens.push({ kind: { tag: 'eof' }, lexeme: '', loc: this.loc() })
        return tokens
    }

    private isEOF(): boolean {
        return this.index >= this.source.length
    }

    private peek(offset = 0): string {
        const i = this.index + offset
        if (i >= this.source.length) return '\0'
        return this.source[i]
    }

    private advance(): void {
        if (this.isEOF()) return
        const c = this.source[this.index]
        if (c === '\n') { this.line += 1; this.column = 1 }
        else { this.column += 1 }
        this.index += 1
    }

    private loc(): SourceLoc {
        return { line: this.line, column: this.column }
    }

    private skipWhitespaceAndComments(): void {
        while (!this.isEOF()) {
            const c = this.peek()
            if (c === ' ' || c === '\t' || c === '\n' || c === '\r') {
                this.advance()
            } else if (c === '/' && this.peek(1) === '/') {
                while (!this.isEOF() && this.peek() !== '\n') this.advance()
            } else if (c === '/' && this.peek(1) === '*') {
                this.advance(); this.advance()
                while (!this.isEOF() && !(this.peek() === '*' && this.peek(1) === '/')) this.advance()
                if (!this.isEOF()) { this.advance(); this.advance() }
            } else {
                break
            }
        }
    }

    private readIdent(): string {
        let name = ''
        while (!this.isEOF() && (isLetter(this.peek()) || isDigit(this.peek()) || this.peek() === '_')) {
            name += this.peek(); this.advance()
        }
        return name
    }

    private readStringLiteral(startLoc: SourceLoc): string {
        this.advance() // opening "
        let s = ''
        while (!this.isEOF() && this.peek() !== '"') {
            const c = this.peek()
            if (c === '\\') {
                this.advance()
                const esc = this.peek()
                switch (esc) {
                    case '"': s += '"'; this.advance(); break
                    case '\\': s += '\\'; this.advance(); break
                    case 'n': s += '\n'; this.advance(); break
                    case 't': s += '\t'; this.advance(); break
                    case 'r': s += '\r'; this.advance(); break
                    default: s += esc; this.advance(); break
                }
            } else {
                s += c; this.advance()
            }
        }
        if (this.isEOF()) throw new LexError(startLoc, 'unterminated string')
        this.advance() // closing "
        return s
    }

    private readNumberOrDate(startLoc: SourceLoc): Token {
        let raw = ''
        if (this.peek() === '-') { raw += '-'; this.advance() }
        while (!this.isEOF() && isDigit(this.peek())) { raw += this.peek(); this.advance() }
        // YYYY-MM-DD date?
        if (!this.isEOF() && this.peek() === '-' && raw.length >= 1 && !isNaN(parseInt(raw, 10))) {
            const saved = { index: this.index, line: this.line, column: this.column }
            this.advance()
            let monthStr = ''
            while (!this.isEOF() && isDigit(this.peek())) { monthStr += this.peek(); this.advance() }
            if (this.peek() === '-') {
                this.advance()
                let dayStr = ''
                while (!this.isEOF() && isDigit(this.peek())) { dayStr += this.peek(); this.advance() }
                if (raw.length === 4 && monthStr.length === 2 && dayStr.length === 2) {
                    const y = parseInt(raw, 10), m = parseInt(monthStr, 10), d = parseInt(dayStr, 10)
                    return {
                        kind: { tag: 'dateLit', year: y, month: m, day: d },
                        lexeme: `${raw}-${monthStr}-${dayStr}`,
                        loc: startLoc,
                    }
                }
            }
            this.index = saved.index; this.line = saved.line; this.column = saved.column
        }
        if (!this.isEOF() && this.peek() === '.' && isDigit(this.peek(1))) {
            raw += '.'; this.advance()
            while (!this.isEOF() && isDigit(this.peek())) { raw += this.peek(); this.advance() }
            const d = parseFloat(raw)
            if (isNaN(d)) throw new LexError(startLoc, `invalid number '${raw}'`)
            return { kind: { tag: 'doubleLit', value: d }, lexeme: raw, loc: startLoc }
        }
        const i = parseInt(raw, 10)
        if (isNaN(i)) throw new LexError(startLoc, `invalid number '${raw}'`)
        return { kind: { tag: 'intLit', value: i }, lexeme: raw, loc: startLoc }
    }
}

function isDigit(c: string): boolean {
    return c >= '0' && c <= '9'
}

function isLetter(c: string): boolean {
    return (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z')
}
