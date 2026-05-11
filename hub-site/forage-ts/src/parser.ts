// Recursive-descent parser. Mirrors `Sources/Forage/Parser/Parser.swift`.

import {
    type AuthStrategy,
    type BearerLogin,
    type BodyValue,
    type BrowserConfig,
    type ComparisonOp,
    type CookiePersist,
    type EngineKind,
    type Emission,
    type Expectation,
    type ExtractionExpr,
    type FieldBinding,
    type FieldType,
    type FormLogin,
    type HTTPBody,
    type HTTPBodyKV,
    type HTTPRequest,
    type HTTPStep,
    type HtmlPrimeVar,
    type HubRecipeRef,
    type InputDecl,
    type JSONValue,
    type Pagination,
    type PathExpr,
    type Recipe,
    type RecipeEnum,
    type RecipeField,
    type RecipeType,
    type SessionAuth,
    type SessionAuthKind,
    type Statement,
    type Template,
    type TemplatePart,
    type TransformCall,
} from './ast.js'
import { Lexer, type SourceLoc, type Token, type TokenKind } from './lexer.js'

export class ParseError extends Error {
    constructor(public readonly loc: SourceLoc, message: string) {
        super(`parser: ${message} at ${loc.line}:${loc.column}`)
        this.name = 'ParseError'
    }
}

export class Parser {
    private pos = 0

    constructor(private readonly tokens: Token[]) {}

    static parse(source: string): Recipe {
        const lex = new Lexer(source)
        const toks = lex.tokenize()
        const p = new Parser(toks)
        return p.parseRecipe()
    }

    parseRecipe(): Recipe {
        const imports: HubRecipeRef[] = []
        while (this.checkKeyword('import')) {
            imports.push(this.parseImportDirective())
        }

        this.expectKeyword('recipe')
        const name = this.consumeStringLit()
        this.expect('lbrace', '{')

        let engineKind: EngineKind = 'http'
        const types: RecipeType[] = []
        const enums: RecipeEnum[] = []
        const inputs: InputDecl[] = []
        let auth: AuthStrategy | null = null
        let browser: BrowserConfig | null = null
        const body: Statement[] = []
        const expectations: Expectation[] = []
        const secrets: string[] = []

        while (!this.check('rbrace') && !this.check('eof')) {
            if (this.matchKeyword('engine')) {
                const k = this.consumeIdentifierOrKeyword()
                if (k === 'http') engineKind = 'http'
                else if (k === 'browser') engineKind = 'browser'
                else throw new ParseError(this.peek().loc, `unknown engine kind '${k}'`)
            } else if (this.matchKeyword('type')) {
                types.push(this.parseTypeDecl())
            } else if (this.matchKeyword('enum')) {
                enums.push(this.parseEnumDecl())
            } else if (this.matchKeyword('input')) {
                inputs.push(this.parseInputDecl())
            } else if (this.matchKeyword('secret')) {
                const n = this.consumeIdentifierOrKeyword()
                this.match('semicolon')
                secrets.push(n)
            } else if (this.matchKeyword('auth')) {
                this.expect('dot', '.')
                auth = this.parseAuthStrategy()
            } else if (this.matchKeyword('browser')) {
                browser = this.parseBrowserConfig()
            } else if (this.matchKeyword('expect')) {
                expectations.push(this.parseExpectation())
            } else if (this.checkKeyword('step') || this.checkKeyword('for') || this.checkKeyword('emit')) {
                body.push(this.parseStatement())
            } else {
                throw new ParseError(this.peek().loc, `unknown top-level declaration '${this.peek().lexeme}'`)
            }
        }
        this.expect('rbrace', '}')

        return {
            name,
            engineKind,
            types,
            enums,
            inputs,
            auth,
            body,
            browser,
            expectations,
            imports,
            secrets,
        }
    }

    private parseImportDirective(): HubRecipeRef {
        const importLoc = this.peek().loc
        this.expectKeyword('import')
        const tok = this.peek()
        if (tok.kind.tag !== 'hubURL') {
            throw new ParseError(tok.loc, `expected 'hub://<slug>' after 'import'`)
        }
        const slug = tok.kind.slug
        this.advance()
        // Validate slug shape: `<name>` or `<author>/<name>`
        if (slug.length === 0 || slug.split('/').length > 2) {
            throw new ParseError(importLoc, `malformed hub slug 'hub://${slug}'`)
        }
        let version: number | null = null
        if (this.peek().kind.tag === 'identifier') {
            const ident = (this.peek().kind as { tag: 'identifier'; name: string }).name
            if (ident.length >= 2 && ident[0] === 'v') {
                const n = parseInt(ident.slice(1), 10)
                if (!isNaN(n)) {
                    this.advance()
                    version = n
                }
            }
        }
        this.match('semicolon')
        return { slug, version }
    }

