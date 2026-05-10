// Forage — declarative scraping platform.
//
// Public API surface so far:
//   - Capture                  : record of one fetch / XHR exchange observed
//                                by the injected JS wrapper
//   - InjectedScripts          : the JS source strings hosts inject into
//                                WKWebView (capture wrapper, age-gate fill,
//                                modal dismiss, scroll+click-load-more,
//                                affordances dump, replay-fork, click-button)
//   - BrowserPaginate          : the engine primitive behind
//                                browserPaginate{ mode: scroll | replay }
//   - BrowserPaginateHost      : protocol the WKWebView host implements so
//                                BrowserPaginate can stay decoupled
//   - Output catalog           : ScrapedDispensary / ScrapedCategory /
//                                ScrapedProduct / ScrapedVariant /
//                                ScrapedPriceObservation / Snapshot — the
//                                fixed types every recipe is required to
//                                output. Downstream consumers ingest
//                                Snapshots and translate to their storage.
//
// Coming next:
//   - Recipe                   : parsed recipe value type
//   - HTTPEngine / BrowserEngine: full runtime that drives a recipe end-to-end
//   - DiagnosticReport         : structured failure artifact
//
// See ../DESIGN.md for the design plan; ../README.md for the broad picture.

public enum Forage {
    public static let version = "0.0.2"
}
