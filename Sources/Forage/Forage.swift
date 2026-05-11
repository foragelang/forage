// Forage — declarative scraping platform.
//
// Public API surface so far:
//
// Engine primitives (browser):
//   - Capture                  : record of one fetch / XHR exchange observed
//                                by the injected JS wrapper
//   - InjectedScripts          : the JS source strings hosts inject into
//                                WKWebView (capture wrapper, age-gate fill,
//                                modal dismiss, scroll+click-load-more,
//                                affordances dump, replay-fork, click-button)
//   - BrowserPaginate          : the engine primitive behind
//                                browserPaginate{ mode: scroll | replay }
//   - BrowserPaginateHost      : protocol the WKWebView host implements
//   - BrowserProgress          : @Observable live progress signal driven by
//                                BrowserEngine (phase, capture/record counts,
//                                current/last observed URL)
//
// Output catalog (domain-agnostic):
//   - Snapshot / ScrapedRecord / TypedValue
//
// Recipe value type:
//   - Recipe / Statement / EngineKind / Expectation
//   - RecipeType / RecipeField / RecipeEnum / InputDecl / FieldType
//   - HTTPGraph / HTTPStep / HTTPRequest / HTTPBody / HTTPBodyKV / BodyValue
//   - Pagination (.pageWithTotal, .untilEmpty, .cursor)
//   - AuthStrategy (.staticHeader, .htmlPrime + HtmlPrimeVar)
//   - PathExpr / Template / TemplatePart
//   - Emission / FieldBinding / ExtractionExpr / TransformCall
//   - BrowserConfig / AgeGateConfig / DismissalConfig /
//     BrowserPaginationConfig / BrowserPaginateUntil / CaptureRule
//
// Runtime (HTTP):
//   - JSONValue                 : wire-format value
//   - Scope                     : variable resolution
//   - PathResolver              : evaluate PathExpr against scope
//   - TemplateRenderer          : render Template with scope
//   - TransformImpls            : built-in transform vocabulary
//   - ExtractionEvaluator       : evaluate ExtractionExpr → TypedValue
//   - HTTPClient / Transport / URLSessionTransport
//   - HTTPEngine                : run an HTTP-engine Recipe
//   - RecipeRunner              : top-level entry point
//   - HTTPProgress              : @Observable live progress signal driven by
//                                 HTTPEngine (phase, requests-sent,
//                                 records-emitted, current URL) — sibling of
//                                 BrowserProgress
//
// Coming next:
//   - Parser (Phase C): .forage text → Recipe
//   - Validator + DiagnosticReport + fixture harness (Phase D)
//   - BrowserEngine (Phase E)
//
// See ../DESIGN.md for the design plan; ../PLANS.md for the roadmap;
// ../README.md for the broad picture.

public enum Forage {
    public static let version = "0.0.3"
}
