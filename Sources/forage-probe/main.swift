// forage-probe — WKWebView-hosted reverse-engineering CLI for Forage recipes.
//
// Loads a URL in a real macOS browser engine, captures every fetch/XHR fired
// by the page (via Forage's injected JS wrapper), drives optional pagination
// (scroll or replay), dumps captures as JSONL + the page's interactable
// affordances at exit, and terminates after the page goes quiet.
//
// Used to:
//  - Reverse-engineer SPA-on-CF sites without being blocked by Cloudflare
//    (we ARE a real browser, so CF treats us like Safari does)
//  - Validate recipe assumptions (does observe pattern match? does the
//    seed filter pick the right request? does scroll-mode produce progress?)
//  - Generate fixtures for recipe development
//
// Compile + run:
//   swift build -c release --product forage-probe
//   .build/release/forage-probe <url> [output.jsonl] [settle-s] \
//     [hard-timeout-s] [paginate-mode] [observe-pattern] [seed-filter]
//
// Defaults: trilogy adult-use menu, /tmp/probe-captures.jsonl, settle=8s,
// timeout=60s, paginate=off, observe=iheartjane.com/v2/multi.

import Foundation
import AppKit
import WebKit
import Forage

// MARK: - Args

struct Args {
    let url: URL
    let outputURL: URL
    let settleSeconds: TimeInterval
    let hardTimeoutSeconds: TimeInterval
    let autoDismiss: Bool
    let visible: Bool
    let paginateMode: String
    let observe: String
    let replayOverride: [String: Any]
    let seedFilter: String?
}

func parseArgs() -> Args {
    let argv = CommandLine.arguments
    let urlStr = argv.count >= 2 ? argv[1] : "https://trilogy.health/shop/adult-use-menu/"
    let outPath = argv.count >= 3 ? argv[2] : "/tmp/probe-captures.jsonl"
    let settle = (argv.count >= 4) ? (TimeInterval(argv[3]) ?? 8) : 8
    let timeout = (argv.count >= 5) ? (TimeInterval(argv[4]) ?? 60) : 60
    let paginate = argv.count >= 6 ? argv[5].lowercased() : "off"
    let observe = argv.count >= 7 ? argv[6] : "iheartjane.com/v2/multi"
    let seedFilter = argv.count >= 8 ? argv[7] : (paginate == "replay" ? "menu_inline_table" : "")
    return Args(
        url: URL(string: urlStr)!,
        outputURL: URL(fileURLWithPath: outPath),
        settleSeconds: settle,
        hardTimeoutSeconds: timeout,
        autoDismiss: true,
        visible: true,
        paginateMode: paginate,
        observe: observe,
        replayOverride: ["placements.0.page": "$i"],
        seedFilter: seedFilter.isEmpty ? nil : seedFilter
    )
}

// MARK: - Probe

@MainActor
final class Probe: NSObject, WKNavigationDelegate, WKScriptMessageHandler, BrowserPaginateHost {
    let args: Args
    let webView: WKWebView
    var window: NSWindow?
    var settleTimer: Timer?
    var hardTimer: Timer?
    var captureCount = 0
    var didFinishNav = false
    var paginate: BrowserPaginate?
    var captures: [Capture] = []

    init(args: Args) {
        self.args = args

        let config = WKWebViewConfiguration()
        if CommandLine.arguments.contains("--fresh") {
            config.websiteDataStore = .nonPersistent()
        } else {
            config.websiteDataStore = .default()
        }
        let ucc = WKUserContentController()
        let captureScript = WKUserScript(
            source: InjectedScripts.captureWrapper,
            injectionTime: .atDocumentStart,
            forMainFrameOnly: false
        )
        ucc.addUserScript(captureScript)
        config.userContentController = ucc

        self.webView = WKWebView(
            frame: NSRect(x: 0, y: 0, width: 1280, height: 900),
            configuration: config
        )
        super.init()
        ucc.add(self, name: "captureNetwork")
        webView.navigationDelegate = self

        if let mode = BrowserPaginate.Mode(rawValue: args.paginateMode), mode != .off {
            self.paginate = BrowserPaginate(
                observe: args.observe,
                mode: mode,
                replayOverride: args.replayOverride,
                seedFilter: args.seedFilter
            )
            self.paginate?.host = self
        }
    }

