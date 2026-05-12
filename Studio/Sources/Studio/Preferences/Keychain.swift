import Foundation
import Security

/// Tiny wrapper around `SecItem*` for storing the Forage hub API key in
/// the user's login keychain. Single keyed entry, service identifier baked
/// in — no per-user namespacing because the Toolkit's a single-user app.
enum Keychain {
    static let service = "com.foragelang.Toolkit"
    static let account = "hub-api-key"

    static func readAPIKey() throws -> String {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        var item: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &item)
        switch status {
        case errSecSuccess:
            if let data = item as? Data, let s = String(data: data, encoding: .utf8) {
                return s
            }
            throw KeychainError.decodeFailed
        case errSecItemNotFound:
            throw KeychainError.notFound
        default:
            throw KeychainError.osStatus(status)
        }
    }

    static func writeAPIKey(_ key: String) throws {
        guard let data = key.data(using: .utf8) else { throw KeychainError.encodeFailed }

        let baseQuery: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]

        let existsStatus = SecItemCopyMatching(baseQuery as CFDictionary, nil)
        if existsStatus == errSecSuccess {
            let attrs: [String: Any] = [kSecValueData as String: data]
            let status = SecItemUpdate(baseQuery as CFDictionary, attrs as CFDictionary)
            guard status == errSecSuccess else { throw KeychainError.osStatus(status) }
        } else {
            var addQuery = baseQuery
            addQuery[kSecValueData as String] = data
            let status = SecItemAdd(addQuery as CFDictionary, nil)
            guard status == errSecSuccess else { throw KeychainError.osStatus(status) }
        }
    }

    static func deleteAPIKey() throws {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
        let status = SecItemDelete(query as CFDictionary)
        if status != errSecSuccess && status != errSecItemNotFound {
            throw KeychainError.osStatus(status)
        }
    }

    // MARK: - M11 OAuth tokens

    /// Stored auth bundle: GitHub login + access/refresh JWTs from the
    /// device-code flow. Stored as one JSON blob under
    /// `account = "hub-oauth-tokens"`. Distinct from the legacy API key
    /// (still supported as the admin / fallback path).
    struct OAuthTokens: Codable {
        let login: String
        let accessToken: String
        let refreshToken: String
        let updatedAt: Date
    }

    static let oauthAccount = "hub-oauth-tokens"

    static func readOAuthTokens() throws -> OAuthTokens {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: oauthAccount,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        var item: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &item)
        switch status {
        case errSecSuccess:
            guard let data = item as? Data else { throw KeychainError.decodeFailed }
            let decoder = JSONDecoder()
            decoder.dateDecodingStrategy = .iso8601
            do {
                return try decoder.decode(OAuthTokens.self, from: data)
            } catch {
                throw KeychainError.decodeFailed
            }
        case errSecItemNotFound:
            throw KeychainError.notFound
        default:
            throw KeychainError.osStatus(status)
        }
    }

    static func writeOAuthTokens(_ tokens: OAuthTokens) throws {
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        let data = try encoder.encode(tokens)

        let baseQuery: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: oauthAccount,
        ]
        let existsStatus = SecItemCopyMatching(baseQuery as CFDictionary, nil)
        if existsStatus == errSecSuccess {
            let attrs: [String: Any] = [kSecValueData as String: data]
            let status = SecItemUpdate(baseQuery as CFDictionary, attrs as CFDictionary)
            guard status == errSecSuccess else { throw KeychainError.osStatus(status) }
        } else {
            var addQuery = baseQuery
            addQuery[kSecValueData as String] = data
            let status = SecItemAdd(addQuery as CFDictionary, nil)
            guard status == errSecSuccess else { throw KeychainError.osStatus(status) }
        }
    }

    static func deleteOAuthTokens() throws {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: oauthAccount,
        ]
        let status = SecItemDelete(query as CFDictionary)
        if status != errSecSuccess && status != errSecItemNotFound {
            throw KeychainError.osStatus(status)
        }
    }
}

enum KeychainError: Error, CustomStringConvertible {
    case notFound
    case decodeFailed
    case encodeFailed
    case osStatus(OSStatus)

    var description: String {
        switch self {
        case .notFound:       return "Keychain item not found."
        case .decodeFailed:   return "Keychain item couldn't be decoded as UTF-8."
        case .encodeFailed:   return "API key couldn't be encoded as UTF-8."
        case .osStatus(let s): return "Keychain error: OSStatus \(s)"
        }
    }
}
