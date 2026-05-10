import Foundation

/// A single fetch / XHR exchange observed by `Forage`'s injected JS wrapper.
///
/// Captures are produced by `InjectedScripts.captureWrapper`, which patches the
/// page's `window.fetch` and `XMLHttpRequest` to forward every request +
/// response to the Swift host via a `WKScriptMessageHandler`. Engines (and
/// pagination strategies) consume the stream to detect progress, identify
/// seeds for replay-mode pagination, and emit the diagnostic JSONL artifact.
public struct Capture: Hashable, Sendable {
    public enum Kind: String, Hashable, Sendable {
        case fetch
        case xhr
        case diagnostic   // synthetic — emitted by injected JS for debugging
    }

    public let id: UUID
    public let timestamp: Date
    public let kind: Kind
    public let method: String
    public let requestUrl: String
    public let responseUrl: String
    public let requestBody: String
    public let status: Int
    public let bodyLength: Int
    public let body: String

    public init(
        id: UUID = UUID(),
        timestamp: Date,
        kind: Kind,
        method: String,
        requestUrl: String,
        responseUrl: String,
        requestBody: String,
        status: Int,
        bodyLength: Int,
        body: String
    ) {
        self.id = id
        self.timestamp = timestamp
        self.kind = kind
        self.method = method
        self.requestUrl = requestUrl
        self.responseUrl = responseUrl
        self.requestBody = requestBody
        self.status = status
        self.bodyLength = bodyLength
        self.body = body
    }

    /// Construct from the raw payload posted by the injected JS bridge.
    /// Returns `nil` if the payload is malformed.
    public init?(jsBridgePayload payload: [String: Any]) {
        guard let kindStr = payload["kind"] as? String,
              let kind = Kind(rawValue: kindStr) else { return nil }
        let body = (payload["body"] as? String) ?? ""
        self.init(
            timestamp: Date(),
            kind: kind,
            method: (payload["method"] as? String) ?? "?",
            requestUrl: (payload["requestUrl"] as? String) ?? "",
            responseUrl: (payload["responseUrl"] as? String) ?? "",
            requestBody: (payload["requestBody"] as? String) ?? "",
            status: (payload["status"] as? Int) ?? -1,
            bodyLength: body.count,
            body: body
        )
    }
}

extension Capture {
    /// Serializable dict suitable for JSONL dumping.
    public var jsonlDict: [String: Any] {
        [
            "timestamp": ISO8601DateFormatter().string(from: timestamp),
            "kind": kind.rawValue,
            "method": method,
            "requestUrl": requestUrl,
            "responseUrl": responseUrl,
            "requestBody": requestBody,
            "status": status,
            "bodyLength": bodyLength,
            "body": body,
        ]
    }
}