    // ---- Type / enum / input decls ----

    private parseTypeDecl(): RecipeType {
        const name = this.consumeTypeName()
        this.expect('lbrace', '{')
        const fields: RecipeField[] = []
        while (!this.check('rbrace')) {
            const fieldName = this.consumeIdentifierOrKeyword()
            this.expect('colon', ':')
            const [type, optional] = this.parseFieldType()
            fields.push({ name: fieldName, type, optional })
            this.match('semicolon')
        }
        this.expect('rbrace', '}')
        return { name, fields }
    }

    private parseEnumDecl(): RecipeEnum {
        const name = this.consumeTypeName()
        this.expect('lbrace', '{')
        const variants: string[] = []
        while (!this.check('rbrace')) {
            variants.push(this.consumeTypeNameOrIdent())
            this.match('comma'); this.match('semicolon')
        }
        this.expect('rbrace', '}')
        return { name, variants }
    }

    private parseInputDecl(): InputDecl {
        const name = this.consumeIdentifierOrKeyword()
        this.expect('colon', ':')
        const [type, optional] = this.parseFieldType()
        this.match('semicolon')
        return { name, type, optional }
    }

    private parseFieldType(): [FieldType, boolean] {
        let t: FieldType
        if (this.match('lbracket')) {
            const [inner, _] = this.parseFieldType()
            this.expect('rbracket', ']')
            t = { tag: 'array', element: inner }
        } else {
            const typeName = this.matchTypeName()
            if (typeName !== null) {
                switch (typeName) {
                    case 'String': t = { tag: 'string' }; break
                    case 'Int': t = { tag: 'int' }; break
                    case 'Double': t = { tag: 'double' }; break
                    case 'Bool': t = { tag: 'bool' }; break
                    default: t = { tag: 'record', name: typeName }
                }
            } else {
                const identLex = this.matchIdentifierOrKeyword()
                if (identLex === null) {
                    throw new ParseError(this.peek().loc, `expected type, got ${this.peek().lexeme}`)
                }
                switch (identLex) {
                    case 'String': t = { tag: 'string' }; break
                    case 'Int': t = { tag: 'int' }; break
                    case 'Double': t = { tag: 'double' }; break
                    case 'Bool': t = { tag: 'bool' }; break
                    default:
                        throw new ParseError(this.previous().loc, `unknown field type '${identLex}'`)
                }
            }
        }
        const optional = this.match('question')
        return [t, optional]
    }

    // ---- Auth ----

    private parseAuthStrategy(): AuthStrategy {
        const kind = this.consumeIdentifierOrKeyword()
        if (kind === 'session') {
            this.expect('dot', '.')
            const variant = this.consumeIdentifierOrKeyword()
            this.expect('lbrace', '{')
            return this.parseSessionAuth(variant)
        }
        this.expect('lbrace', '{')
        switch (kind) {
            case 'staticHeader': {
                let name: string | null = null
                let value: Template | null = null
                while (!this.check('rbrace')) {
                    const key = this.consumeIdentifierOrKeyword()
                    this.expect('colon', ':')
                    if (key === 'name') {
                        name = this.consumeStringLit()
                    } else if (key === 'value') {
                        value = this.parseTemplateLiteral()
                    } else {
                        throw new ParseError(this.previous().loc, `unknown staticHeader field '${key}'`)
                    }
                    this.match('semicolon'); this.match('comma')
                }
                this.expect('rbrace', '}')
                if (name === null || value === null) {
                    throw new ParseError(this.peek().loc, `missing required field 'name/value' in staticHeader`)
                }
                return { tag: 'staticHeader', name, value }
            }
            case 'htmlPrime': {
                let stepName: string | null = null
                let nonceVar: string | null = null
                let ajaxUrlVar: string | null = null
                while (!this.check('rbrace')) {
                    const key = this.consumeIdentifierOrKeyword()
                    this.expect('colon', ':')
                    if (key === 'step' || key === 'stepName') {
                        stepName = this.consumeIdentifierOrKeyword()
                    } else if (key === 'nonceVar') {
                        nonceVar = this.consumeStringLit()
                    } else if (key === 'ajaxUrlVar') {
                        ajaxUrlVar = this.consumeStringLit()
                    } else {
                        throw new ParseError(this.previous().loc, `unknown htmlPrime field '${key}'`)
                    }
                    this.match('semicolon'); this.match('comma')
                }
                this.expect('rbrace', '}')
                if (stepName === null) {
                    throw new ParseError(this.peek().loc, `missing required field 'step' in htmlPrime`)
                }
                const captured: HtmlPrimeVar[] = []
                if (nonceVar !== null) {
                    captured.push({ varName: nonceVar, regexPattern: '<placeholder>', groupIndex: 0 })
                }
                if (ajaxUrlVar !== null) {
                    captured.push({ varName: ajaxUrlVar, regexPattern: '<placeholder>', groupIndex: 0 })
                }
                return { tag: 'htmlPrime', stepName, capturedVars: captured }
            }
            default:
                throw new ParseError(this.previous().loc, `unknown auth strategy '${kind}'`)
        }
    }

