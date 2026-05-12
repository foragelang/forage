//! Forage browser engine: drives a `wry` webview (WKWebView on Mac, WebView2
//! on Windows, WebKitGTK on Linux), intercepts fetch/XHR via injected JS,
//! handles age-gate auto-fill, scroll/button/url pagination, captures.match
//! and captures.document rules, and the M10 interactive-bootstrap flow.
//!
//! Filled in during R4.
