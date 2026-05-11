import Testing
import Foundation
@testable import Forage

// MARK: - Filter / dedup / cap logic

@Test
func unhandledAffordancesKeepsPaginationKeywordsAndStripsHandled() {
    let items: [AffordanceItem] = [
        AffordanceItem(selector: "button.cart", text: "Add to cart"),
        AffordanceItem(selector: "button.load-more", text: "View more"),
        AffordanceItem(selector: "button.show-more", text: "Show more"),
        AffordanceItem(selector: "button.filter", text: "Filter"),
        AffordanceItem(selector: "a.next", text: "Next →"),
    ]
    let out = BrowserEngine.unhandledAffordances(
        items: items,
        additionalHandledLabels: ["View more"]
    )
    #expect(out == ["Next → (a.next)"])
}

@Test
func unhandledAffordancesEmptyDumpReturnsEmpty() {
    let out = BrowserEngine.unhandledAffordances(items: [], additionalHandledLabels: [])
    #expect(out.isEmpty)
}

@Test
func unhandledAffordancesCapsAt50() {
    var items: [AffordanceItem] = []
    for i in 0..<100 {
        items.append(AffordanceItem(selector: "button.b\(i)", text: "Next page \(i)"))
    }
    let out = BrowserEngine.unhandledAffordances(items: items, additionalHandledLabels: [])
    #expect(out.count == 50)
    #expect(out.first == "Next page 0 (button.b0)")
    #expect(out.last == "Next page 49 (button.b49)")
}

@Test
func unhandledAffordancesDeDupsRepeatedEntries() {
    let items: [AffordanceItem] = [
        AffordanceItem(selector: "button.a", text: "View more"),
        AffordanceItem(selector: "button.a", text: "View more"),
        AffordanceItem(selector: "button.a", text: "View more"),
    ]
    // "View more" is in engineClickedLabels → would be filtered as handled by
    // the engine. Use a label that's pagination-shaped but NOT in the
    // engine's built-in click list. "More results" works.
    let items2: [AffordanceItem] = [
        AffordanceItem(selector: "button.x", text: "More results"),
        AffordanceItem(selector: "button.x", text: "More results"),
    ]
    let out = BrowserEngine.unhandledAffordances(items: items2, additionalHandledLabels: [])
    #expect(out == ["More results (button.x)"])

    // And a sanity check that view-more-style buttons get stripped because
    // the engine claims them.
    let stripped = BrowserEngine.unhandledAffordances(items: items, additionalHandledLabels: [])
    #expect(stripped.isEmpty)
}

@Test
func unhandledAffordancesOmitsSelectorWhenAbsent() {
    let items: [AffordanceItem] = [
        AffordanceItem(selector: nil, text: "Older"),
        AffordanceItem(selector: "", text: "Next page"),
    ]
    let out = BrowserEngine.unhandledAffordances(items: items, additionalHandledLabels: [])
    #expect(out.sorted() == ["Next page", "Older"])
}

@Test
func unhandledAffordancesSubtractsRecipeWarmupClicks() {
    let items: [AffordanceItem] = [
        AffordanceItem(selector: "button.a", text: "More results"),
        AffordanceItem(selector: "button.b", text: "Older"),
    ]
    let out = BrowserEngine.unhandledAffordances(
        items: items,
        additionalHandledLabels: ["More results"]
    )
    #expect(out == ["Older (button.b)"])
}

@Test
func unhandledAffordancesIsCaseInsensitive() {
    let items: [AffordanceItem] = [
        AffordanceItem(selector: "button.x", text: "VIEW MORE PRODUCTS"),
        AffordanceItem(selector: "button.y", text: "show more"),
    ]
    let out = BrowserEngine.unhandledAffordances(items: items, additionalHandledLabels: [])
    // "VIEW MORE PRODUCTS" contains the substring "view more" but doesn't
    // exact-match an engine-clicked label, so it surfaces.
    // "show more" exact-matches engineClickedLabels and is stripped.
    #expect(out == ["VIEW MORE PRODUCTS (button.x)"])
}

@Test
func unhandledAffordancesSkipsNonPaginationButtons() {
    let items: [AffordanceItem] = [
        AffordanceItem(selector: "button.cart", text: "Add to cart"),
        AffordanceItem(selector: "button.checkout", text: "Checkout"),
        AffordanceItem(selector: "button.filter", text: "Filter products"),
    ]
    let out = BrowserEngine.unhandledAffordances(items: items, additionalHandledLabels: [])
    #expect(out.isEmpty)
}

// MARK: - JS-result parsing

@Test
func parseUnhandledAffordancesHandlesNilJSResult() {
    let out = BrowserEngine.parseUnhandledAffordances(jsResult: nil, additionalHandledLabels: [])
    #expect(out.isEmpty)
}

@Test
func parseUnhandledAffordancesHandlesNonStringJSResult() {
    let out = BrowserEngine.parseUnhandledAffordances(jsResult: 42, additionalHandledLabels: [])
    #expect(out.isEmpty)
}

@Test
func parseUnhandledAffordancesHandlesGarbledJSON() {
    let out = BrowserEngine.parseUnhandledAffordances(jsResult: "{not json", additionalHandledLabels: [])
    #expect(out.isEmpty)
}

@Test
func parseUnhandledAffordancesDecodesRealDumpShape() {
    let payload = #"""
    {
        "buttons": [
            {"selector": "button.load-more", "text": "View more"},
            {"selector": "button.more", "text": "More results"}
        ],
        "links": [
            {"selector": "a.next", "text": "Next →"}
        ],
        "roleButtons": [
            {"selector": "div.older", "text": "Older posts"}
        ],
        "scrollables": [
            {"selector": "div.list", "scrollHeight": 4000, "clientHeight": 600}
        ],
        "inputs": []
    }
    """#
    let out = BrowserEngine.parseUnhandledAffordances(
        jsResult: payload,
        additionalHandledLabels: []
    )
    // "View more" is in engineClickedLabels → stripped.
    // The rest match pagination keywords and aren't claimed → surface.
    #expect(out.sorted() == [
        "More results (button.more)",
        "Next → (a.next)",
        "Older posts (div.older)",
    ])
}