    // ---- Session auth ----

    private parseSessionAuth(variant: string): AuthStrategy {
        let maxReauthRetries = 1
        let cacheDuration: number | null = null
        let cacheEncrypted = false
        let requiresMFA = false
        let mfaFieldName = 'code'

        let url: Template | null = null
        let method: string | null = null
        let body: HTTPBody | null = null
        let captureCookies = true
        let tokenPath: PathExpr | null = null
        let headerName = 'Authorization'
        let headerPrefix = 'Bearer '
        let sourcePath: Template | null = null
        let format: 'json' | 'netscape' = 'json'

        while (!this.check('rbrace')) {
            if (this.matchKeyword('body')) {
                this.expect('dot', '.')
                const kind = this.consumeIdentifierOrKeyword()
                switch (kind) {
                    case 'json':
                        body = { tag: 'jsonObject', entries: this.parseJSONBodyKVs() }
                        break
                    case 'form':
                        body = { tag: 'form', entries: this.parseFormBodyKVs() }
                        break
                    case 'raw':
                        body = { tag: 'raw', template: this.parseTemplateLiteral() }
                        break
                    default:
                        throw new ParseError(this.previous().loc, `body.${kind} not supported`)
                }
                this.match('semicolon'); this.match('comma')
                continue
            }
            const key = this.consumeIdentifierOrKeyword()
            this.expect('colon', ':')
            switch (key) {
                case 'maxReauthRetries':
                    maxReauthRetries = this.consumeIntLit()
                    break
                case 'cache': {
                    const i = this.matchIntLit()
                    if (i !== null) cacheDuration = i
                    else {
                        const d = this.matchDoubleLit()
                        if (d !== null) cacheDuration = d
                        else throw new ParseError(this.peek().loc, `expected duration in seconds`)
                    }
                    break
                }
                case 'cacheEncrypted':
                    cacheEncrypted = this.consumeBoolLit()
                    break
                case 'requiresMFA':
                    requiresMFA = this.consumeBoolLit()
                    break
                case 'mfaFieldName':
                    mfaFieldName = this.consumeStringLit()
                    break
                case 'url':
                    url = this.parseTemplateLiteral()
                    break
                case 'method':
                    method = this.consumeStringLit()
                    break
                case 'captureCookies':
                    captureCookies = this.consumeBoolLit()
                    break
                case 'tokenPath':
                    tokenPath = this.parsePathExpr()
                    break
                case 'headerName':
                    headerName = this.consumeStringLit()
                    break
                case 'headerPrefix':
                    headerPrefix = this.consumeStringLit()
                    break
                case 'sourcePath':
                    sourcePath = this.parseTemplateLiteral()
                    break
                case 'format': {
                    const raw = this.consumeIdentifierOrKeyword()
                    if (raw === 'json' || raw === 'netscape') format = raw
                    else throw new ParseError(this.previous().loc, `unknown cookie format '${raw}' (expected json | netscape)`)
                    break
                }
                default:
                    throw new ParseError(this.previous().loc, `unknown auth.session.${variant} field '${key}'`)
            }
            this.match('semicolon'); this.match('comma')
        }
        this.expect('rbrace', '}')

        let kind: SessionAuthKind
        switch (variant) {
            case 'formLogin': {
                if (url === null || body === null) {
                    throw new ParseError(this.peek().loc, `missing required field 'url/body' in auth.session.formLogin`)
                }
                const f: FormLogin = {
                    url, method: method ?? 'POST', body, captureCookies,
                }
                kind = { tag: 'formLogin', formLogin: f }
                break
            }
            case 'bearerLogin': {
                if (url === null || body === null || tokenPath === null) {
                    throw new ParseError(this.peek().loc, `missing required field 'url/body/tokenPath' in auth.session.bearerLogin`)
                }
                const b: BearerLogin = {
                    url, method: method ?? 'POST', body, tokenPath, headerName, headerPrefix,
                }
                kind = { tag: 'bearerLogin', bearerLogin: b }
                break
            }
            case 'cookiePersist': {
                if (sourcePath === null) {
                    throw new ParseError(this.peek().loc, `missing required field 'sourcePath' in auth.session.cookiePersist`)
                }
                const c: CookiePersist = { sourcePath, format }
                kind = { tag: 'cookiePersist', cookiePersist: c }
                break
            }
            default:
                throw new ParseError(this.previous().loc, `unknown auth.session.${variant}`)
        }
        const session: SessionAuth = {
            kind,
            maxReauthRetries,
            cacheDuration,
            cacheEncrypted,
            requiresMFA,
            mfaFieldName,
        }
        return { tag: 'session', session }
    }

