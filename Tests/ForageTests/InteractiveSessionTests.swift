import Testing
import Foundation
@testable import Forage

/// Tests for the M10 interactive-bootstrap pieces that don't require a
/// live WKWebView:
///
/// - Parser accepts and round-trips `browser.interactive { … }`.
/// - `InteractiveSession` JSON round-trip is portable.
/// - `InteractiveSessionStore` writes with `chmod 600` and reads back.
/// - Expired sessions are detected.
struct InteractiveSessionTests {

    @Test func parserAcceptsInteractiveBlock() throws {
        let source = """
            recipe "ebay-sold" {
                engine browser

                type Listing { title: String }

                input query: String

                browser {
                    initialURL: "https://www.ebay.com/sch/i.html?_nkw={$input.query}&LH_Sold=1"
                    observe:    "ebay.com"
                    paginate browserPaginate.scroll {
                        until: noProgressFor(2)
                        maxIterations: 0
                    }

                    interactive {
                        bootstrapURL:  "https://www.ebay.com/sch/i.html?_nkw={$input.query}&LH_Sold=1"
                        cookieDomains: ["ebay.com", ".ebay.com"]
                        gatePattern:   "Security Measure"
                    }

                    captures.document {
                        for $card in $ | select(".s-item") {
                            emit Listing {
                                title ← $card | select(".s-item__title") | text
                            }
                        }
                    }
                }
            }
            """
        let recipe = try Parser.parse(source: source)
        #expect(!Validator.validate(recipe).hasErrors)
        let cfg = recipe.browser?.interactive
        #expect(cfg != nil)
        #expect(cfg?.cookieDomains == ["ebay.com", ".ebay.com"])
        #expect(cfg?.gatePattern == "Security Measure")
    }

    @Test func parserRejectsDuplicateInteractiveBlock() {
        let source = """
            recipe "dup-interactive" {
                engine browser

                type Item { title: String }

                browser {
                    initialURL: "https://example.com"
                    observe:    "example.com"
                    paginate browserPaginate.scroll {
                        until: noProgressFor(3)
                    }
                    interactive {
                        gatePattern: "gate-a"
                    }
                    interactive {
                        gatePattern: "gate-b"
                    }
                }
            }
            """
        #expect(throws: (any Error).self) {
            _ = try Parser.parse(source: source)
        }
    }

    @Test func sessionJSONRoundTrip() throws {
        let original = InteractiveSession(
            recipeSlug: "ebay-sold",
            bootstrappedAt: Date(timeIntervalSince1970: 1_750_000_000),
            expiresAt: Date(timeIntervalSince1970: 1_900_000_000),
            cookies: [
                SessionCookie(name: "sid", value: "abc123", domain: ".ebay.com", path: "/"),
                SessionCookie(name: "trk", value: "xyz", domain: "ebay.com", path: "/")
            ],
            localStorage: [
                "https://www.ebay.com": ["userPref": "dark", "lastSeen": "42"]
            ]
        )

        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        let data = try encoder.encode(original)

        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        let decoded = try decoder.decode(InteractiveSession.self, from: data)

        #expect(decoded == original)
    }

    @Test func storeWriteRoundTripWithChmod600() throws {
        let tmpRoot = tempRoot()
        defer { try? FileManager.default.removeItem(at: tmpRoot) }

        let session = InteractiveSession(
            recipeSlug: "ebay-sold",
            cookies: [SessionCookie(name: "s", value: "v", domain: "ebay.com", path: "/")]
        )
        let url = InteractiveSessionStore.file(for: "ebay-sold", root: tmpRoot)
        try InteractiveSessionStore.write(session, to: url)

        // Round-trip recovers identical content.
        let decoded = InteractiveSessionStore.read(at: url)
        #expect(decoded?.recipeSlug == "ebay-sold")
        #expect(decoded?.cookies.first?.value == "v")

        // Permissions: 600.
        let attrs = try FileManager.default.attributesOfItem(atPath: url.path)
        let perms = (attrs[.posixPermissions] as? NSNumber)?.intValue ?? 0
        #expect(perms == 0o600)
    }

    @Test func storeEvictRemovesFile() throws {
        let tmpRoot = tempRoot()
        defer { try? FileManager.default.removeItem(at: tmpRoot) }
        let session = InteractiveSession(recipeSlug: "evict-test")
        let url = InteractiveSessionStore.file(for: "evict-test", root: tmpRoot)
        try InteractiveSessionStore.write(session, to: url)
        #expect(FileManager.default.fileExists(atPath: url.path))
        InteractiveSessionStore.evict(at: url)
        #expect(!FileManager.default.fileExists(atPath: url.path))
    }

    @Test func expiredSessionDetected() {
        let past = InteractiveSession(
            recipeSlug: "expired",
            expiresAt: Date(timeIntervalSinceNow: -60)  // 1 min ago
        )
        #expect(past.isExpired())

        let future = InteractiveSession(
            recipeSlug: "fresh",
            expiresAt: Date(timeIntervalSinceNow: 3600)  // 1h from now
        )
        #expect(!future.isExpired())

        let openEnded = InteractiveSession(recipeSlug: "no-expiry")
        #expect(!openEnded.isExpired())
    }

    @Test func slugWithPathSeparatorsSanitized() {
        let url = InteractiveSessionStore.file(for: "../../../etc/passwd", root: URL(fileURLWithPath: "/tmp"))
        // Sanitized: no ".." or "/" survives in the path component.
        #expect(!url.path.contains("/etc/passwd"))
        // Lives under /tmp regardless.
        #expect(url.path.hasPrefix("/tmp"))
    }

    private func tempRoot() -> URL {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("forage-interactive-\(UUID().uuidString)", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }
}
