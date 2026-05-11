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
