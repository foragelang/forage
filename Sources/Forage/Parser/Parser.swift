import Foundation

/// Recursive-descent parser. Consumes tokens from a `Lexer` and produces a
/// `Recipe`. Hand-rolled (no parser-combinator framework) so the error
/// messages stay specific to forage's grammar — `parser: expected …, got …`
/// with line/column info.
public struct Parser {
    private let tokens: [Token]
    private var pos: Int = 0

    public init(tokens: [Token]) { self.tokens = tokens }

    public static func parse(source: String) throws -> Recipe {
        var lexer = Lexer(source: source)
        let toks = try lexer.tokenize()
        var parser = Parser(tokens: toks)
        return try parser.parseRecipe()
    }

    // MARK: - Top-level

    public mutating func parseRecipe() throws -> Recipe {
        // Top-level `import <ref>` directives (Docker-style refs) appear
        // before the `recipe` header. Multiple are allowed; the order is
        // preserved.
        var imports: [HubRecipeRef] = []
        while checkKeyword("import") {
            imports.append(try parseImportDirective())
        }

        try expectKeyword("recipe")
        let nameTok = try consumeStringLit()
        let name = nameTok.0
        try expect(.lbrace, "{")

        var engineKind: EngineKind = .http
        var types: [RecipeType] = []
        var enums: [RecipeEnum] = []
        var inputs: [InputDecl] = []
        var auth: AuthStrategy? = nil
        var browser: BrowserConfig? = nil
        var body: [Statement] = []
        var expectations: [Expectation] = []
        var secrets: [String] = []

        while !check(.rbrace) && !check(.eof) {
            if matchKeyword("engine") {
                let k = try consumeIdentifierOrKeyword()
                switch k {
                case "http": engineKind = .http
                case "browser": engineKind = .browser
                default: throw ParseError.unsupportedConstruct("unknown engine kind '\(k)'", loc: peek().loc)
                }
            } else if matchKeyword("type") {
                types.append(try parseTypeDecl())
            } else if matchKeyword("enum") {
                enums.append(try parseEnumDecl())
            } else if matchKeyword("input") {
                inputs.append(try parseInputDecl())
            } else if matchKeyword("secret") {
                let n = try consumeIdentifierOrKeyword()
                _ = match(.semicolon)
                secrets.append(n)
            } else if matchKeyword("auth") {
                try expect(.dot, ".")
                auth = try parseAuthStrategy()
            } else if matchKeyword("browser") {
                browser = try parseBrowserConfig()
            } else if matchKeyword("expect") {
                expectations.append(try parseExpectation())
            } else if checkKeyword("step") || checkKeyword("for") || checkKeyword("emit") {
                body.append(try parseStatement())
            } else {
                throw ParseError.unknownDeclaration(peek().lexeme, loc: peek().loc)
            }
        }

        try expect(.rbrace, "}")

        return Recipe(
            name: name,
            engineKind: engineKind,
            types: types,
            enums: enums,
            inputs: inputs,
            auth: auth,
            body: body,
            browser: browser,
            expectations: expectations,
            imports: imports,
            secrets: secrets
        )
    }

    // MARK: - Import directives

    /// `import <ref> [v<N>]` — Docker-style reference. Examples:
    ///
    ///     import sweed
    ///     import alice/zen-leaf v3
    ///     import hub.example.com/team/scraper
    ///     import localhost:5000/me/test v1
    private mutating func parseImportDirective() throws -> HubRecipeRef {
        let importLoc = peek().loc
        try expectKeyword("import")

        guard case .refLit(let raw) = peek().kind else {
            throw ParseError.unsupportedConstruct(
                "expected import reference after 'import' at \(importLoc.line):\(importLoc.column)",
                loc: peek().loc
            )
        }
        let refLoc = peek().loc
        advance()

        // Optional `v<N>` version pin. Lexer reads `v3` as one identifier
        // (letter then digits), so we sniff the identifier text.
        var version: Int? = nil
        if case .identifier(let ident) = peek().kind,
           ident.hasPrefix("v"),
           ident.count >= 2,
           let n = Int(ident.dropFirst())
        {
            advance()
            version = n
        }
        _ = match(.semicolon)

        do {
            return try HubRecipeRef(parsing: raw, version: version)
        } catch let err as HubRecipeRef.ParseError {
            throw ParseError.unsupportedConstruct(err.description, loc: refLoc)
        }
    }

    // MARK: - Type / enum / input decls

    private mutating func parseTypeDecl() throws -> RecipeType {
        let name = try consumeTypeName()
        try expect(.lbrace, "{")
        var fields: [RecipeField] = []
        while !check(.rbrace) {
            let fieldName = try consumeIdentifierOrKeyword()
            try expect(.colon, ":")
            let (type, optional) = try parseFieldType()
            fields.append(RecipeField(name: fieldName, type: type, optional: optional))
            _ = match(.semicolon)
        }
        try expect(.rbrace, "}")
        return RecipeType(name: name, fields: fields)
    }

    private mutating func parseEnumDecl() throws -> RecipeEnum {
        let name = try consumeTypeName()
        try expect(.lbrace, "{")
        var variants: [String] = []
        while !check(.rbrace) {
            let variant = try consumeTypeNameOrIdent()
            variants.append(variant)
            _ = match(.comma); _ = match(.semicolon)
        }
        try expect(.rbrace, "}")
        return RecipeEnum(name: name, variants: variants)
    }

    private mutating func parseInputDecl() throws -> InputDecl {
        let name = try consumeIdentifierOrKeyword()
        try expect(.colon, ":")
        let (type, optional) = try parseFieldType()
        _ = match(.semicolon)
        return InputDecl(name: name, type: type, optional: optional)
    }

    /// `Type` or `Type?` or `[Type]` or `[Type]?`. Type can be a builtin
    /// (String/Int/Double/Bool), a type-name reference, or an enum-name reference
    /// (resolved later in the validator).
    private mutating func parseFieldType() throws -> (FieldType, Bool) {
        let t: FieldType
        if match(.lbracket) {
            let (inner, _) = try parseFieldType()
            try expect(.rbracket, "]")
            t = .array(inner)
        } else if let typeNameLexeme = matchTypeName() {
            switch typeNameLexeme {
            case "String": t = .string
            case "Int": t = .int
            case "Double": t = .double
            case "Bool": t = .bool
            default:
                // Default: assume record reference; AST2Recipe / validator
                // checks whether it's actually an enum at validation time.
                t = .record(typeNameLexeme)
            }
        } else if let identLex = matchIdentifierOrKeyword() {
            // Identifier-form type — `String` etc. matched as keyword.
            switch identLex {
            case "String": t = .string
            case "Int": t = .int
            case "Double": t = .double
            case "Bool": t = .bool
            default:
                throw ParseError.unknownFieldType(identLex, loc: previous().loc)
            }
        } else {
            throw ParseError.unexpected(peek(), expected: "type")
        }

        let optional = match(.question)
        return (t, optional)
    }

