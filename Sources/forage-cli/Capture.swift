import ArgumentParser
import Foundation
import AppKit
import WebKit
import Forage

struct CaptureCommand: AsyncParsableCommand {
    static let configuration = CommandConfiguration(
        commandName: "capture",
        abstract: "Launch a WKWebView and record every fetch / XHR exchange to JSONL."
    )

    @Argument(help: "URL to load.")
    var url: String

    @Option(name: .customLong("out"), help: "Output JSONL path.")
    var out: String = "/tmp/probe-captures.jsonl"

    @Option(name: .customLong("settle"), help: "Idle seconds before finishing.")
    var settle: Double = 8

    @Option(name: .customLong("timeout"), help: "Hard timeout in seconds.")
    var timeout: Double = 60

    func run() async throws {
        guard let pageURL = URL(string: url) else {
            FileHandle.standardError.write("invalid URL: \(url)\n".data(using: .utf8)!)
            throw ExitCode.failure
        }
        try await Self.runCapture(
            url: pageURL,
            output: URL(fileURLWithPath: out),
            settle: settle,
            timeout: timeout
        )
    }

    @MainActor
    static func runCapture(
        url: URL,
        output: URL,
        settle: TimeInterval,
        timeout: TimeInterval
    ) async throws {
        let app = NSApplication.shared
        app.setActivationPolicy(.regular)
        app.activate(ignoringOtherApps: true)

        try await withCheckedThrowingContinuation { (cont: CheckedContinuation<Void, any Error>) in
            let session = CaptureSession(
                url: url,
                outputURL: output,
                settle: settle,
                hardTimeout: timeout,
                continuation: cont
            )
            session.run()
        }
    }
}

@MainActor
final class CaptureSession: NSObject, WKNavigationDelegate, WKScriptMessageHandler {
    let url: URL
    let outputURL: URL
    let settle: TimeInterval
    let hardTimeout: TimeInterval
    let webView: WKWebView
    var window: NSWindow?
    var settleTimer: Timer?
    var hardTimer: Timer?
    private var continuation: CheckedContinuation<Void, any Error>?

    init(
        url: URL,
        outputURL: URL,
        settle: TimeInterval,
        hardTimeout: TimeInterval,
        continuation: CheckedContinuation<Void, any Error>
    ) {
        self.url = url
        self.outputURL = outputURL
        self.settle = settle
        self.hardTimeout = hardTimeout
        self.continuation = continuation

        let config = WKWebViewConfiguration()
        config.websiteDataStore = .default()
        let ucc = WKUserContentController()
        let script = WKUserScript(
            source: InjectedScripts.captureWrapper,
            injectionTime: .atDocumentStart,
            forMainFrameOnly: false
        )
        ucc.addUserScript(script)
        self.webView = WKWebView(
            frame: NSRect(x: 0, y: 0, width: 1280, height: 900),
            configuration: config
        )
        super.init()
        ucc.add(self, name: "captureNetwork")
        webView.navigationDelegate = self
    }

    func run() {
        try? Data().write(to: outputURL)
        log("loading \(url.absoluteString)")
        log("dumping captures to \(outputURL.path)")
        let win = NSWindow(
            contentRect: NSRect(x: 60, y: 60, width: 1280, height: 900),
            styleMask: [.titled, .closable, .resizable],
            backing: .buffered,
            defer: false
        )
        win.title = "Capture — \(url.host ?? "")"
        win.contentView = webView
        win.makeKeyAndOrderFront(nil)
        self.window = win
        webView.load(URLRequest(url: url))

        hardTimer = Timer.scheduledTimer(withTimeInterval: hardTimeout, repeats: false) { [weak self] _ in
            self?.finish(reason: "hard-timeout")
        }
        resetSettleTimer()
    }

    private func resetSettleTimer() {
        settleTimer?.invalidate()
        settleTimer = Timer.scheduledTimer(withTimeInterval: settle, repeats: false) { [weak self] _ in
            self?.finish(reason: "settled")
        }
    }

    private func finish(reason: String) {
        log("finishing (\(reason))")
        settleTimer?.invalidate(); settleTimer = nil
        hardTimer?.invalidate(); hardTimer = nil
        if let cont = continuation {
            continuation = nil
            window?.close()
            cont.resume()
        }
    }

    private func log(_ s: String) {
        if let data = "[capture] \(s)\n".data(using: .utf8) {
            FileHandle.standardError.write(data)
        }
    }

    nonisolated func userContentController(_ ucc: WKUserContentController, didReceive message: WKScriptMessage) {
        let body = message.body
        MainActor.assumeIsolated {
            guard let dict = body as? [String: Any],
                  let cap = Capture(jsBridgePayload: dict) else { return }
            resetSettleTimer()
            let record: [String: Any] = [
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
            if let data = try? JSONSerialization.data(withJSONObject: record, options: [.sortedKeys]) {
                var line = data
                line.append(0x0A)
                if FileManager.default.fileExists(atPath: outputURL.path),
                   let handle = try? FileHandle(forWritingTo: outputURL) {
                    defer { try? handle.close() }
                    try? handle.seekToEnd()
                    try? handle.write(contentsOf: line)
                } else {
                    try? line.write(to: outputURL)
                }
            }
            let host = URL(string: cap.responseUrl)?.host ?? "?"
            log("\(cap.method) \(host) → \(cap.status) (\(cap.bodyLength) B)")
        }
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        log("page loaded — title=\(webView.title ?? "")")
    }
    func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: any Error) {
        log("nav error: \(error.localizedDescription)")
    }
    func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: any Error) {
        log("nav error (provisional): \(error.localizedDescription)")
    }
}
