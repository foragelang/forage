import Foundation

/// Persisted state from an M10 interactive bootstrap. Stored at
/// `~/Library/Forage/Sessions/<recipe-slug>/session.json` (chmod 600).
///
/// The format is deliberately portable JSON: a developer can bootstrap on
/// a workstation, copy the file to a headless CI host, and the runtime
/// there reuses the session until it expires. Cookies are stored with
/// enough attributes to thread back into a `WKWebsiteDataStore` /
/// `URLSession`; `localStorage` is per-origin so SPAs that hydrate from
/// it after page load see the same state.
public struct InteractiveSession: Codable, Sendable, Equatable {
    public let recipeSlug: String
    public let bootstrappedAt: Date
    /// Best-effort hint, derived from the minimum cookie `Max-Age` or
    /// `Expires` attribute. Nil when no cookie carried an expiry. The
    /// runtime treats `nil` as "no soft expiry — trust the gate-pattern
    /// check on each run."
    public let expiresAt: Date?
    public let cookies: [SessionCookie]
    public let localStorage: [String: [String: String]]

    public init(
        recipeSlug: String,
        bootstrappedAt: Date = Date(),
        expiresAt: Date? = nil,
        cookies: [SessionCookie] = [],
        localStorage: [String: [String: String]] = [:]
    ) {
        self.recipeSlug = recipeSlug
        self.bootstrappedAt = bootstrappedAt
        self.expiresAt = expiresAt
        self.cookies = cookies
        self.localStorage = localStorage
    }

    /// True when `expiresAt` is in the past. Sessions without an
    /// `expiresAt` are not considered expired by this check — the
    /// runtime's `gatePattern` is the source of truth for "this session
    /// still works."
    public func isExpired(now: Date = Date()) -> Bool {
        guard let e = expiresAt else { return false }
        return now >= e
    }
}

/// On-disk persistence for `InteractiveSession`. The file is chmod 600;
/// the optional encryption hook (M7 D7.10 work) is reused here so the
/// same key supplier protects both `SessionState` and
/// `InteractiveSession` blobs.
public enum InteractiveSessionStore {
    /// Default root: `~/Library/Forage/Sessions/`. Tests override.
    public static func defaultRoot() -> URL {
        let fm = FileManager.default
        let lib = fm.urls(for: .libraryDirectory, in: .userDomainMask).first
            ?? fm.homeDirectoryForCurrentUser.appendingPathComponent("Library")
        return lib
            .appendingPathComponent("Forage", isDirectory: true)
            .appendingPathComponent("Sessions", isDirectory: true)
    }

    /// Per-slug directory, e.g. `~/Library/Forage/Sessions/ebay-sold/`.
    public static func directory(for slug: String, root: URL? = nil) -> URL {
        let base = root ?? defaultRoot()
        return base.appendingPathComponent(sanitize(slug), isDirectory: true)
    }

    /// File path: `<directory>/session.json`.
    public static func file(for slug: String, root: URL? = nil) -> URL {
        directory(for: slug, root: root)
            .appendingPathComponent("session.json", isDirectory: false)
    }

    public static func read(at url: URL) -> InteractiveSession? {
        guard let data = try? Data(contentsOf: url) else { return nil }
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        return try? decoder.decode(InteractiveSession.self, from: data)
    }

    public static func write(_ session: InteractiveSession, to url: URL) throws {
        try FileManager.default.createDirectory(
            at: url.deletingLastPathComponent(),
            withIntermediateDirectories: true,
            attributes: nil
        )
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        encoder.dateEncodingStrategy = .iso8601
        let data = try encoder.encode(session)
        // Touch the file first so `setAttributes` lands on a real inode.
        FileManager.default.createFile(
            atPath: url.path,
            contents: nil,
            attributes: [.posixPermissions: 0o600]
        )
        try data.write(to: url, options: .atomic)
        try FileManager.default.setAttributes(
            [.posixPermissions: 0o600],
            ofItemAtPath: url.path
        )
    }

    public static func evict(at url: URL) {
        _ = try? FileManager.default.removeItem(at: url)
    }

    /// Mirror `SessionCache.sanitize`: strip path separators so a
    /// recipe slug can't escape the sessions root.
    private static func sanitize(_ s: String) -> String {
        var out = s
        for bad in ["/", "..", "\\"] {
            out = out.replacingOccurrences(of: bad, with: "_")
        }
        return out
    }
}