    // MARK: - Auth

    private mutating func parseAuthStrategy() throws -> AuthStrategy {
        let kindTok = try consumeIdentifierOrKeyword()
        // `auth.session.<variant> { … }` — session has a second discriminator.
        if kindTok == "session" {
            try expect(.dot, ".")
            let variant = try consumeIdentifierOrKeyword()
            try expect(.lbrace, "{")
            return try parseSessionAuth(variant: variant)
        }
        try expect(.lbrace, "{")
        switch kindTok {
        case "staticHeader":
            var name: String?
            var value: Template?
            while !check(.rbrace) {
                let key = try consumeIdentifierOrKeyword()
                try expect(.colon, ":")
                if key == "name" {
                    let (s, _) = try consumeStringLit()
                    name = s
                } else if key == "value" {
                    value = try parseTemplateLiteral()
                } else {
                    throw ParseError.unsupportedConstruct("unknown staticHeader field '\(key)'", loc: previous().loc)
                }
                _ = match(.semicolon); _ = match(.comma)
            }
            try expect(.rbrace, "}")
            guard let n = name, let v = value else {
                throw ParseError.missingRequiredField("name/value", container: "staticHeader", loc: peek().loc)
            }
            return .staticHeader(name: n, value: v)
        case "htmlPrime":
            var stepName: String?
            var nonceVar: String?
            var ajaxUrlVar: String?
            while !check(.rbrace) {
                let key = try consumeIdentifierOrKeyword()
                try expect(.colon, ":")
                if key == "step" || key == "stepName" {
                    let s = try consumeIdentifierOrKeyword()
                    stepName = s
                } else if key == "nonceVar" {
                    let (s, _) = try consumeStringLit()
                    nonceVar = s
                } else if key == "ajaxUrlVar" {
                    let (s, _) = try consumeStringLit()
                    ajaxUrlVar = s
                } else {
                    throw ParseError.unsupportedConstruct("unknown htmlPrime field '\(key)'", loc: previous().loc)
                }
                _ = match(.semicolon); _ = match(.comma)
            }
            try expect(.rbrace, "}")
            guard let s = stepName else {
                throw ParseError.missingRequiredField("step", container: "htmlPrime", loc: peek().loc)
            }
            // Concrete vars — recipe defines via the prime step's `extract.regex` block.
            // Here we just record the variable names; the prime step's extract block
            // gets parsed when that step is encountered (Phase D adds proper wire-up).
            // For now: capture two named slots if both supplied.
            var captured: [HtmlPrimeVar] = []
            if let n = nonceVar {
                // Pattern + groupIndex come from the prime step's extract.regex block
                // when that's parsed; we register a placeholder var here that the
                // parser fills in when it processes the prime step.
                captured.append(HtmlPrimeVar(varName: n, regexPattern: "<placeholder>", groupIndex: 0))
            }
            if let u = ajaxUrlVar {
                captured.append(HtmlPrimeVar(varName: u, regexPattern: "<placeholder>", groupIndex: 0))
            }
            return .htmlPrime(stepName: s, capturedVars: captured)
        default:
            throw ParseError.unknownAuthStrategy(kindTok, loc: previous().loc)
        }
    }

    // MARK: - Session auth (auth.session.<variant>)

    /// Parse the body of `auth.session.<variant> { ... }` after the opening
    /// `{` has been consumed. Variants share the option vocabulary
    /// (`maxReauthRetries`, `cache`, `cacheEncrypted`, `requiresMFA`,
    /// `mfaFieldName`); kind-specific keys are dispatched here.
    private mutating func parseSessionAuth(variant: String) throws -> AuthStrategy {
        // Shared session options.
        var maxReauthRetries: Int = 1
        var cacheDuration: TimeInterval? = nil
        var cacheEncrypted: Bool = false
        var requiresMFA: Bool = false
        var mfaFieldName: String = "code"

        // Variant-specific.
        var url: Template? = nil
        var method: String? = nil
        var body: HTTPBody? = nil
        var captureCookies: Bool = true
        var tokenPath: PathExpr? = nil
        var headerName: String = "Authorization"
        var headerPrefix: String = "Bearer "
        var sourcePath: Template? = nil
        var format: CookieFormat = .json

        while !check(.rbrace) {
            // `body.<kind>` block: keyword "body" + "." + variant + block.
            if matchKeyword("body") {
                try expect(.dot, ".")
                let kindTok = try consumeIdentifierOrKeyword()
                switch kindTok {
                case "json":
                    body = .jsonObject(try parseJSONBodyKVs())
                case "form":
                    body = .form(try parseFormBodyKVs())
                case "raw":
                    body = .raw(try parseTemplateLiteral())
                default:
                    throw ParseError.unsupportedConstruct("body.\(kindTok) not supported", loc: previous().loc)
                }
                _ = match(.semicolon); _ = match(.comma)
                continue
            }

            let key = try consumeIdentifierOrKeyword()
            try expect(.colon, ":")
            switch key {
            // Shared
            case "maxReauthRetries":
                maxReauthRetries = try consumeIntLit()
            case "cache":
                // Accept int (seconds) or double.
                if let i = matchIntLit() {
                    cacheDuration = TimeInterval(i)
                } else if let d = matchDoubleLit() {
                    cacheDuration = d
                } else {
                    throw ParseError.unexpected(peek(), expected: "duration in seconds")
                }
            case "cacheEncrypted":
                cacheEncrypted = try consumeBoolLit()
            case "requiresMFA":
                requiresMFA = try consumeBoolLit()
            case "mfaFieldName":
                let (s, _) = try consumeStringLit()
                mfaFieldName = s

            // Variant-specific
            case "url":
                url = try parseTemplateLiteral()
            case "method":
                let (s, _) = try consumeStringLit()
                method = s
            case "captureCookies":
                captureCookies = try consumeBoolLit()
            case "tokenPath":
                tokenPath = try parsePathExpr()
            case "headerName":
                let (s, _) = try consumeStringLit()
                headerName = s
            case "headerPrefix":
                let (s, _) = try consumeStringLit()
                headerPrefix = s
            case "sourcePath":
                sourcePath = try parseTemplateLiteral()
            case "format":
                // Bare identifier `json` / `netscape` (keyword or ident).
                let raw = try consumeIdentifierOrKeyword()
                switch raw {
                case "json": format = .json
                case "netscape": format = .netscape
                default:
                    throw ParseError.unsupportedConstruct("unknown cookie format '\(raw)' (expected json | netscape)", loc: previous().loc)
                }
            default:
                throw ParseError.unsupportedConstruct("unknown auth.session.\(variant) field '\(key)'", loc: previous().loc)
            }
            _ = match(.semicolon); _ = match(.comma)
        }
        try expect(.rbrace, "}")

        let kind: SessionAuth.Kind
        switch variant {
        case "formLogin":
            guard let u = url, let b = body else {
                throw ParseError.missingRequiredField("url/body", container: "auth.session.formLogin", loc: peek().loc)
            }
            kind = .formLogin(FormLogin(
                url: u, method: method ?? "POST", body: b, captureCookies: captureCookies
            ))
        case "bearerLogin":
            guard let u = url, let b = body, let tp = tokenPath else {
                throw ParseError.missingRequiredField("url/body/tokenPath", container: "auth.session.bearerLogin", loc: peek().loc)
            }
            kind = .bearerLogin(BearerLogin(
                url: u, method: method ?? "POST", body: b,
                tokenPath: tp, headerName: headerName, headerPrefix: headerPrefix
            ))
        case "cookiePersist":
            guard let sp = sourcePath else {
                throw ParseError.missingRequiredField("sourcePath", container: "auth.session.cookiePersist", loc: peek().loc)
            }
            kind = .cookiePersist(CookiePersist(sourcePath: sp, format: format))
        default:
            throw ParseError.unsupportedConstruct("unknown auth.session.\(variant)", loc: previous().loc)
        }
        return .session(SessionAuth(
            kind: kind,
            maxReauthRetries: maxReauthRetries,
            cacheDuration: cacheDuration,
            cacheEncrypted: cacheEncrypted,
            requiresMFA: requiresMFA,
            mfaFieldName: mfaFieldName
        ))
    }