    // ---- Statements ----

    private parseStatement(): Statement {
        if (this.matchKeyword('step')) return { tag: 'step', step: this.parseStep() }
        if (this.matchKeyword('for')) return this.parseForLoop()
        if (this.matchKeyword('emit')) return { tag: 'emit', emission: this.parseEmit() }
        throw new ParseError(this.peek().loc, `expected 'step | for | emit', got ${this.peek().lexeme}`)
    }

    private parseForLoop(): Statement {
        const variable = this.consumeDollarVariable()
        this.expectKeyword('in')
        const collection = this.parsePathExpr()
        this.expect('lbrace', '{')
        const body: Statement[] = []
        while (!this.check('rbrace')) body.push(this.parseStatement())
        this.expect('rbrace', '}')
        return { tag: 'forLoop', variable, collection, body }
    }

    private parseStep(): HTTPStep {
        const name = this.consumeIdentifierOrKeyword()
        this.expect('lbrace', '{')
        let method = 'GET'
        let url: Template = { parts: [{ tag: 'literal', value: '' }] }
        const headers: Array<{ key: string; value: Template }> = []
        let body: HTTPBody | null = null
        let pagination: Pagination | null = null

        while (!this.check('rbrace')) {
            if (this.matchKeyword('method')) {
                method = this.consumeStringLit()
            } else if (this.matchKeyword('url')) {
                url = this.parseTemplateLiteral()
            } else if (this.matchKeyword('headers')) {
                this.expect('lbrace', '{')
                while (!this.check('rbrace')) {
                    const k = this.consumeStringLit()
                    this.expect('colon', ':')
                    const v = this.parseTemplateLiteral()
                    headers.push({ key: k, value: v })
                    this.match('semicolon'); this.match('comma')
                }
                this.expect('rbrace', '}')
            } else if (this.matchKeyword('body')) {
                this.expect('dot', '.')
                const kind = this.consumeIdentifierOrKeyword()
                switch (kind) {
                    case 'json':
                        body = { tag: 'jsonObject', entries: this.parseJSONBodyKVs() }
                        break
                    case 'form':
                        body = { tag: 'form', entries: this.parseFormBodyKVs() }
                        break
                    case 'raw':
                        body = { tag: 'raw', template: this.parseTemplateLiteral() }
                        break
                    default:
                        throw new ParseError(this.previous().loc, `body.${kind} not supported`)
                }
            } else if (this.matchKeyword('paginate')) {
                pagination = this.parsePagination()
            } else if (this.matchKeyword('extract')) {
                this.expect('dot', '.')
                this.consumeIdentifierOrKeyword()
                this.skipBracedBlock()
            } else {
                throw new ParseError(this.peek().loc, `unknown step field '${this.peek().lexeme}'`)
            }
            this.match('semicolon')
        }
        this.expect('rbrace', '}')
        return {
            name,
            request: { method, url, headers, body },
            pagination,
        }
    }

    private parseJSONBodyKVs(): HTTPBodyKV[] {
        this.expect('lbrace', '{')
        const kvs: HTTPBodyKV[] = []
        while (!this.check('rbrace')) {
            const key = this.consumeIdentifierOrKeyword()
            this.expect('colon', ':')
            const value = this.parseBodyValue()
            kvs.push({ key, value })
            this.match('semicolon'); this.match('comma')
        }
        this.expect('rbrace', '}')
        return kvs
    }

    private parseFormBodyKVs(): Array<{ key: string; value: BodyValue }> {
        this.expect('lbrace', '{')
        const kvs: Array<{ key: string; value: BodyValue }> = []
        while (!this.check('rbrace')) {
            const k = this.consumeStringLit()
            this.expect('colon', ':')
            const v = this.parseBodyValue()
            kvs.push({ key: k, value: v })
            this.match('semicolon'); this.match('comma')
        }
        this.expect('rbrace', '}')
        return kvs
    }

