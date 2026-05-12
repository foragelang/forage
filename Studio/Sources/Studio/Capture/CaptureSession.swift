import Foundation
import AppKit
import WebKit
import Forage
import Observation

/// Owns a `WKWebView` preconfigured with `InjectedScripts.captureWrapper`
/// (the same JS the engine uses) and exposes the live capture stream as an
/// `@Observable` array of `LiveCapture` rows (one per fetch / XHR).
///
/// Unlike `BrowserEngine`, no recipe is involved: we just capture everything
/// observed and let the UI mark rows keep/skip before writing them out as
/// JSONL.
@MainActor
@Observable
final class CaptureSession: NSObject, WKNavigationDelegate, WKScriptMessageHandler {
    let webView: WKWebView
    var captures: [LiveCapture] = []
    var currentURL: String?

    override init() {
        let config = WKWebViewConfiguration()
        config.websiteDataStore = .default()
        let ucc = WKUserContentController()
        let captureScript = WKUserScript(
            source: InjectedScripts.captureWrapper,
            injectionTime: .atDocumentStart,
            forMainFrameOnly: false
        )
        ucc.addUserScript(captureScript)
        config.userContentController = ucc
        self.webView = WKWebView(
            frame: NSRect(x: 0, y: 0, width: 1100, height: 700),
            configuration: config
        )
        super.init()
        ucc.add(self, name: "captureNetwork")
        webView.navigationDelegate = self
    }

    func load(urlString: String) {
        guard let url = URL(string: urlString) else { return }
        currentURL = urlString
        webView.load(URLRequest(url: url))
    }

    func clearCaptures() {
        captures.removeAll()
    }

    /// Persist `keep=true` captures as JSONL to `outputURL`. The format
    /// matches `BrowserReplayer`'s reader (one Capture per line, ISO8601
    /// dates, response body inline).
    func saveKeptCaptures(to outputURL: URL, append: Bool) throws {
        let kept = captures.filter(\.keep).map(\.capture)
        try writeJSONL(captures: kept, to: outputURL, append: append)
    }

    // MARK: - WKScriptMessageHandler

    nonisolated func userContentController(_ ucc: WKUserContentController, didReceive message: WKScriptMessage) {
        MainActor.assumeIsolated {
            guard let dict = message.body as? [String: Any],
                  let cap = Capture(jsBridgePayload: dict) else { return }
            captures.append(LiveCapture(capture: cap, keep: true))
        }
    }

    // MARK: - WKNavigationDelegate

    nonisolated func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        MainActor.assumeIsolated {
            currentURL = webView.url?.absoluteString ?? currentURL
        }
    }

    // MARK: - JSONL writer

    private func writeJSONL(captures: [Capture], to url: URL, append: Bool) throws {
        let fm = FileManager.default
        try fm.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        if !append || !fm.fileExists(atPath: url.path) {
            try Data().write(to: url)
        }
        let handle = try FileHandle(forWritingTo: url)
        defer { try? handle.close() }
        try handle.seekToEnd()
        for cap in captures {
            let record: [String: Any] = [
                "id": cap.id.uuidString,
                "timestamp": ISO8601DateFormatter().string(from: cap.timestamp),
                "kind": cap.kind.rawValue,
                "method": cap.method,
                "requestUrl": cap.requestUrl,
                "responseUrl": cap.responseUrl,
                "requestBody": cap.requestBody,
                "status": cap.status,
                "bodyLength": cap.bodyLength,
                "body": cap.body,
            ]
            let data = try JSONSerialization.data(withJSONObject: record, options: [.sortedKeys, .withoutEscapingSlashes])
            try handle.write(contentsOf: data)
            try handle.write(contentsOf: Data([0x0A]))
        }
    }
}

/// One row in the capture feed. `keep` toggles whether the row is included
/// when the user saves to fixtures.
struct LiveCapture: Identifiable, Hashable {
    let capture: Capture
    var keep: Bool
    var id: UUID { capture.id }
}