    // MARK: - Statements

    private mutating func parseStatement() throws -> Statement {
        if matchKeyword("step") {
            return .step(try parseStep())
        }
        if matchKeyword("for") {
            return try parseForLoop()
        }
        if matchKeyword("emit") {
            return .emit(try parseEmit())
        }
        throw ParseError.unexpected(peek(), expected: "step | for | emit")
    }

    private mutating func parseForLoop() throws -> Statement {
        // `for $var in <extractionExpr> { body }`
        //
        // The collection is an ExtractionExpr (not just a PathExpr) so
        // pipelines work as iteration sources — e.g.
        // `for $card in $page | parseHtml | select(".card") { … }`
        // for HTML-extraction recipes. Bare paths `$arr[*]` still parse
        // cleanly: an atom-only ExtractionExpr wraps a PathExpr.
        let varTok = try consumeDollarVariable()
        try expectKeyword("in")
        let collection = try parseExtractionExpr()
        try expect(.lbrace, "{")
        var body: [Statement] = []
        while !check(.rbrace) {
            body.append(try parseStatement())
        }
        try expect(.rbrace, "}")
        return .forLoop(variable: varTok, collection: collection, body: body)
    }

    private mutating func parseStep() throws -> HTTPStep {
        let name = try consumeIdentifierOrKeyword()
        try expect(.lbrace, "{")
        var method = "GET"
        var url = Template(literal: "")
        var headers: [(String, Template)] = []
        var body: HTTPBody? = nil
        var pagination: Pagination? = nil

        while !check(.rbrace) {
            if matchKeyword("method") {
                let (s, _) = try consumeStringLit()
                method = s
            } else if matchKeyword("url") {
                url = try parseTemplateLiteral()
            } else if matchKeyword("headers") {
                try expect(.lbrace, "{")
                while !check(.rbrace) {
                    let (k, _) = try consumeStringLit()
                    try expect(.colon, ":")
                    let v = try parseTemplateLiteral()
                    headers.append((k, v))
                    _ = match(.semicolon); _ = match(.comma)
                }
                try expect(.rbrace, "}")
            } else if matchKeyword("body") {
                try expect(.dot, ".")
                let kindTok = try consumeIdentifierOrKeyword()
                switch kindTok {
                case "json":
                    body = .jsonObject(try parseJSONBodyKVs())
                case "form":
                    body = .form(try parseFormBodyKVs())
                case "raw":
                    body = .raw(try parseTemplateLiteral())
                default:
                    throw ParseError.unsupportedConstruct("body.\(kindTok) not supported", loc: previous().loc)
                }
            } else if matchKeyword("paginate") {
                pagination = try parsePagination()
            } else if matchKeyword("extract") {
                // Currently: ignore extract.<kind> { … } sub-block (Phase D wires it up).
                // Consume the `.` + kind keyword + block.
                try expect(.dot, ".")
                _ = try consumeIdentifierOrKeyword()
                try skipBracedBlock()
            } else {
                throw ParseError.unsupportedConstruct("unknown step field '\(peek().lexeme)'", loc: peek().loc)
            }
            _ = match(.semicolon)
        }
        try expect(.rbrace, "}")
        return HTTPStep(
            name: name,
            request: HTTPRequest(method: method, url: url, headers: headers, body: body),
            pagination: pagination
        )
    }

    private mutating func parseJSONBodyKVs() throws -> [HTTPBodyKV] {
        try expect(.lbrace, "{")
        var kvs: [HTTPBodyKV] = []
        while !check(.rbrace) {
            let key = try consumeIdentifierOrKeyword()
            try expect(.colon, ":")
            let value = try parseBodyValue()
            kvs.append(HTTPBodyKV(key: key, value: value))
            _ = match(.semicolon); _ = match(.comma)
        }
        try expect(.rbrace, "}")
        return kvs
    }

    private mutating func parseFormBodyKVs() throws -> [(String, BodyValue)] {
        try expect(.lbrace, "{")
        var kvs: [(String, BodyValue)] = []
        while !check(.rbrace) {
            let (k, _) = try consumeStringLit()
            try expect(.colon, ":")
            let v = try parseBodyValue()
            kvs.append((k, v))
            _ = match(.semicolon); _ = match(.comma)
        }
        try expect(.rbrace, "}")
        return kvs
    }