    private parseBodyValue(): BodyValue {
        if (this.matchKeyword('case')) return this.parseBodyValueCaseOf()
        if (this.check('lbrace')) return { tag: 'object', entries: this.parseJSONBodyKVs() }
        if (this.check('lbracket')) {
            this.expect('lbracket', '[')
            const elems: BodyValue[] = []
            while (!this.check('rbracket')) {
                elems.push(this.parseBodyValue())
                this.match('comma')
            }
            this.expect('rbracket', ']')
            return { tag: 'array', items: elems }
        }
        if (this.checkPathStart()) return { tag: 'path', path: this.parsePathExpr() }
        const s = this.matchStringLit()
        if (s !== null) return { tag: 'templateString', template: this.makeTemplate(s) }
        const i = this.matchIntLit()
        if (i !== null) return { tag: 'literal', value: { tag: 'int', value: i } }
        const d = this.matchDoubleLit()
        if (d !== null) return { tag: 'literal', value: { tag: 'double', value: d } }
        const b = this.matchBoolLit()
        if (b !== null) return { tag: 'literal', value: { tag: 'bool', value: b } }
        if (this.matchNullLit()) return { tag: 'literal', value: { tag: 'null' } }
        const tn = this.matchTypeName()
        if (tn !== null) return { tag: 'literal', value: { tag: 'string', value: tn } }
        throw new ParseError(this.peek().loc, `expected body value, got ${this.peek().lexeme}`)
    }

    private parseBodyValueCaseOf(): BodyValue {
        const scrutinee = this.parsePathExpr()
        this.expectKeyword('of')
        this.expect('lbrace', '{')
        const branches: Array<{ label: string; value: BodyValue }> = []
        while (!this.check('rbrace')) {
            const label = this.consumeTypeNameOrIdent()
            this.expect('caseArrow', '→')
            const value = this.parseBodyValue()
            branches.push({ label, value })
            this.match('semicolon'); this.match('comma')
        }
        this.expect('rbrace', '}')
        return { tag: 'caseOf', scrutinee, branches }
    }

    // ---- Pagination ----

    private parsePagination(): Pagination {
        const kind = this.consumeIdentifierOrKeyword()
        this.expect('lbrace', '{')
        switch (kind) {
            case 'pageWithTotal': {
                let itemsPath: PathExpr | null = null
                let totalPath: PathExpr | null = null
                let pageParam: string | null = null
                let pageSize = 100
                let pageZeroIndexed = false
                while (!this.check('rbrace')) {
                    const key = this.consumeIdentifierOrKeyword()
                    this.expect('colon', ':')
                    switch (key) {
                        case 'items': itemsPath = this.parsePathExpr(); break
                        case 'total': totalPath = this.parsePathExpr(); break
                        case 'pageParam': pageParam = this.consumeStringLit(); break
                        case 'pageSize': pageSize = this.consumeIntLit(); break
                        case 'pageZeroIndexed': pageZeroIndexed = this.consumeBoolLit(); break
                        default:
                            throw new ParseError(this.previous().loc, `unknown pageWithTotal field '${key}'`)
                    }
                    this.match('semicolon'); this.match('comma')
                }
                this.expect('rbrace', '}')
                if (itemsPath === null || totalPath === null || pageParam === null) {
                    throw new ParseError(this.peek().loc, `missing required field 'items/total/pageParam' in pageWithTotal`)
                }
                return { tag: 'pageWithTotal', itemsPath, totalPath, pageParam, pageSize, pageZeroIndexed }
            }
            case 'untilEmpty': {
                let itemsPath: PathExpr | null = null
                let pageParam: string | null = null
                let pageZeroIndexed = false
                while (!this.check('rbrace')) {
                    const key = this.consumeIdentifierOrKeyword()
                    this.expect('colon', ':')
                    switch (key) {
                        case 'items': itemsPath = this.parsePathExpr(); break
                        case 'pageParam': pageParam = this.consumeStringLit(); break
                        case 'pageZeroIndexed': pageZeroIndexed = this.consumeBoolLit(); break
                        default:
                            throw new ParseError(this.previous().loc, `unknown untilEmpty field '${key}'`)
                    }
                    this.match('semicolon'); this.match('comma')
                }
                this.expect('rbrace', '}')
                if (itemsPath === null || pageParam === null) {
                    throw new ParseError(this.peek().loc, `missing required field 'items/pageParam' in untilEmpty`)
                }
                return { tag: 'untilEmpty', itemsPath, pageParam, pageZeroIndexed }
            }
            case 'cursor': {
                let itemsPath: PathExpr | null = null
                let cursorPath: PathExpr | null = null
                let cursorParam: string | null = null
                while (!this.check('rbrace')) {
                    const key = this.consumeIdentifierOrKeyword()
                    this.expect('colon', ':')
                    switch (key) {
                        case 'items': itemsPath = this.parsePathExpr(); break
                        case 'cursorPath': cursorPath = this.parsePathExpr(); break
                        case 'cursorParam': cursorParam = this.consumeStringLit(); break
                        default:
                            throw new ParseError(this.previous().loc, `unknown cursor field '${key}'`)
                    }
                    this.match('semicolon'); this.match('comma')
                }
                this.expect('rbrace', '}')
                if (itemsPath === null || cursorPath === null || cursorParam === null) {
                    throw new ParseError(this.peek().loc, `missing required field 'items/cursorPath/cursorParam' in cursor`)
                }
                return { tag: 'cursor', itemsPath, cursorPath, cursorParam }
            }
            default:
                throw new ParseError(this.previous().loc, `unknown pagination strategy '${kind}'`)
        }
    }

