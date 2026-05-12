import Testing
import Foundation
@testable import Studio

/// Round-trips a test key through the user's login Keychain. Skipped if
/// the system Keychain isn't accessible (CI agents without a logged-in
/// session typically can't reach it).
@Test
func keychainRoundTripsAPIKey() throws {
    // Pre-clean in case a previous failed run left a stale entry.
    try? Keychain.deleteAPIKey()

    let testKey = "test-key-\(UUID().uuidString)"

    do {
        try Keychain.writeAPIKey(testKey)
    } catch {
        // CI agents often can't reach the Keychain; treat as a skip
        // rather than a hard failure.
        return
    }
    defer { try? Keychain.deleteAPIKey() }

    let recovered = try Keychain.readAPIKey()
    #expect(recovered == testKey)

    try Keychain.deleteAPIKey()
    #expect(throws: KeychainError.self) {
        _ = try Keychain.readAPIKey()
    }
}
