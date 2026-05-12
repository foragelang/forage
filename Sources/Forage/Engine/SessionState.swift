import Foundation
import CryptoKit

/// Runtime state held by the HTTP engine while a session-auth recipe runs.
/// Snapshotted to disk (`auth.session.cache`) so subsequent runs skip the
/// login flow. Two payload shapes: cookies (from form/cookiePersist) or a
/// bearer token (from bearerLogin). One is non-empty at a time.
public struct SessionState: Codable, Sendable, Equatable {
    public enum Payload: Codable, Sendable, Equatable {
        case cookies([SessionCookie])
        case bearer(token: String, headerName: String, headerPrefix: String)
    }

    /// When the session was established (Unix seconds). Used to honor
    /// `auth.session.cache: <duration>` — the cache is expired when
    /// `now - createdAt >= duration`.
    public var createdAt: Date
    public var payload: Payload

    public init(createdAt: Date = Date(), payload: Payload) {
        self.createdAt = createdAt
        self.payload = payload
    }

    public func isExpired(now: Date = Date(), maxAge: TimeInterval) -> Bool {
        now.timeIntervalSince(createdAt) >= maxAge
    }
}

/// A captured HTTP cookie — `name=value` plus the attributes we need to
/// thread it back into a request `Cookie:` header. We deliberately keep the
/// shape minimal; tests don't need full RFC 6265 fidelity.
public struct SessionCookie: Codable, Sendable, Equatable, Hashable {
    public var name: String
    public var value: String
    public var domain: String?
    public var path: String?

    public init(name: String, value: String, domain: String? = nil, path: String? = nil) {
        self.name = name
        self.value = value
        self.domain = domain
        self.path = path
    }
}

// MARK: - Cookie serialization

extension SessionCookie {
    /// Render a list of cookies as a single `Cookie:` header value.
    public static func headerValue(_ cookies: [SessionCookie]) -> String {
        cookies.map { "\($0.name)=\($0.value)" }.joined(separator: "; ")
    }

    /// Parse one or more `Set-Cookie` header values into our trimmed
    /// `SessionCookie` shape. Only the `name=value`, `Domain=`, and `Path=`
    /// attributes are retained; the rest is dropped.
    public static func parseSetCookieHeaders(_ values: [String]) -> [SessionCookie] {
        var out: [SessionCookie] = []
        for raw in values {
            let parts = raw.split(separator: ";", omittingEmptySubsequences: true).map { $0.trimmingCharacters(in: .whitespaces) }
            guard let first = parts.first, let eq = first.firstIndex(of: "=") else { continue }
            let name = String(first[..<eq])
            let value = String(first[first.index(after: eq)...])
            var domain: String? = nil
            var path: String? = nil
            for attr in parts.dropFirst() {
                let lower = attr.lowercased()
                if lower.hasPrefix("domain=") { domain = String(attr.dropFirst("domain=".count)) }
                else if lower.hasPrefix("path=") { path = String(attr.dropFirst("path=".count)) }
            }
            out.append(SessionCookie(name: name, value: value, domain: domain, path: path))
        }
        return out
    }
}

// MARK: - Cache file management

/// `~/Library/Forage/Cache/sessions/<recipe-slug>/<fingerprint>.json`
/// Files are written with `chmod 600`. Optional AES-GCM encryption keyed by
/// a per-machine secret held in the macOS Keychain.
public enum SessionCache {
    /// Path used by both Studio and CLI hosts. `recipeSlug` is sanitized
    /// (path-separator-stripped) before joining.
    public static func cacheDir(for recipeSlug: String, root: URL? = nil) -> URL {
        let base = root ?? defaultRoot()
        let safe = sanitize(recipeSlug)
        return base
            .appendingPathComponent("sessions", isDirectory: true)
            .appendingPathComponent(safe, isDirectory: true)
    }

    public static func cacheFile(
        for recipeSlug: String,
        fingerprint: String,
        root: URL? = nil
    ) -> URL {
        cacheDir(for: recipeSlug, root: root)
            .appendingPathComponent("\(fingerprint).json", isDirectory: false)
    }

    /// Default cache root: `~/Library/Forage/Cache`.
    public static func defaultRoot() -> URL {
        let fm = FileManager.default
        let lib = fm.urls(for: .libraryDirectory, in: .userDomainMask).first ?? fm.homeDirectoryForCurrentUser.appendingPathComponent("Library")
        return lib.appendingPathComponent("Forage", isDirectory: true).appendingPathComponent("Cache", isDirectory: true)
    }