    // ---- Emit ----

    private parseEmit(): Emission {
        const typeName = this.consumeTypeName()
        this.expect('lbrace', '{')
        const bindings: FieldBinding[] = []
        while (!this.check('rbrace')) {
            const fieldName = this.consumeIdentifierOrKeyword()
            this.expect('arrow', '←')
            const expr = this.parseExtractionExpr()
            bindings.push({ fieldName, expr })
            this.match('semicolon'); this.match('comma')
        }
        this.expect('rbrace', '}')
        return { typeName, bindings }
    }

    private parseExtractionExpr(): ExtractionExpr {
        let head = this.parseExtractionAtom()
        if (this.match('pipe')) {
            const calls: TransformCall[] = [this.parseTransformCall()]
            while (this.match('pipe')) {
                calls.push(this.parseTransformCall())
            }
            head = { tag: 'pipe', inner: head, calls }
        }
        return head
    }

    private parseExtractionAtom(): ExtractionExpr {
        if (this.matchKeyword('case')) {
            const scrutinee = this.parsePathExpr()
            this.expectKeyword('of')
            this.expect('lbrace', '{')
            const branches: Array<{ label: string; expr: ExtractionExpr }> = []
            while (!this.check('rbrace')) {
                const label = this.consumeTypeNameOrIdent()
                this.expect('caseArrow', '→')
                const expr = this.parseExtractionExpr()
                branches.push({ label, expr })
                this.match('semicolon'); this.match('comma')
            }
            this.expect('rbrace', '}')
            return { tag: 'caseOf', scrutinee, branches }
        }
        if (this.checkPathStart()) return { tag: 'path', path: this.parsePathExpr() }
        const s = this.matchStringLit()
        if (s !== null) return { tag: 'template', template: this.makeTemplate(s) }
        const i = this.matchIntLit()
        if (i !== null) return { tag: 'literal', value: { tag: 'int', value: i } }
        const d = this.matchDoubleLit()
        if (d !== null) return { tag: 'literal', value: { tag: 'double', value: d } }
        const b = this.matchBoolLit()
        if (b !== null) return { tag: 'literal', value: { tag: 'bool', value: b } }
        if (this.matchNullLit()) return { tag: 'literal', value: { tag: 'null' } }
        const id = this.matchIdentifierOrKeyword()
        if (id !== null) {
            if (this.check('lparen')) {
                this.expect('lparen', '(')
                const args: ExtractionExpr[] = []
                while (!this.check('rparen')) {
                    args.push(this.parseExtractionExpr())
                    this.match('comma')
                }
                this.expect('rparen', ')')
                return { tag: 'call', name: id, args }
            }
            return { tag: 'literal', value: { tag: 'string', value: id } }
        }
        throw new ParseError(this.peek().loc, `expected extraction expression, got ${this.peek().lexeme}`)
    }

    private parseTransformCall(): TransformCall {
        const name = this.consumeIdentifierOrKeyword()
        if (name === 'map') {
            throw new ParseError(this.peek().loc, `map(...) transforms not yet supported in TS port`)
        }
        if (this.check('lparen')) {
            this.expect('lparen', '(')
            const args: ExtractionExpr[] = []
            while (!this.check('rparen')) {
                args.push(this.parseExtractionExpr())
                this.match('comma')
            }
            this.expect('rparen', ')')
            return { name, args }
        }
        return { name, args: [] }
    }

    // ---- Path expression ----

    private parsePathExpr(): PathExpr {
        let head: PathExpr
        if (this.check('dollarRoot')) {
            this.advance()
            head = { tag: 'current' }
        } else if (this.check('dollarInput')) {
            this.advance()
            head = { tag: 'input' }
        } else if (this.check('dollarSecret')) {
            this.advance()
            this.expect('dot', '.')
            const secretName = this.consumeIdentifierOrKeyword()
            head = { tag: 'secret', name: secretName }
        } else {
            const varName = this.matchDollarVariable()
            if (varName !== null) {
                head = { tag: 'variable', name: varName }
            } else {
                throw new ParseError(this.peek().loc, `expected path-expression head ($ / $input / $secret.<name> / $name), got ${this.peek().lexeme}`)
            }
        }
        while (true) {
            if (this.match('dot')) {
                const name = this.consumeIdentifierOrKeyword()
                head = { tag: 'field', base: head, name }
            } else if (this.match('qDot')) {
                const name = this.consumeIdentifierOrKeyword()
                head = { tag: 'optField', base: head, name }
            } else if (this.match('lbracket')) {
                const i = this.consumeIntLit()
                this.expect('rbracket', ']')
                head = { tag: 'index', base: head, idx: i }
            } else if (this.match('wildcard')) {
                head = { tag: 'wildcard', base: head }
            } else {
                break
            }
        }
        return head
    }