    private mutating func parseBodyValue() throws -> BodyValue {
        // Accept: string template, number, bool, null, $path, { …obj }, [ …arr ], case-of
        if matchKeyword("case") {
            return try parseBodyValueCaseOf()
        }
        if check(.lbrace) {
            return .object(try parseJSONBodyKVs())
        }
        if check(.lbracket) {
            try expect(.lbracket, "[")
            var elems: [BodyValue] = []
            while !check(.rbracket) {
                elems.append(try parseBodyValue())
                _ = match(.comma)
            }
            try expect(.rbracket, "]")
            return .array(elems)
        }
        if checkPathStart() {
            // `$.x` or `$input.x` or `$cat.id` -> path
            let p = try parsePathExpr()
            return .path(p)
        }
        if let s = matchStringLit() {
            return .templateString(makeTemplate(s))
        }
        if let i = matchIntLit() {
            return .literal(.int(i))
        }
        if let d = matchDoubleLit() {
            return .literal(.double(d))
        }
        if let b = matchBoolLit() {
            return .literal(.bool(b))
        }
        if matchNullLit() {
            return .literal(.null)
        }
        // Unquoted type-name → enum variant string (used inside case-of branches; treated as string label)
        if let t = matchTypeName() {
            return .literal(.string(t))
        }
        throw ParseError.unexpected(peek(), expected: "body value")
    }

    private mutating func parseBodyValueCaseOf() throws -> BodyValue {
        // `case $scrutinee of { LABEL → <bodyValue>; LABEL → <bodyValue> }`
        let scrutinee = try parsePathExpr()
        try expectKeyword("of")
        try expect(.lbrace, "{")
        var branches: [(label: String, value: BodyValue)] = []
        while !check(.rbrace) {
            let label = try consumeTypeNameOrIdent()
            try expect(.caseArrow, "→")
            let value = try parseBodyValue()
            branches.append((label, value))
            _ = match(.semicolon); _ = match(.comma)
        }
        try expect(.rbrace, "}")
        return .caseOf(scrutinee: scrutinee, branches: branches)
    }

    // MARK: - Pagination

    private mutating func parsePagination() throws -> Pagination {
        let kind = try consumeIdentifierOrKeyword()
        try expect(.lbrace, "{")
        switch kind {
        case "pageWithTotal":
            var itemsPath: PathExpr?
            var totalPath: PathExpr?
            var pageParam: String?
            var pageSize: Int = 100
            var pageZeroIndexed: Bool = false
            while !check(.rbrace) {
                let key = try consumeIdentifierOrKeyword()
                try expect(.colon, ":")
                switch key {
                case "items":
                    itemsPath = try parsePathExpr()
                case "total":
                    totalPath = try parsePathExpr()
                case "pageParam":
                    let (s, _) = try consumeStringLit()
                    pageParam = s
                case "pageSize":
                    pageSize = try consumeIntLit()
                case "pageZeroIndexed":
                    pageZeroIndexed = try consumeBoolLit()
                default:
                    throw ParseError.unsupportedConstruct("unknown pageWithTotal field '\(key)'", loc: previous().loc)
                }
                _ = match(.semicolon); _ = match(.comma)
            }
            try expect(.rbrace, "}")
            guard let i = itemsPath, let t = totalPath, let pp = pageParam else {
                throw ParseError.missingRequiredField("items/total/pageParam", container: "pageWithTotal", loc: peek().loc)
            }
            return .pageWithTotal(itemsPath: i, totalPath: t, pageParam: pp, pageSize: pageSize, pageZeroIndexed: pageZeroIndexed)
        case "untilEmpty":
            var itemsPath: PathExpr?
            var pageParam: String?
            var pageZeroIndexed: Bool = false
            while !check(.rbrace) {
                let key = try consumeIdentifierOrKeyword()
                try expect(.colon, ":")
                switch key {
                case "items":
                    itemsPath = try parsePathExpr()
                case "pageParam":
                    let (s, _) = try consumeStringLit()
                    pageParam = s
                case "pageZeroIndexed":
                    pageZeroIndexed = try consumeBoolLit()
                default:
                    throw ParseError.unsupportedConstruct("unknown untilEmpty field '\(key)'", loc: previous().loc)
                }
                _ = match(.semicolon); _ = match(.comma)
            }
            try expect(.rbrace, "}")
            guard let i = itemsPath, let pp = pageParam else {
                throw ParseError.missingRequiredField("items/pageParam", container: "untilEmpty", loc: peek().loc)
            }
            return .untilEmpty(itemsPath: i, pageParam: pp, pageZeroIndexed: pageZeroIndexed)
        case "cursor":
            var itemsPath: PathExpr?
            var cursorPath: PathExpr?
            var cursorParam: String?
            while !check(.rbrace) {
                let key = try consumeIdentifierOrKeyword()
                try expect(.colon, ":")
                switch key {
                case "items": itemsPath = try parsePathExpr()
                case "cursorPath": cursorPath = try parsePathExpr()
                case "cursorParam":
                    let (s, _) = try consumeStringLit()
                    cursorParam = s
                default:
                    throw ParseError.unsupportedConstruct("unknown cursor field '\(key)'", loc: previous().loc)
                }
                _ = match(.semicolon); _ = match(.comma)
            }
            try expect(.rbrace, "}")
            guard let i = itemsPath, let c = cursorPath, let cp = cursorParam else {
                throw ParseError.missingRequiredField("items/cursorPath/cursorParam", container: "cursor", loc: peek().loc)
            }
            return .cursor(itemsPath: i, cursorPath: c, cursorParam: cp)
        default:
            throw ParseError.unknownPaginationStrategy(kind, loc: previous().loc)
        }
    }

    // MARK: - Emit + extraction expressions

    private mutating func parseEmit() throws -> Emission {
        let typeName = try consumeTypeName()
        try expect(.lbrace, "{")
        var bindings: [FieldBinding] = []
        while !check(.rbrace) {
            let fieldName = try consumeIdentifierOrKeyword()
            try expect(.arrow, "←")
            let expr = try parseExtractionExpr()
            bindings.append(FieldBinding(fieldName: fieldName, expr: expr))
            _ = match(.semicolon); _ = match(.comma)
        }
        try expect(.rbrace, "}")
        return Emission(typeName: typeName, bindings: bindings)
    }

