//! Token kinds produced by the lexer and consumed by the parser.

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    // Punctuation / brackets
    LBrace,
    RBrace,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Comma,
    Semicolon,
    Colon,
    Dot,
    Question,
    /// `?.`
    QDot,
    /// `[*]` recognized as a single token by the lexer.
    Wildcard,
    Pipe,
    /// `←` (left arrow — binding).
    Arrow,
    /// `→` (right arrow — case branch).
    CaseArrow,
    Equal,
    Gt,
    Lt,
    Bang,

    // Path-expression heads.
    /// Bare `$` (followed by `.` for current-value paths).
    DollarRoot,
    /// `$input`
    DollarInput,
    /// `$secret`
    DollarSecret,
    /// `$<ident>`
    DollarVar(String),

    // Literals.
    Str(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    Null,
    Date {
        year: i32,
        month: u32,
        day: u32,
    },

    /// Docker-style recipe reference, scanned after `import`.
    Ref(String),

    /// Lowercase-starting identifier.
    Ident(String),
    /// Uppercase-starting identifier.
    TypeName(String),
    /// Reserved word.
    Keyword(String),
}

impl Token {
    /// Display name for diagnostics.
    pub fn describe(&self) -> &'static str {
        match self {
            Token::LBrace => "'{'",
            Token::RBrace => "'}'",
            Token::LParen => "'('",
            Token::RParen => "')'",
            Token::LBracket => "'['",
            Token::RBracket => "']'",
            Token::Comma => "','",
            Token::Semicolon => "';'",
            Token::Colon => "':'",
            Token::Dot => "'.'",
            Token::Question => "'?'",
            Token::QDot => "'?.'",
            Token::Wildcard => "'[*]'",
            Token::Pipe => "'|'",
            Token::Arrow => "'\u{2190}'",
            Token::CaseArrow => "'\u{2192}'",
            Token::Equal => "'='",
            Token::Gt => "'>'",
            Token::Lt => "'<'",
            Token::Bang => "'!'",
            Token::DollarRoot => "'$'",
            Token::DollarInput => "'$input'",
            Token::DollarSecret => "'$secret'",
            Token::DollarVar(_) => "'$<var>'",
            Token::Str(_) => "string literal",
            Token::Int(_) => "integer literal",
            Token::Float(_) => "float literal",
            Token::Bool(_) => "boolean literal",
            Token::Null => "'null'",
            Token::Date { .. } => "date literal",
            Token::Ref(_) => "recipe ref",
            Token::Ident(_) => "identifier",
            Token::TypeName(_) => "type name",
            Token::Keyword(_) => "keyword",
        }
    }
}

/// All reserved words in Forage.
pub const KEYWORDS: &[&str] = &[
    "import",
    "recipe",
    "engine",
    "http",
    "browser",
    "type",
    "enum",
    "input",
    "step",
    "method",
    "url",
    "headers",
    "body",
    "json",
    "form",
    "raw",
    "auth",
    "staticHeader",
    "htmlPrime",
    "extract",
    "regex",
    "groups",
    "paginate",
    "pageWithTotal",
    "untilEmpty",
    "cursor",
    "items",
    "total",
    "pageParam",
    "pageSize",
    "cursorPath",
    "cursorParam",
    "for",
    "in",
    "emit",
    "case",
    "of",
    "let",
    "where",
    "expect",
    "true",
    "false",
    "null",
    "observe",
    "browserPaginate",
    "scroll",
    "replay",
    "ageGate",
    "autoFill",
    "warmupClicks",
    "navigate",
    "until",
    "noProgressFor",
    "maxIterations",
    "iterationDelay",
    "seedFilter",
    "replayOverride",
    "captures",
    "match",
    "dismissals",
    "dob",
    "reloadAfter",
    "reloadAfterSubmit",
    "name",
    "value",
    "stepName",
    "nonceVar",
    "ajaxUrlVar",
    "pageZeroIndexed",
    "records",
    "count",
    "typeName",
    "initialURL",
    "loadMoreLabels",
    "extraLabels",
    "captureExtractions",
    "iterPath",
    "urlPattern",
    "withCookies",
    "as",
    "String",
    "Int",
    "Double",
    "Bool",
    // Session auth.
    "secret",
    "session",
    "formLogin",
    "bearerLogin",
    "cookiePersist",
    "captureCookies",
    "maxReauthRetries",
    "cache",
    "cacheEncrypted",
    "requiresMFA",
    "mfaFieldName",
    "tokenPath",
    "headerName",
    "headerPrefix",
    "sourcePath",
    "format",
    // Interactive bootstrap (M10).
    "interactive",
    "bootstrapURL",
    "cookieDomains",
    "sessionExpiredPattern",
    // Document captures (M9).
    "document",
];

pub fn is_keyword(s: &str) -> bool {
    KEYWORDS.contains(&s)
}

pub const TYPE_KEYWORDS: &[&str] = &["String", "Int", "Double", "Bool"];
