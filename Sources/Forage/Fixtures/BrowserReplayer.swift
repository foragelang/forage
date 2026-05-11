import Foundation

/// Drives a `BrowserEngine` run from a pre-recorded list of `Capture` values
/// instead of from a live `WKWebView` session. Pair this with the
/// `captures.jsonl` an `Archive.write(...)` produced and you can iterate a
/// recipe's extraction logic against a frozen page response set without
/// hitting the network, the age gate, or the SPA.
///
/// Construct from a file written by `Archive.write` (`captures.jsonl`) or
/// directly from an in-memory list. `BrowserEngine.init(...,
/// replayer:)` accepts one of these and short-circuits its live pipeline:
/// no navigation, no age-gate / dismissal / warmup, no settle / hard-
/// timeout timers; just feed each capture through the same capture handler
/// the JS bridge calls and let the recipe's `captures.match` rules run.
public struct BrowserReplayer: Sendable {
    public let captures: [Capture]

    public init(captures: [Capture]) {
        self.captures = captures
    }

    /// Read a JSONL file in the format `Archive.write` produces (one
    /// `Capture` per line, ISO8601 dates). An empty file is allowed and
    /// produces an empty replayer — a degenerate but valid replay set.
    public init(capturesFile url: URL) throws {
        let data = try Data(contentsOf: url)
        self.captures = try Self.decodeJSONL(data)
    }

    // MARK: - JSONL

    private static func decodeJSONL(_ data: Data) throws -> [Capture] {
        guard !data.isEmpty else { return [] }
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        var captures: [Capture] = []
        var lineStart = data.startIndex
        for i in data.indices {
            if data[i] == 0x0A {
                if i > lineStart {
                    let line = data[lineStart..<i]
                    captures.append(try decoder.decode(Capture.self, from: line))
                }
                lineStart = data.index(after: i)
            }
        }
        if lineStart < data.endIndex {
            let tail = data[lineStart..<data.endIndex]
            captures.append(try decoder.decode(Capture.self, from: tail))
        }
        return captures
    }
}
