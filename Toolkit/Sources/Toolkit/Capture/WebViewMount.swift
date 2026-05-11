import SwiftUI
import WebKit

/// `NSViewRepresentable` mount for an externally-owned `WKWebView`. We hand
/// it back the same instance built by `CaptureSession`, which preconfigures
/// the script handler / capture-wrapper injection. SwiftUI just hosts it.
struct WebViewMount: NSViewRepresentable {
    let webView: WKWebView

    func makeNSView(context: Context) -> WKWebView { webView }
    func updateNSView(_ nsView: WKWebView, context: Context) {}
}