    private mutating func parseExtractionExpr() throws -> ExtractionExpr {
        var head = try parseExtractionAtom()
        // Pipeline: `head | name | name(args) | name(args)`
        while match(.pipe) {
            var calls: [TransformCall] = [try parseTransformCall()]
            while match(.pipe) {
                calls.append(try parseTransformCall())
            }
            head = .pipe(head, calls)
            break
        }
        return head
    }

    private mutating func parseExtractionAtom() throws -> ExtractionExpr {
        if matchKeyword("case") {
            // case $x of { LABEL → expr; … }
            let scrutinee = try parsePathExpr()
            try expectKeyword("of")
            try expect(.lbrace, "{")
            var branches: [(label: String, expr: ExtractionExpr)] = []
            while !check(.rbrace) {
                let label = try consumeTypeNameOrIdent()
                try expect(.caseArrow, "→")
                let expr = try parseExtractionExpr()
                branches.append((label, expr))
                _ = match(.semicolon); _ = match(.comma)
            }
            try expect(.rbrace, "}")
            return .caseOf(scrutinee: scrutinee, branches: branches)
        }
        if checkPathStart() {
            return .path(try parsePathExpr())
        }
        if let s = matchStringLit() {
            // String literals as extraction values are templates (so we can interpolate $vars).
            return .template(makeTemplate(s))
        }
        if let i = matchIntLit() {
            return .literal(.int(i))
        }
        if let d = matchDoubleLit() {
            return .literal(.double(d))
        }
        if let b = matchBoolLit() {
            return .literal(.bool(b))
        }
        if matchNullLit() {
            return .literal(.null)
        }
        // Function-call form: `coalesce(a, b)` — unquoted identifier followed by (.
        if let id = matchIdentifierOrKeyword() {
            if check(.lparen) {
                try expect(.lparen, "(")
                var args: [ExtractionExpr] = []
                while !check(.rparen) {
                    args.append(try parseExtractionExpr())
                    _ = match(.comma)
                }
                try expect(.rparen, ")")
                return .call(name: id, args: args)
            }
            // Bare identifier in extraction position — treat as path-like enum label string.
            return .literal(.string(id))
        }
        throw ParseError.unexpected(peek(), expected: "extraction expression")
    }

    private mutating func parseTransformCall() throws -> TransformCall {
        // Either `name` or `name(args)` or `map(<emit>)` (special)
        let name = try consumeIdentifierOrKeyword()
        if name == "map" {
            try expect(.lparen, "(")
            // Either an emit sub-block or a function-call-style (name) — for now,
            // require: `map(<TypeName> { … })` OR `map(<bareName>)`.
            // We accept the inline emit form.
            if let typeName = matchTypeName() {
                if check(.lbrace) {
                    let saved = pos
                    pos = saved
                    let emission = try parseEmitInline(typeName: typeName)
                    try expect(.rparen, ")")
                    // Map transform doesn't fit cleanly into TransformCall; recast as
                    // ExtractionExpr.mapTo which the parser substitutes at the pipe site.
                    // Workaround: encode as a call with one literal arg holding an
                    // unrepresentable value won't work; instead, the caller of
                    // parseTransformCall recognizes `map` and lifts to mapTo.
                    return TransformCall(name: "__map__", args: [.literal(.string(typeName)), .literal(.string(synthesizeMapMarker(emission)))])
                }
                throw ParseError.unsupportedConstruct("map(TypeName) without { … } body", loc: peek().loc)
            }
            // map(bareIdentifier) — interpret as `map(<emitName>)` referring to a sub-recipe.
            // For now, not supported.
            throw ParseError.unsupportedConstruct("map(<name>) without inline emit body — not yet supported", loc: peek().loc)
        }
        if check(.lparen) {
            try expect(.lparen, "(")
            var args: [ExtractionExpr] = []
            while !check(.rparen) {
                args.append(try parseExtractionExpr())
                _ = match(.comma)
            }
            try expect(.rparen, ")")
            return TransformCall(name: name, args: args)
        }
        return TransformCall(name: name, args: [])
    }

    private var mapEmissions: [String: Emission] = [:]
    private func synthesizeMapMarker(_ emission: Emission) -> String {
        let key = "_map_emission_\(mapEmissions.count)_\(emission.typeName)"
        // We don't actually need to store these; the AST→Recipe translator
        // unfortunately can't see this state, so instead we encode the emission
        // inline. Switch to a different representation:
        // `TransformCall(name: "__map__", args: [.literal(.string(typeName)), <bindings as nested calls>])`.
        // Simpler: we move map handling into parseExtractionExpr below.
        return key
    }

    private mutating func parseEmitInline(typeName: String) throws -> Emission {
        try expect(.lbrace, "{")
        var bindings: [FieldBinding] = []
        while !check(.rbrace) {
            let fieldName = try consumeIdentifierOrKeyword()
            try expect(.arrow, "←")
            let expr = try parseExtractionExpr()
            bindings.append(FieldBinding(fieldName: fieldName, expr: expr))
            _ = match(.semicolon); _ = match(.comma)
        }
        try expect(.rbrace, "}")
        return Emission(typeName: typeName, bindings: bindings)
    }

    // MARK: - Path expression

    private mutating func parsePathExpr() throws -> PathExpr {
        var head: PathExpr
        if check(.dollarRoot) {
            advance()
            head = .current
        } else if check(.dollarInput) {
            advance()
            head = .input
        } else if check(.dollarSecret) {
            // `$secret.<name>` — must be followed by `.<ident>`.
            advance()
            try expect(.dot, ".")
            let secretName = try consumeIdentifierOrKeyword()
            head = .secret(secretName)
        } else if let varName = matchDollarVariable() {
            head = .variable(varName)
        } else {
            throw ParseError.unexpected(peek(), expected: "path-expression head ($ / $input / $secret.<name> / $name)")
        }

        // Trailing chain
        while true {
            if match(.dot) {
                let name = try consumeIdentifierOrKeyword()
                head = .field(head, name)
            } else if match(.qDot) {
                let name = try consumeIdentifierOrKeyword()
                head = .optField(head, name)
            } else if match(.lbracket) {
                // [N]
                let i = try consumeIntLit()
                try expect(.rbracket, "]")
                head = .index(head, i)
            } else if match(.wildcard) {
                head = .wildcard(head)
            } else {
                break
            }
        }
        return head
    }

    // MARK: - Templates

    /// Parse a string-literal token, expand `{$path}` interpolations, return
    /// a Template. Without interpolations it's a single `.literal(s)` part.
    private mutating func parseTemplateLiteral() throws -> Template {
        let (s, _) = try consumeStringLit()
        return makeTemplate(s)
    }