    // ---- Templates ----

    private parseTemplateLiteral(): Template {
        const s = this.consumeStringLit()
        return this.makeTemplate(s)
    }

    private makeTemplate(s: string): Template {
        const parts: TemplatePart[] = []
        let literal = ''
        let i = 0
        while (i < s.length) {
            const c = s[i]
            if (c === '{') {
                if (literal !== '') { parts.push({ tag: 'literal', value: literal }); literal = '' }
                let j = i + 1
                let content = ''
                while (j < s.length && s[j] !== '}') {
                    content += s[j]
                    j += 1
                }
                if (j < s.length) {
                    try {
                        const expr = parseExtractionSnippet(content)
                        parts.push({ tag: 'interp', expr })
                    } catch {
                        literal += `{${content}}`
                    }
                    i = j + 1
                    continue
                } else {
                    literal += '{'
                    literal += content
                }
            } else {
                literal += c
            }
            i += 1
        }
        if (literal !== '') parts.push({ tag: 'literal', value: literal })
        return { parts }
    }

    // ---- BrowserConfig (skipped detail) ----

    private parseBrowserConfig(): BrowserConfig {
        // The TS port doesn't run browser recipes; we just consume the block
        // so HTTP recipes following a browser block parse, and we keep the
        // initialURL for display purposes.
        this.expect('lbrace', '{')
        let initialURL: Template = { parts: [{ tag: 'literal', value: '' }] }
        let depth = 1
        while (depth > 0 && !this.check('eof')) {
            if (this.checkKeyword('initialURL') && depth === 1) {
                this.advance()
                this.expect('colon', ':')
                initialURL = this.parseTemplateLiteral()
                this.match('semicolon')
                continue
            }
            if (this.check('lbrace')) depth += 1
            else if (this.check('rbrace')) {
                depth -= 1
                if (depth === 0) break
            }
            this.advance()
        }
        this.expect('rbrace', '}')
        return { initialURL }
    }

    // ---- Expectations ----

    private parseExpectation(): Expectation {
        this.expect('lbrace', '{')
        this.expectKeyword('records')
        this.expect('dot', '.')
        this.expectKeyword('where')
        this.expect('lparen', '(')
        this.expectKeyword('typeName')
        this.expect('equal', '=')
        this.expect('equal', '=')
        const typeName = this.consumeStringLit()
        this.expect('rparen', ')')
        this.expect('dot', '.')
        this.expectKeyword('count')
        let op: ComparisonOp
        if (this.match('equal')) {
            this.expect('equal', '=')
            op = '=='
        } else if (this.match('gt')) {
            op = this.match('equal') ? '>=' : '>'
        } else if (this.match('lt')) {
            op = this.match('equal') ? '<=' : '<'
        } else if (this.match('bang')) {
            this.expect('equal', '=')
            op = '!='
        } else {
            throw new ParseError(this.peek().loc, `expected comparison operator, got ${this.peek().lexeme}`)
        }
        const value = this.consumeIntLit()
        this.expect('rbrace', '}')
        return { kind: { tag: 'recordCount', typeName, op, value } }
    }

    // ---- Token helpers ----

    private peek(): Token { return this.tokens[this.pos] }
    private previous(): Token { return this.tokens[Math.max(this.pos - 1, 0)] }

    private advance(): Token {
        const t = this.tokens[this.pos]
        if (this.pos < this.tokens.length - 1) this.pos += 1
        return t
    }

    private check(tag: TokenKind['tag']): boolean {
        return this.peek().kind.tag === tag
    }

    private checkKeyword(word: string): boolean {
        const k = this.peek().kind
        return k.tag === 'keyword' && k.name === word
    }

    private checkPathStart(): boolean {
        const t = this.peek().kind.tag
        return t === 'dollarRoot' || t === 'dollarInput' || t === 'dollarSecret' || t === 'dollarVariable'
    }

    private match(tag: TokenKind['tag']): boolean {
        if (this.check(tag)) { this.advance(); return true }
        return false
    }

    private matchKeyword(word: string): boolean {
        if (this.checkKeyword(word)) { this.advance(); return true }
        return false
    }

