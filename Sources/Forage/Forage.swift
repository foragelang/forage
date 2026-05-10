// Forage — declarative scraping platform.
//
// This file is a placeholder for the v0 module. The runtime will land here in
// pieces over the next few iterations:
//
//   - HTTPEngine: parses HTTP-recipe portions of a recipe and runs them
//   - BrowserEngine: WKWebView-hosted runtime that captures fetch/XHR + drives
//     the SPA via scroll/click/navigate primitives
//   - Recipe: parsed-recipe value type the engines consume
//   - OutputCatalog: the fixed type catalog recipes target (currently shaped
//     for the weed-prices consumer; lifted once a second consumer surfaces)
//   - DiagnosticReport: the structured failure artifact the engine emits when
//     a recipe stalls below its declared expectations
//
// See ../README.md for the broad picture and ../../weed-prices/notes/scraping-dsl.md
// for the design plan.

public enum Forage {
    public static let version = "0.0.0"
}