    private func makeTemplate(_ s: String) -> Template {
        var parts: [TemplatePart] = []
        var literal = ""
        var i = s.startIndex
        while i < s.endIndex {
            let c = s[i]
            if c == "{" {
                if !literal.isEmpty { parts.append(.literal(literal)); literal = "" }
                // Find matching }
                var j = s.index(after: i)
                var content = ""
                while j < s.endIndex && s[j] != "}" {
                    content.append(s[j])
                    j = s.index(after: j)
                }
                if j < s.endIndex {
                    // Lex-and-parse the interpolation as a full extraction
                    // expression — supports `$x`, `$x | transform`, and
                    // `coalesce(a, b)`.
                    if let ee = try? Self.parseExtractionSnippet(content) {
                        parts.append(.interp(ee))
                    } else {
                        // Fallback: treat as literal text (with the braces intact)
                        literal.append("{\(content)}")
                    }
                    i = s.index(after: j)
                    continue
                } else {
                    literal.append("{")
                    literal.append(content)
                }
            } else {
                literal.append(c)
            }
            i = s.index(after: i)
        }
        if !literal.isEmpty { parts.append(.literal(literal)) }
        return Template(parts: parts)
    }

    /// Helper: lex+parse a small extraction-expression snippet (the content of
    /// a `{...}` interpolation in a string literal).
    private static func parseExtractionSnippet(_ snippet: String) throws -> ExtractionExpr {
        var lex = Lexer(source: snippet)
        let toks = try lex.tokenize()
        var p = Parser(tokens: toks)
        return try p.parseExtractionExpr()
    }

    // MARK: - BrowserConfig (Jane recipes)

    private mutating func parseBrowserConfig() throws -> BrowserConfig {
        try expect(.lbrace, "{")
        var initialURL = Template(literal: "")
        var ageGate: AgeGateConfig?
        var dismissals: DismissalConfig?
        var warmupClicks: [String] = []
        var observe = ""
        var pagination: BrowserPaginationConfig?
        var captures: [CaptureRule] = []
        var documentCapture: DocumentCaptureRule?
        var interactive: InteractiveConfig?

        while !check(.rbrace) {
            if matchKeyword("initialURL") {
                try expect(.colon, ":")
                initialURL = try parseTemplateLiteral()
            } else if matchKeyword("ageGate") {
                try expect(.dot, ".")
                let kind = try consumeIdentifierOrKeyword()
                guard kind == "autoFill" else {
                    throw ParseError.unsupportedConstruct("ageGate.\(kind) not supported", loc: previous().loc)
                }
                try expect(.lbrace, "{")
                var year = 1990, month = 1, day = 1
                var reload = true
                while !check(.rbrace) {
                    let key = try consumeIdentifierOrKeyword()
                    try expect(.colon, ":")
                    if key == "dob" {
                        let date = try consumeDateLit()
                        year = date.year; month = date.month; day = date.day
                    } else if key == "reloadAfter" || key == "reloadAfterSubmit" {
                        reload = try consumeBoolLit()
                    } else {
                        throw ParseError.unsupportedConstruct("unknown ageGate.autoFill field '\(key)'", loc: previous().loc)
                    }
                    _ = match(.semicolon); _ = match(.comma)
                }
                try expect(.rbrace, "}")
                ageGate = AgeGateConfig(year: year, month: month, day: day, reloadAfter: reload)
            } else if matchKeyword("dismissals") {
                try expect(.lbrace, "{")
                var maxAttempts = 8
                var extraLabels: [String] = []
                while !check(.rbrace) {
                    let key = try consumeIdentifierOrKeyword()
                    try expect(.colon, ":")
                    if key == "maxAttempts" {
                        maxAttempts = try consumeIntLit()
                    } else if key == "extraLabels" {
                        extraLabels = try consumeStringArray()
                    } else {
                        throw ParseError.unsupportedConstruct("unknown dismissals field '\(key)'", loc: previous().loc)
                    }
                    _ = match(.semicolon); _ = match(.comma)
                }
                try expect(.rbrace, "}")
                dismissals = DismissalConfig(maxAttempts: maxAttempts, extraLabels: extraLabels)
            } else if matchKeyword("warmupClicks") {
                try expect(.colon, ":")
                warmupClicks = try consumeStringArray()
            } else if matchKeyword("observe") {
                try expect(.colon, ":")
                let (s, _) = try consumeStringLit()
                observe = s
            } else if matchKeyword("paginate") {
                pagination = try parseBrowserPaginationConfig()
            } else if matchKeyword("captures") {
                try expect(.dot, ".")
                let kind = try consumeIdentifierOrKeyword()
                switch kind {
                case "match":
                    let rule = try parseCaptureRule()
                    captures.append(rule)
                case "document":
                    guard documentCapture == nil else {
                        throw ParseError.unsupportedConstruct(
                            "captures.document declared more than once; only one document rule per recipe",
                            loc: previous().loc
                        )
                    }
                    documentCapture = try parseDocumentCaptureRule()
                default:
                    throw ParseError.unsupportedConstruct("captures.\(kind) not supported", loc: previous().loc)
                }
            } else if matchKeyword("interactive") {
                guard interactive == nil else {
                    throw ParseError.unsupportedConstruct(
                        "browser.interactive declared more than once",
                        loc: previous().loc
                    )
                }
                interactive = try parseInteractiveConfig()
            } else {
                throw ParseError.unsupportedConstruct("unknown browser field '\(peek().lexeme)'", loc: peek().loc)
            }
            _ = match(.semicolon)
        }
        try expect(.rbrace, "}")
        return BrowserConfig(
            initialURL: initialURL,
            ageGate: ageGate,
            dismissals: dismissals,
            warmupClicks: warmupClicks,
            observe: observe,
            pagination: pagination ?? BrowserPaginationConfig(mode: .scroll, until: .noProgressFor(3)),
            captures: captures,
            documentCapture: documentCapture,
            interactive: interactive
        )
    }

