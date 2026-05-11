import Foundation
import CryptoKit

#if canImport(Security)
import Security
#endif

/// Supplies the symmetric key used to encrypt session-cache files when
/// the recipe sets `auth.session.cacheEncrypted: true`.
///
/// Returning `nil` from `key()` means the engine falls back to plaintext
/// (still `chmod 600`) — used for tests, headless contexts without a
/// keychain, or hosts that haven't wired up a key source yet.
public protocol SessionCacheKeyProvider: Sendable {
    func key() -> SymmetricKey?
}

/// Default Keychain-backed key provider for macOS/iOS. On first use,
/// generates a fresh 256-bit AES key and stores it as a
/// `kSecClassGenericPassword` SecItem. Subsequent reads fetch the
/// existing key. The SecItem is accessible only after-first-unlock so
/// the cache works after a reboot, but the key never syncs to iCloud
/// and never leaves the device.
public struct KeychainSessionCacheKeyProvider: SessionCacheKeyProvider {
    public let service: String
    public let account: String

    public init(
        service: String = "com.foragelang.forage.session-cache",
        account: String = "encryption-key"
    ) {
        self.service = service
        self.account = account
    }

    public func key() -> SymmetricKey? {
        #if canImport(Security)
        if let existing = readExisting() { return existing }
        return generateAndStore()
        #else
        return nil
        #endif
    }

    #if canImport(Security)
    private func readExisting() -> SymmetricKey? {
        let query: [String: Any] = [
            kSecClass as String:       kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecReturnData as String:  true,
            kSecMatchLimit as String:  kSecMatchLimitOne,
        ]
        var item: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &item)
        guard status == errSecSuccess, let data = item as? Data else { return nil }
        return SymmetricKey(data: data)
    }

    private func generateAndStore() -> SymmetricKey? {
        let new = SymmetricKey(size: .bits256)
        let bytes = new.withUnsafeBytes { Data($0) }
        let attrs: [String: Any] = [
            kSecClass as String:           kSecClassGenericPassword,
            kSecAttrService as String:     service,
            kSecAttrAccount as String:     account,
            kSecAttrAccessible as String:  kSecAttrAccessibleAfterFirstUnlock,
            kSecAttrSynchronizable as String: false,
            kSecValueData as String:       bytes,
        ]
        let status = SecItemAdd(attrs as CFDictionary, nil)
        if status == errSecDuplicateItem {
            // Race: another process beat us to it. Read the existing one.
            return readExisting()
        }
        guard status == errSecSuccess else { return nil }
        return new
    }
    #endif
}

/// In-memory key provider for tests. Holds a fixed key so test
/// fixtures encrypt and decrypt deterministically.
public struct InMemorySessionCacheKeyProvider: SessionCacheKeyProvider {
    private let backing: SymmetricKey

    public init(key: SymmetricKey = SymmetricKey(size: .bits256)) {
        self.backing = key
    }

    public func key() -> SymmetricKey? { backing }
}

/// Provider that always returns `nil`. The fallback when no keychain is
/// reachable; the engine treats `nil` as "skip encryption, keep
/// chmod-600 plaintext."
public struct NullSessionCacheKeyProvider: SessionCacheKeyProvider {
    public init() {}
    public func key() -> SymmetricKey? { nil }
}
