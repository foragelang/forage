// forage-probe — load and run a `.forage` recipe (browser-engine kind), or
// drop into a passive WKWebView fixture-capture mode for a URL when no
// recipe is supplied. Used to:
//
//   - Reverse-engineer SPA-on-CF sites without being blocked by Cloudflare
//     (we ARE a real browser, so CF treats us like Safari does).
//   - Validate browser-engine recipes (does observe pattern match? does
//     scroll-mode produce progress? are captures.match rules well-formed?).
//   - Generate fixtures for recipe development.
//
// Usage:
//   .build/debug/forage-probe run <recipe.forage> [--input k=v]…
//   .build/debug/forage-probe capture <url> [output.jsonl] [settle-s] [timeout-s]
//
// `run` parses the recipe, constructs a Recipe value, runs through
// BrowserEngine, prints the resulting Snapshot as JSON to stdout.
// `capture` is the legacy passive-capture mode for reverse-engineering.

import Foundation
import AppKit
import WebKit
import Forage

let app = NSApplication.shared
app.setActivationPolicy(.regular)

let args = CommandLine.arguments
let mode = args.count >= 2 ? args[1] : "capture"

switch mode {
case "run":
    runRecipe()
case "capture":
    captureMode()
default:
    FileHandle.standardError.write("usage: forage-probe run <recipe.forage> | capture <url> [out] [settle] [timeout]\n".data(using: .utf8)!)
    exit(2)
}

app.activate(ignoringOtherApps: true)
app.run()

// MARK: - Commands

func runRecipe() {
    guard CommandLine.arguments.count >= 3 else {
        FileHandle.standardError.write("forage-probe run: need recipe path\n".data(using: .utf8)!)
        exit(2)
    }
    let path = CommandLine.arguments[2]
    let extras = Array(CommandLine.arguments.dropFirst(3))

    do {
        let src = try String(contentsOfFile: path, encoding: .utf8)
        let recipe = try Parser.parse(source: src)
        let issues = Validator.validate(recipe)
        if issues.hasErrors {
            FileHandle.standardError.write("validation failed:\n".data(using: .utf8)!)
            for e in issues.errors {
                FileHandle.standardError.write(" - \(e.message) [\(e.location)]\n".data(using: .utf8)!)
            }
            exit(1)
        }
        for w in issues.warnings {
            FileHandle.standardError.write("warning: \(w.message) [\(w.location)]\n".data(using: .utf8)!)
        }

        let inputs = parseExtras(extras)

        Task { @MainActor in
            do {
                let result: RunResult
                if recipe.engineKind == .browser {
                    let engine = BrowserEngine(recipe: recipe, inputs: inputs)
                    result = try await engine.run()
                } else {
                    let runner = RecipeRunner(httpClient: HTTPClient(transport: URLSessionTransport()))
                    result = try await runner.run(recipe: recipe, inputs: inputs)
                }
                let data = try SnapshotIO.encode(result.snapshot)
                FileHandle.standardOutput.write(data)
                FileHandle.standardOutput.write("\n".data(using: .utf8)!)
                FileHandle.standardError.write("stallReason: \(result.report.stallReason)\n".data(using: .utf8)!)
                NSApp.terminate(nil)
            } catch {
                FileHandle.standardError.write("run failed: \(error)\n".data(using: .utf8)!)
                exit(1)
            }
        }
    } catch {
        FileHandle.standardError.write("parse failed: \(error)\n".data(using: .utf8)!)
        exit(1)
    }
}

/// Parse `--input k=v` arguments into a `[String: JSONValue]`. Values are
/// best-effort: numeric → int/double, "true"/"false" → bool, JSON literals
/// (`[1,2]`, `{"a":1}`) are decoded, else string.
func parseExtras(_ extras: [String]) -> [String: JSONValue] {
    var out: [String: JSONValue] = [:]
    var i = 0
    while i < extras.count {
        let a = extras[i]
        if a == "--input", i + 1 < extras.count {
            let kv = extras[i + 1]
            if let eq = kv.firstIndex(of: "=") {
                let key = String(kv[..<eq])
                let raw = String(kv[kv.index(after: eq)...])
                out[key] = parseInputValue(raw)
            }
            i += 2
        } else { i += 1 }
    }
    return out
}

func parseInputValue(_ raw: String) -> JSONValue {
    if let i = Int(raw) { return .int(i) }
    if let d = Double(raw) { return .double(d) }
    if raw == "true" { return .bool(true) }
    if raw == "false" { return .bool(false) }
    if raw == "null" { return .null }
    if raw.hasPrefix("[") || raw.hasPrefix("{") {
        if let data = raw.data(using: .utf8), let v = try? JSONValue.decode(data) { return v }
    }
    return .string(raw)
}

// MARK: - Capture mode (legacy reverse-engineering tool)

func captureMode() {
    let urlStr = CommandLine.arguments.count >= 3 ? CommandLine.arguments[2] : "https://trilogy.health/shop/adult-use-menu/"
    let outPath = CommandLine.arguments.count >= 4 ? CommandLine.arguments[3] : "/tmp/probe-captures.jsonl"
    let settle = (CommandLine.arguments.count >= 5) ? (TimeInterval(CommandLine.arguments[4]) ?? 8) : 8
    let timeout = (CommandLine.arguments.count >= 6) ? (TimeInterval(CommandLine.arguments[5]) ?? 60) : 60

    Task { @MainActor in
        let probe = CaptureSession(url: URL(string: urlStr)!, outputURL: URL(fileURLWithPath: outPath), settle: settle, hardTimeout: timeout)
        probe.run()
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

    init(url: URL, outputURL: URL, settle: TimeInterval, hardTimeout: TimeInterval) {
        self.url = url
        self.outputURL = outputURL
        self.settle = settle
        self.hardTimeout = hardTimeout

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
        win.title = "Probe — \(url.host ?? "")"
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
        NSApp.terminate(nil)
    }

    private func log(_ s: String) {
        if let data = "[probe] \(s)\n".data(using: .utf8) {
            FileHandle.standardError.write(data)
        }
    }

    nonisolated func userContentController(_ ucc: WKUserContentController, didReceive message: WKScriptMessage) {
        let body = message.body
        MainActor.assumeIsolated {
            guard let dict = body as? [String: Any],
                  let cap = Capture(jsBridgePayload: dict) else { return }
            resetSettleTimer()
            // Append as JSONL
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