    func run() {
        try? Data().write(to: args.outputURL)
        log("loading \(args.url.absoluteString)")
        log("dumping captures to \(args.outputURL.path)")

        if args.visible {
            let win = NSWindow(
                contentRect: NSRect(x: 60, y: 60, width: 1280, height: 900),
                styleMask: [.titled, .closable, .resizable],
                backing: .buffered,
                defer: false
            )
            win.title = "Probe — \(args.url.host ?? "")"
            win.contentView = webView
            win.makeKeyAndOrderFront(nil)
            self.window = win
        }

        webView.load(URLRequest(url: args.url))

        hardTimer = Timer.scheduledTimer(withTimeInterval: args.hardTimeoutSeconds, repeats: false) { [weak self] _ in
            self?.finish(reason: "hard-timeout")
        }
        resetSettleTimer()
    }

    private func resetSettleTimer() {
        settleTimer?.invalidate()
        settleTimer = Timer.scheduledTimer(withTimeInterval: args.settleSeconds, repeats: false) { [weak self] _ in
            self?.finish(reason: "settled")
        }
    }

    private func finish(reason: String) {
        log("finishing (\(reason)) — \(captureCount) captures dumped to \(args.outputURL.path)")
        NSApp.terminate(nil)
    }

    // MARK: - Dismissal orchestration

    private var dismissAttempts = 0
    private let maxDismissAttempts = 8