    private expect(tag: TokenKind['tag'], name: string): void {
        if (!this.check(tag)) {
            throw new ParseError(this.peek().loc, `expected ${name}, got ${this.peek().lexeme}`)
        }
        this.advance()
    }

    private expectKeyword(word: string): void {
        if (!this.checkKeyword(word)) {
            throw new ParseError(this.peek().loc, `expected keyword '${word}', got ${this.peek().lexeme}`)
        }
        this.advance()
    }

    private consumeStringLit(): string {
        const k = this.peek().kind
        if (k.tag === 'stringLit') { this.advance(); return k.value }
        throw new ParseError(this.peek().loc, `expected string literal, got ${this.peek().lexeme}`)
    }

    private matchStringLit(): string | null {
        const k = this.peek().kind
        if (k.tag === 'stringLit') { this.advance(); return k.value }
        return null
    }

    private consumeIntLit(): number {
        const k = this.peek().kind
        if (k.tag === 'intLit') { this.advance(); return k.value }
        throw new ParseError(this.peek().loc, `expected integer literal, got ${this.peek().lexeme}`)
    }

    private matchIntLit(): number | null {
        const k = this.peek().kind
        if (k.tag === 'intLit') { this.advance(); return k.value }
        return null
    }

    private matchDoubleLit(): number | null {
        const k = this.peek().kind
        if (k.tag === 'doubleLit') { this.advance(); return k.value }
        return null
    }

    private consumeBoolLit(): boolean {
        const k = this.peek().kind
        if (k.tag === 'boolLit') { this.advance(); return k.value }
        throw new ParseError(this.peek().loc, `expected bool literal, got ${this.peek().lexeme}`)
    }

    private matchBoolLit(): boolean | null {
        const k = this.peek().kind
        if (k.tag === 'boolLit') { this.advance(); return k.value }
        return null
    }

    private matchNullLit(): boolean {
        if (this.peek().kind.tag === 'nullLit') { this.advance(); return true }
        return false
    }

    private consumeIdentifierOrKeyword(): string {
        const k = this.peek().kind
        if (k.tag === 'identifier') { this.advance(); return k.name }
        if (k.tag === 'keyword') { this.advance(); return k.name }
        if (k.tag === 'typeName') { this.advance(); return k.name }
        throw new ParseError(this.peek().loc, `expected identifier, got ${this.peek().lexeme}`)
    }

    private matchIdentifierOrKeyword(): string | null {
        const k = this.peek().kind
        if (k.tag === 'identifier') { this.advance(); return k.name }
        if (k.tag === 'keyword') { this.advance(); return k.name }
        return null
    }

    private consumeTypeName(): string {
        const k = this.peek().kind
        if (k.tag === 'typeName') { this.advance(); return k.name }
        throw new ParseError(this.peek().loc, `expected type name (capitalized), got ${this.peek().lexeme}`)
    }

    private matchTypeName(): string | null {
        const k = this.peek().kind
        if (k.tag === 'typeName') { this.advance(); return k.name }
        return null
    }

    private consumeTypeNameOrIdent(): string {
        const k = this.peek().kind
        if (k.tag === 'typeName') { this.advance(); return k.name }
        if (k.tag === 'identifier') { this.advance(); return k.name }
        if (k.tag === 'keyword') { this.advance(); return k.name }
        if (k.tag === 'boolLit') { this.advance(); return String(k.value) }
        if (k.tag === 'intLit') { this.advance(); return String(k.value) }
        if (k.tag === 'nullLit') { this.advance(); return 'null' }
        if (k.tag === 'stringLit') { this.advance(); return k.value }
        throw new ParseError(this.peek().loc, `expected type name, identifier, or literal, got ${this.peek().lexeme}`)
    }

    private matchDollarVariable(): string | null {
        const k = this.peek().kind
        if (k.tag === 'dollarVariable') { this.advance(); return k.name }
        return null
    }

    private consumeDollarVariable(): string {
        const k = this.peek().kind
        if (k.tag === 'dollarVariable') { this.advance(); return k.name }
        throw new ParseError(this.peek().loc, `expected $variable, got ${this.peek().lexeme}`)
    }

    private skipBracedBlock(): void {
        this.expect('lbrace', '{')
        let depth = 1
        while (depth > 0 && !this.check('eof')) {
            if (this.check('lbrace')) depth += 1
            if (this.check('rbrace')) depth -= 1
            if (depth === 0) break
            this.advance()
        }
        this.expect('rbrace', '}')
    }
}

function parseExtractionSnippet(snippet: string): ExtractionExpr {
    const lex = new Lexer(snippet)
    const toks = lex.tokenize()
    const p = new Parser(toks)
    return p['parseExtractionExpr']()
}
