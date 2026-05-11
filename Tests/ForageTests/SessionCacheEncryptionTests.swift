import Testing
import Foundation
import CryptoKit
@testable import Forage

/// Tests for the M7 D7.10 followup — session-cache encryption-at-rest.
/// The SessionCache machinery has supported AES-GCM all along; what was
/// missing until now was the key supplier. `SessionCacheKeyProvider`
/// closes that loop: `KeychainSessionCacheKeyProvider` in production,
/// `InMemorySessionCacheKeyProvider` in tests, `NullSessionCacheKeyProvider`
/// when no Keychain is reachable (degrades to chmod-600 plaintext).
struct SessionCacheEncryptionTests {

    @Test func encryptedRoundTripWithProviderRecoversState() throws {
        let provider = InMemorySessionCacheKeyProvider()
        let key = provider.key()
        #expect(key != nil)

        let url = tempCacheFile("encrypted-roundtrip")
        defer { try? FileManager.default.removeItem(at: url) }

        let state = SessionState(payload: .cookies([
            SessionCookie(name: "s", value: "abc", domain: "example.com", path: "/")
        ]))

        try SessionCache.write(state, to: url, encryptionKey: key)

        // Raw bytes are NOT plaintext JSON.
        let raw = try Data(contentsOf: url)
        let asText = String(data: raw, encoding: .utf8) ?? ""
        #expect(!asText.contains("cookies"))
        #expect(!asText.contains("example.com"))

        // Round-trip with the same key recovers the state.
        let decoded = SessionCache.read(at: url, encryptionKey: key)
        #expect(decoded != nil)
        if case .cookies(let cookies) = decoded?.payload {
            #expect(cookies.first?.value == "abc")
        } else {
            Issue.record("expected cookies payload, got \(String(describing: decoded?.payload))")
        }
    }

    @Test func nullProviderFallsBackToPlaintext() throws {
        let provider = NullSessionCacheKeyProvider()
        #expect(provider.key() == nil)

        let url = tempCacheFile("null-provider")
        defer { try? FileManager.default.removeItem(at: url) }

        let state = SessionState(payload: .bearer(
            token: "T", headerName: "Authorization", headerPrefix: "Bearer "
        ))

        try SessionCache.write(state, to: url, encryptionKey: provider.key())

        // No key → plaintext, so the body shows up in the bytes.
        let raw = try Data(contentsOf: url)
        let asText = String(data: raw, encoding: .utf8) ?? ""
        #expect(asText.contains("Authorization"))
    }

    @Test func wrongKeyFailsToDecrypt() throws {
        let writer = InMemorySessionCacheKeyProvider()
        let reader = InMemorySessionCacheKeyProvider()  // different random key
        #expect(writer.key() != nil && reader.key() != nil)

        let url = tempCacheFile("wrong-key")
        defer { try? FileManager.default.removeItem(at: url) }

        let state = SessionState(payload: .cookies([]))
        try SessionCache.write(state, to: url, encryptionKey: writer.key())

        let recovered = SessionCache.read(at: url, encryptionKey: reader.key())
        #expect(recovered == nil)
    }

    @Test func cacheFileMaintainsChmod600() throws {
        let url = tempCacheFile("chmod-check")
        defer { try? FileManager.default.removeItem(at: url) }

        let state = SessionState(payload: .cookies([]))
        try SessionCache.write(state, to: url, encryptionKey: InMemorySessionCacheKeyProvider().key())

        let attrs = try FileManager.default.attributesOfItem(atPath: url.path)
        let perms = (attrs[.posixPermissions] as? NSNumber)?.intValue ?? 0
        #expect(perms == 0o600)
    }

    // MARK: - Helpers

    private func tempCacheFile(_ name: String) -> URL {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("forage-cache-test-\(UUID().uuidString)", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir.appendingPathComponent("\(name).bin")
    }
}