    /// Parse `browser.interactive { bootstrapURL: …, cookieDomains: […], gatePattern: "…" }`
    /// Used by M10 recipes that need a human handshake on first run.
    private mutating func parseInteractiveConfig() throws -> InteractiveConfig {
        try expect(.lbrace, "{")
        var bootstrapURL: Template? = nil
        var cookieDomains: [String] = []
        var gatePattern: String? = nil
        while !check(.rbrace) {
            if matchKeyword("bootstrapURL") {
                try expect(.colon, ":")
                bootstrapURL = try parseTemplateLiteral()
            } else if matchKeyword("cookieDomains") {
                try expect(.colon, ":")
                cookieDomains = try consumeStringArray()
            } else if matchKeyword("gatePattern") {
                try expect(.colon, ":")
                let (s, _) = try consumeStringLit()
                gatePattern = s
            } else {
                throw ParseError.unsupportedConstruct(
                    "unknown interactive field '\(peek().lexeme)'", loc: peek().loc
                )
            }
            _ = match(.semicolon); _ = match(.comma)
        }
        try expect(.rbrace, "}")
        return InteractiveConfig(
            bootstrapURL: bootstrapURL,
            cookieDomains: cookieDomains,
            gatePattern: gatePattern
        )
    }

    /// Parse `captures.document { for $x in <expr> { … } }`. Same body shape
    /// as `captures.match` but no `urlPattern` — the document body is
    /// matched by virtue of being the one document the browser settled on.
    private mutating func parseDocumentCaptureRule() throws -> DocumentCaptureRule {
        try expect(.lbrace, "{")
        var iterPath: ExtractionExpr = .path(.current)
        var body: [Statement] = []
        while !check(.rbrace) {
            if matchKeyword("for") {
                let stmt = try parseForLoop()
                body.append(stmt)
                if case .forLoop(_, let coll, _) = stmt { iterPath = coll }
            } else if checkKeyword("emit") {
                body.append(try parseStatement())
            } else {
                throw ParseError.unsupportedConstruct(
                    "unknown captures.document field '\(peek().lexeme)'", loc: peek().loc
                )
            }
            _ = match(.semicolon)
        }
        try expect(.rbrace, "}")
        return DocumentCaptureRule(iterPath: iterPath, body: body)
    }

    private mutating func parseBrowserPaginationConfig() throws -> BrowserPaginationConfig {
        // Two surface forms accepted:
        //   browserPaginate.scroll { … }
        //   browserPaginate { mode: scroll; … }     (alternative — not yet)
        // Today: require the dotted form.
        try expectKeyword("browserPaginate")
        try expect(.dot, ".")
        let modeName = try consumeIdentifierOrKeyword()
        let mode: BrowserPaginationConfig.Mode
        switch modeName {
        case "scroll": mode = .scroll
        case "replay": mode = .replay
        default:
            throw ParseError.unsupportedConstruct("browserPaginate mode '\(modeName)'", loc: previous().loc)
        }
        try expect(.lbrace, "{")
        var until: BrowserPaginateUntil = .noProgressFor(3)
        var maxIterations = 30
        var iterationDelay: TimeInterval = 1.8
        var seedFilter: String?
        while !check(.rbrace) {
            let key = try consumeIdentifierOrKeyword()
            try expect(.colon, ":")
            if key == "until" {
                let kind = try consumeIdentifierOrKeyword()
                if kind == "noProgressFor" {
                    try expect(.lparen, "(")
                    let n = try consumeIntLit()
                    try expect(.rparen, ")")
                    until = .noProgressFor(n)
                } else {
                    throw ParseError.unsupportedConstruct("until.\(kind)", loc: previous().loc)
                }
            } else if key == "maxIterations" {
                maxIterations = try consumeIntLit()
            } else if key == "iterationDelay" {
                if let d = matchDoubleLit() { iterationDelay = d }
                else if let i = matchIntLit() { iterationDelay = TimeInterval(i) }
                else { throw ParseError.unexpected(peek(), expected: "number") }
            } else if key == "seedFilter" {
                let (s, _) = try consumeStringLit()
                seedFilter = s
            } else {
                throw ParseError.unsupportedConstruct("unknown browserPaginate field '\(key)'", loc: previous().loc)
            }
            _ = match(.semicolon); _ = match(.comma)
        }
        try expect(.rbrace, "}")
        return BrowserPaginationConfig(
            mode: mode,
            until: until,
            maxIterations: maxIterations,
            iterationDelay: iterationDelay,
            seedFilter: seedFilter
        )
    }

    private mutating func parseCaptureRule() throws -> CaptureRule {
        try expect(.lbrace, "{")
        var urlPattern = ""
        var iterPath: ExtractionExpr = .path(.current)
        var body: [Statement] = []
        while !check(.rbrace) {
            if matchKeyword("urlPattern") {
                try expect(.colon, ":")
                let (s, _) = try consumeStringLit()
                urlPattern = s
            } else if matchKeyword("for") {
                let stmt = try parseForLoop()
                body.append(stmt)
                if case .forLoop(_, let coll, _) = stmt { iterPath = coll }
            } else if checkKeyword("emit") {
                body.append(try parseStatement())
            } else {
                throw ParseError.unsupportedConstruct("unknown captures.match field '\(peek().lexeme)'", loc: peek().loc)
            }
            _ = match(.semicolon)
        }
        try expect(.rbrace, "}")
        return CaptureRule(urlPattern: urlPattern, iterPath: iterPath, body: body)
    }

    // MARK: - Expectations

    private mutating func parseExpectation() throws -> Expectation {
        // Currently support: expect { records.where(typeName == "X").count <op> N }
        try expect(.lbrace, "{")
        try expectKeyword("records")
        try expect(.dot, ".")
        try expectKeyword("where")
        try expect(.lparen, "(")
        try expectKeyword("typeName")
        // ==
        try expect(.equal, "=")
        try expect(.equal, "=")
        let (typeName, _) = try consumeStringLit()
        try expect(.rparen, ")")
        try expect(.dot, ".")
        try expectKeyword("count")
        let opStr: String
        if match(.equal) {
            try expect(.equal, "=")
            opStr = "=="
        } else if match(.gt) {
            opStr = match(.equal) ? ">=" : ">"
        } else if match(.lt) {
            opStr = match(.equal) ? "<=" : "<"
        } else if match(.bang) {
            try expect(.equal, "=")
            opStr = "!="
        } else {
            throw ParseError.unexpected(peek(), expected: "comparison operator")
        }
        let n = try consumeIntLit()
        try expect(.rbrace, "}")
        let op: ComparisonOp
        switch opStr {
        case ">=": op = .ge
        case ">":  op = .gt
        case "<=": op = .le
        case "<":  op = .lt
        case "==": op = .eq
        case "!=": op = .ne
        default: op = .eq
        }
        return Expectation(.recordCount(typeName: typeName, op: op, value: n))
    }

    // MARK: - Token helpers

    private func peek() -> Token { tokens[pos] }
    private func previous() -> Token { tokens[max(pos - 1, 0)] }