    private func attemptAutoDismiss() {
        guard dismissAttempts < maxDismissAttempts else {
            log("dismiss: gave up after \(dismissAttempts) attempts")
            dumpHTMLOnce()
            return
        }
        dismissAttempts += 1
        webView.evaluateJavaScript(InjectedScripts.ageGateFill) { [weak self] result, _ in
            guard let self else { return }
            if let s = result as? String, !s.isEmpty {
                self.log("dismiss[\(self.dismissAttempts)]: \(s)")
                DispatchQueue.main.asyncAfter(deadline: .now() + 2.0) { [weak self] in
                    self?.log("dismiss[\(self?.dismissAttempts ?? 0)]: reloading page after age-gate submit")
                    self?.webView.reload()
                }
                DispatchQueue.main.asyncAfter(deadline: .now() + 4.0) { [weak self] in
                    self?.dismissAttempts = 0
                    self?.attemptAutoDismiss()
                }
                return
            }
            self.webView.evaluateJavaScript(InjectedScripts.dismissModal) { [weak self] result, _ in
                guard let self else { return }
                if let s = result as? String, !s.isEmpty {
                    self.log("dismiss[\(self.dismissAttempts)]: clicked '\(s)'")
                    DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) { [weak self] in
                        self?.attemptAutoDismiss()
                    }
                } else {
                    self.log("dismiss[\(self.dismissAttempts)]: no match")
                    self.runWarmupClicksThenPaginate()
                    self.dumpHTMLOnce()
                }
            }
        }
    }

    private var didRunWarmup = false
    private func runWarmupClicksThenPaginate() {
        guard !didRunWarmup else { return }
        didRunWarmup = true
        let labels: [String] = []  // future: lift into recipe config
        clickButtonsInSequence(labels) { [weak self] in
            DispatchQueue.main.asyncAfter(deadline: .now() + 2.0) { [weak self] in
                self?.paginate?.start()
            }
        }
    }

    private func clickButtonsInSequence(_ labels: [String], completion: @escaping () -> Void) {
        var remaining = labels
        func step() {
            guard !remaining.isEmpty else { completion(); return }
            let label = remaining.removeFirst()
            let js = InjectedScripts.clickButtonByText(label)
            self.webView.evaluateJavaScript(js) { [weak self] result, _ in
                let status = (result as? String) ?? "?"
                self?.log("warmup-click '\(label)': \(status)")
                DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) { step() }
            }
        }
        step()
    }

    private var didDumpHTML = false
    private func dumpHTMLOnce() {
        guard !didDumpHTML else { return }
        didDumpHTML = true
        webView.evaluateJavaScript("document.documentElement.outerHTML") { [weak self] result, _ in
            guard let self, let html = result as? String else { return }
            let url = self.args.outputURL.deletingLastPathComponent().appendingPathComponent("probe-page.html")
            try? html.data(using: .utf8)?.write(to: url)
            self.log("dumped page HTML (\(html.count) chars) to \(url.path)")
        }
        dumpAffordances()
    }

    private func dumpAffordances() {
        webView.evaluateJavaScript(InjectedScripts.dumpAffordances) { [weak self] result, _ in
            guard let self, let s = result as? String,
                  let data = s.data(using: .utf8) else { return }
            let url = self.args.outputURL.deletingLastPathComponent().appendingPathComponent("probe-affordances.json")
            try? data.write(to: url)
            if let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any] {
                let buttons = (obj["buttons"] as? [Any])?.count ?? 0
                let links = (obj["links"] as? [Any])?.count ?? 0
                let roleButtons = (obj["roleButtons"] as? [Any])?.count ?? 0
                let scrollables = (obj["scrollables"] as? [Any])?.count ?? 0
                let inputs = (obj["inputs"] as? [Any])?.count ?? 0
                self.log("affordances: \(buttons) buttons / \(links) links / \(roleButtons) role=button / \(inputs) inputs / \(scrollables) scrollables → \(url.lastPathComponent)")
            }
        }
    }

    // MARK: - WKNavigationDelegate

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        didFinishNav = true
        log("page loaded — title=\(webView.title ?? "")")
        if args.autoDismiss {
            DispatchQueue.main.asyncAfter(deadline: .now() + 2.0) { [weak self] in
                self?.attemptAutoDismiss()
            }
        }
    }

    func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) {
        log("nav error: \(error.localizedDescription)")
    }

    func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) {
        log("nav error (provisional): \(error.localizedDescription)")
    }

    // MARK: - WKScriptMessageHandler

    nonisolated func userContentController(_ ucc: WKUserContentController, didReceive message: WKScriptMessage) {
        let body = message.body
        MainActor.assumeIsolated {
            guard let dict = body as? [String: Any],
                  let capture = Capture(jsBridgePayload: dict) else { return }
            handleCapture(capture)
        }
    }

    private func handleCapture(_ capture: Capture) {
        captureCount += 1
        resetSettleTimer()
        captures.append(capture)
        appendJSONL(capture)
        let host = URL(string: capture.responseUrl)?.host ?? "?"
        log("\(capture.method) \(host) → \(capture.status) (\(capture.bodyLength) B)")
        paginate?.handleCapture(capture)
    }

    private func appendJSONL(_ capture: Capture) {
        guard let data = try? JSONSerialization.data(withJSONObject: capture.jsonlDict, options: [.sortedKeys]) else { return }
        var line = data
        line.append(0x0A)
        if FileManager.default.fileExists(atPath: args.outputURL.path),
           let handle = try? FileHandle(forWritingTo: args.outputURL) {
            defer { try? handle.close() }
            try? handle.seekToEnd()
            try? handle.write(contentsOf: line)
        } else {
            try? line.write(to: args.outputURL)
        }
    }

    // MARK: - BrowserPaginateHost

    func paginateLog(_ message: String) {
        log(message)
    }

    func paginateEvalJS(_ js: String, completion: @MainActor @Sendable @escaping (Any?, Error?) -> Void) {
        webView.evaluateJavaScript(js) { result, err in
            // WKWebView's completion is called on main thread; assume the
            // isolation so the @MainActor @Sendable closure can run.
            MainActor.assumeIsolated { completion(result, err) }
        }
    }

    func paginateCountCaptures(matching pattern: String) -> Int {
        captures.reduce(0) { acc, c in
            c.responseUrl.contains(pattern) ? acc + 1 : acc
        }
    }

    // MARK: - Logging

    fileprivate func log(_ s: String) {
        if let data = "[probe] \(s)\n".data(using: .utf8) {
            FileHandle.standardError.write(data)
        }
    }
}

// MARK: - Main

let args = parseArgs()
let app = NSApplication.shared
app.setActivationPolicy(.regular)
let probe = Probe(args: args)
probe.run()
app.activate(ignoringOtherApps: true)
app.run()
