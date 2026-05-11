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

    /// Stateful authenticated session — log in, maintain cookies / bearer
    /// token across requests, re-authenticate on 401/403. Three flavors:
    /// form-login (cookies), bearer-login (token), cookie-persist (escape
    /// hatch). See `SessionAuth`.
    case session(SessionAuth)
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

// MARK: - SessionAuth

/// `auth.session.<variant>` block. Drives a login flow before the first
/// step runs and re-auths on 401/403 mid-run.
public struct SessionAuth: Hashable, Sendable {
    public let kind: Kind
    /// How many times to re-authenticate after a 401/403 before giving up.
    /// 0 disables re-auth; 1 (default) does it once.
    public let maxReauthRetries: Int
    /// If non-nil, the resolved session is persisted to
    /// `~/Library/Forage/Cache/sessions/<recipe>/<fingerprint>.json` and
    /// reused on subsequent runs until this many seconds elapse.
    public let cacheDuration: TimeInterval?
    /// Encrypt the cache file at rest using a per-machine keychain-stored
    /// AES key. Default false — the cache file is `chmod 600` regardless.
    public let cacheEncrypted: Bool
    /// Login flow requires a second-factor code; the engine pauses, calls
    /// the host's `MFAProvider`, then retries the login with the code.
    public let requiresMFA: Bool
    /// Field name to attach the MFA code to in the login body. Default `code`.
    public let mfaFieldName: String

    public enum Kind: Hashable, Sendable {
        case formLogin(FormLogin)
        case bearerLogin(BearerLogin)
        case cookiePersist(CookiePersist)
    }

    public init(
        kind: Kind,
        maxReauthRetries: Int = 1,
        cacheDuration: TimeInterval? = nil,
        cacheEncrypted: Bool = false,
        requiresMFA: Bool = false,
        mfaFieldName: String = "code"
    ) {
        self.kind = kind
        self.maxReauthRetries = maxReauthRetries
        self.cacheDuration = cacheDuration
        self.cacheEncrypted = cacheEncrypted
        self.requiresMFA = requiresMFA
        self.mfaFieldName = mfaFieldName
    }
}

/// POST credentials to a login endpoint; the engine captures
/// `Set-Cookie`s and threads them as `Cookie:` on every subsequent step.
public struct FormLogin: Hashable, Sendable {
    public let url: Template
    public let method: String          // typically "POST"
    public let body: HTTPBody
    public let captureCookies: Bool

    public init(url: Template, method: String = "POST", body: HTTPBody, captureCookies: Bool = true) {
        self.url = url
        self.method = method
        self.body = body
        self.captureCookies = captureCookies
    }
}

/// POST credentials to a token endpoint; the engine extracts a bearer
/// token from the response via `tokenPath` and injects it as
/// `<headerName>: <headerPrefix><token>` on every subsequent step.
public struct BearerLogin: Hashable, Sendable {
    public let url: Template
    public let method: String
    public let body: HTTPBody
    public let tokenPath: PathExpr      // e.g. $.access_token
    public let headerName: String       // default "Authorization"
    public let headerPrefix: String     // default "Bearer "

    public init(
        url: Template,
        method: String = "POST",
        body: HTTPBody,
        tokenPath: PathExpr,
        headerName: String = "Authorization",
        headerPrefix: String = "Bearer "
    ) {
        self.url = url
        self.method = method
        self.body = body
        self.tokenPath = tokenPath
        self.headerName = headerName
        self.headerPrefix = headerPrefix
    }
}

/// Load cookies from a file. Escape hatch for sites whose login the
/// recipe can't drive (MFA across multiple devices, embedded CAPTCHAs).
public struct CookiePersist: Hashable, Sendable {
    public let sourcePath: Template
    public let format: CookieFormat

    public init(sourcePath: Template, format: CookieFormat = .json) {
        self.sourcePath = sourcePath
        self.format = format
    }
}

public enum CookieFormat: String, Hashable, Sendable, Codable {
    /// JSON array `[{"name": "...", "value": "...", "domain": "..."}, ...]`.
    case json
    /// Netscape `cookies.txt` format — tab-separated.
    case netscape
}