    @discardableResult
    private mutating func advance() -> Token {
        defer { if pos < tokens.count - 1 { pos += 1 } }
        return tokens[pos]
    }

    private func check(_ kind: TokenKind) -> Bool {
        // Rough enum-case match; for cases with payload we compare just the case.
        switch (peek().kind, kind) {
        case (.lbrace, .lbrace), (.rbrace, .rbrace),
             (.lparen, .lparen), (.rparen, .rparen),
             (.lbracket, .lbracket), (.rbracket, .rbracket),
             (.comma, .comma), (.semicolon, .semicolon),
             (.colon, .colon), (.dot, .dot),
             (.question, .question), (.qDot, .qDot),
             (.wildcard, .wildcard), (.pipe, .pipe),
             (.arrow, .arrow), (.caseArrow, .caseArrow),
             (.equal, .equal), (.gt, .gt), (.lt, .lt), (.bang, .bang),
             (.dollarRoot, .dollarRoot),
             (.dollarInput, .dollarInput),
             (.dollarSecret, .dollarSecret),
             (.eof, .eof), (.nullLit, .nullLit):
            return true
        default: return false
        }
    }

    private func checkKeyword(_ word: String) -> Bool {
        if case .keyword(let k) = peek().kind { return k == word }
        return false
    }

    private func checkPathStart() -> Bool {
        switch peek().kind {
        case .dollarRoot, .dollarInput, .dollarSecret, .dollarVariable: return true
        default: return false
        }
    }

    @discardableResult
    private mutating func match(_ kind: TokenKind) -> Bool {
        if check(kind) { advance(); return true }
        return false
    }

    @discardableResult
    private mutating func matchKeyword(_ word: String) -> Bool {
        if checkKeyword(word) { advance(); return true }
        return false
    }

    private mutating func expect(_ kind: TokenKind, _ name: String) throws {
        if !check(kind) {
            throw ParseError.unexpected(peek(), expected: name)
        }
        advance()
    }

    private mutating func expectKeyword(_ word: String) throws {
        if !checkKeyword(word) {
            throw ParseError.unexpected(peek(), expected: "keyword '\(word)'")
        }
        advance()
    }

    private mutating func consumeStringLit() throws -> (String, SourceLoc) {
        if case .stringLit(let s) = peek().kind {
            let loc = peek().loc; advance(); return (s, loc)
        }
        throw ParseError.unexpected(peek(), expected: "string literal")
    }
    private mutating func matchStringLit() -> String? {
        if case .stringLit(let s) = peek().kind { advance(); return s }
        return nil
    }

    private mutating func consumeIntLit() throws -> Int {
        if case .intLit(let i) = peek().kind { advance(); return i }
        throw ParseError.unexpected(peek(), expected: "integer literal")
    }
    private mutating func matchIntLit() -> Int? {
        if case .intLit(let i) = peek().kind { advance(); return i }
        return nil
    }

    private mutating func matchDoubleLit() -> Double? {
        if case .doubleLit(let d) = peek().kind { advance(); return d }
        return nil
    }

    private mutating func consumeBoolLit() throws -> Bool {
        if case .boolLit(let b) = peek().kind { advance(); return b }
        throw ParseError.unexpected(peek(), expected: "bool literal")
    }
    private mutating func matchBoolLit() -> Bool? {
        if case .boolLit(let b) = peek().kind { advance(); return b }
        return nil
    }

    private mutating func matchNullLit() -> Bool {
        if case .nullLit = peek().kind { advance(); return true }
        return false
    }

    private mutating func consumeDateLit() throws -> (year: Int, month: Int, day: Int) {
        if case .dateLit(let y, let m, let d) = peek().kind { advance(); return (y, m, d) }
        throw ParseError.unexpected(peek(), expected: "date literal YYYY-MM-DD")
    }

    private mutating func consumeIdentifierOrKeyword() throws -> String {
        if case .identifier(let s) = peek().kind { advance(); return s }
        if case .keyword(let s) = peek().kind { advance(); return s }
        if case .typeName(let s) = peek().kind { advance(); return s }
        throw ParseError.unexpected(peek(), expected: "identifier")
    }
    private mutating func matchIdentifierOrKeyword() -> String? {
        if case .identifier(let s) = peek().kind { advance(); return s }
        if case .keyword(let s) = peek().kind { advance(); return s }
        return nil
    }

    private mutating func consumeTypeName() throws -> String {
        if case .typeName(let s) = peek().kind { advance(); return s }
        throw ParseError.unexpected(peek(), expected: "type name (capitalized)")
    }
    private mutating func matchTypeName() -> String? {
        if case .typeName(let s) = peek().kind { advance(); return s }
        return nil
    }
    private mutating func consumeTypeNameOrIdent() throws -> String {
        if case .typeName(let s) = peek().kind { advance(); return s }
        if case .identifier(let s) = peek().kind { advance(); return s }
        if case .keyword(let s) = peek().kind { advance(); return s }
        // case-of labels can also be primitive literals (bool / int / null / string).
        // We capture them as their canonical string form; the runtime stringifies
        // the scrutinee value to match.
        if case .boolLit(let b) = peek().kind { advance(); return String(b) }
        if case .intLit(let i) = peek().kind { advance(); return String(i) }
        if case .nullLit = peek().kind { advance(); return "null" }
        if case .stringLit(let s) = peek().kind { advance(); return s }
        throw ParseError.unexpected(peek(), expected: "type name, identifier, or literal")
    }

    private mutating func matchDollarVariable() -> String? {
        if case .dollarVariable(let s) = peek().kind { advance(); return s }
        return nil
    }
    private mutating func consumeDollarVariable() throws -> String {
        if case .dollarVariable(let s) = peek().kind { advance(); return s }
        throw ParseError.unexpected(peek(), expected: "$variable")
    }

    private mutating func consumeStringArray() throws -> [String] {
        try expect(.lbracket, "[")
        var out: [String] = []
        while !check(.rbracket) {
            let (s, _) = try consumeStringLit()
            out.append(s)
            _ = match(.comma)
        }
        try expect(.rbracket, "]")
        return out
    }

    private mutating func skipBracedBlock() throws {
        try expect(.lbrace, "{")
        var depth = 1
        while depth > 0 && !check(.eof) {
            if check(.lbrace) { depth += 1 }
            if check(.rbrace) { depth -= 1 }
            if depth == 0 { break }
            advance()
        }
        try expect(.rbrace, "}")
    }
}