    /// Read and decode a cache file. Returns `nil` on any I/O or decode
    /// error (caller treats missing/invalid as cache-miss).
    public static func read(at url: URL, encryptionKey: SymmetricKey? = nil) -> SessionState? {
        guard let data = try? Data(contentsOf: url) else { return nil }
        if let key = encryptionKey {
            guard let plain = decrypt(data, key: key) else { return nil }
            return try? JSONDecoder().decode(SessionState.self, from: plain)
        }
        return try? JSONDecoder().decode(SessionState.self, from: data)
    }

    /// Encode and write a cache file, chmod 600. Throws on encoding /
    /// I/O / attribute-set failures.
    public static func write(
        _ state: SessionState,
        to url: URL,
        encryptionKey: SymmetricKey? = nil
    ) throws {
        try FileManager.default.createDirectory(
            at: url.deletingLastPathComponent(),
            withIntermediateDirectories: true,
            attributes: nil
        )
        let json = try JSONEncoder().encode(state)
        let payload: Data
        if let key = encryptionKey {
            payload = try encrypt(json, key: key)
        } else {
            payload = json
        }
        // Touch the file first so `setAttributes` lands on a real inode.
        FileManager.default.createFile(atPath: url.path, contents: nil, attributes: [.posixPermissions: 0o600])
        try payload.write(to: url, options: .atomic)
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: url.path)
    }

    /// Delete a cache file. Missing-file is not an error.
    public static func evict(at url: URL) {
        _ = try? FileManager.default.removeItem(at: url)
    }

    // MARK: - Encryption (optional)

    /// AES-GCM with a 32-byte key. Studio stores a per-machine key in
    /// the macOS Keychain and threads it in here when the recipe opts in via
    /// `auth.session.cacheEncrypted: true`.
    public static func encrypt(_ plaintext: Data, key: SymmetricKey) throws -> Data {
        let sealed = try AES.GCM.seal(plaintext, using: key)
        guard let combined = sealed.combined else { throw SessionCacheError.encryptionFailed }
        return combined
    }

    public static func decrypt(_ ciphertext: Data, key: SymmetricKey) -> Data? {
        guard let box = try? AES.GCM.SealedBox(combined: ciphertext) else { return nil }
        return try? AES.GCM.open(box, using: key)
    }

    // MARK: - Internals

    private static func sanitize(_ slug: String) -> String {
        slug.replacingOccurrences(of: "/", with: "_")
            .replacingOccurrences(of: ":", with: "_")
            .replacingOccurrences(of: "..", with: "_")
    }
}

// MARK: - Credential fingerprint

/// Stable hash over the resolved values of the secrets a recipe's auth
/// block references. Used as the cache filename so a credential rotation
/// produces a fresh cache entry instead of mixing with stale state.
public enum CredentialFingerprint {
    /// SHA256 over `name1\0value1\0name2\0value2…` of sorted names.
    /// Hex-encoded; first 16 chars are enough to keep cache filenames short.
    public static func compute(secrets: [String: String]) -> String {
        let sortedNames = secrets.keys.sorted()
        var hasher = SHA256()
        for n in sortedNames {
            hasher.update(data: Data(n.utf8))
            hasher.update(data: Data([0]))
            hasher.update(data: Data((secrets[n] ?? "").utf8))
            hasher.update(data: Data([0]))
        }
        let digest = hasher.finalize()
        let hex = digest.map { String(format: "%02x", $0) }.joined()
        return String(hex.prefix(16))
    }
}

public enum SessionCacheError: Error, CustomStringConvertible {
    case encryptionFailed

    public var description: String {
        switch self {
        case .encryptionFailed: return "session cache: AES-GCM encryption failed"
        }
    }
}

// MARK: - SecretRedactor

/// Replace every occurrence of any resolved secret value with `<redacted>`
/// in a candidate string. Used when surfacing `DiagnosticReport.stallReason`
/// so accidental credential leaks via `failed: <error>` envelopes don't
/// reach logs or the snapshot output.
public struct SecretRedactor: Sendable {
    public let values: [String]

    public init(_ values: [String]) {
        // Skip empty + very short values — a 1-character "secret" matching
        // every occurrence of that character would do more harm than good.
        // 4 chars is the threshold the redactor needs to be useful without
        // being destructive (passwords, tokens, API keys are all longer).
        self.values = values.filter { $0.count >= 4 }
    }

    public init(from scope: Scope) {
        self.init(Array(scope.secrets.values))
    }

    public func redact(_ s: String) -> String {
        var out = s
        for v in values { out = out.replacingOccurrences(of: v, with: "<redacted>") }
        return out
    }
}
