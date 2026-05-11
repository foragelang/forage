import Testing
import Foundation
@testable import Forage

/// M9 — `captures.document { … }` extraction. Live browser tests require
/// WKWebView and aren't run here; instead we exercise the replayer path
/// with a synthesized `.document` capture, which is the same code path
/// the live engine takes after settling.
struct DocumentCaptureTests {

    @Test func parserAcceptsCapturesDocumentBlock() throws {
        let source = """
            recipe "doc-cap" {
                engine browser

                type Item {
                    title: String
                    url:   String?
                }

                browser {
                    initialURL: "https://example.com"
                    observe:    "example.com"
                    paginate browserPaginate.scroll {
                        until: noProgressFor(3)
                    }

                    captures.document {
                        for $card in $ | select(".card") {
                            emit Item {
                                title ← $card | select("h3") | text
                                url   ← $card | select("a") | attr("href")
                            }
                        }
                    }
                }

                expect { records.where(typeName == "Item").count >= 1 }
            }
            """
        let recipe = try Parser.parse(source: source)
        let issues = Validator.validate(recipe)
        #expect(!issues.hasErrors)
        #expect(recipe.browser?.documentCapture != nil)
        #expect(recipe.browser?.captures.isEmpty == true)
    }

    @Test func parserRejectsDuplicateDocumentBlock() {
        let source = """
            recipe "dup-doc" {
                engine browser

                type Item { title: String }

                browser {
                    initialURL: "https://example.com"
                    observe:    "example.com"
                    paginate browserPaginate.scroll {
                        until: noProgressFor(3)
                    }
                    captures.document {
                        for $x in $ | select(".a") {
                            emit Item { title ← $x | text }
                        }
                    }
                    captures.document {
                        for $x in $ | select(".b") {
                            emit Item { title ← $x | text }
                        }
                    }
                }
            }
            """
        #expect(throws: (any Error).self) {
            _ = try Parser.parse(source: source)
        }
    }

    @MainActor
    @Test func browserReplayerRoutesDocumentCaptureToDocumentRule() async throws {
        let source = """
            recipe "doc-cap-replay" {
                engine browser

                type Story {
                    title: String
                    url:   String?
                }

                browser {
                    initialURL: "https://example.com/news"
                    observe:    "example.com"
                    paginate browserPaginate.scroll {
                        until: noProgressFor(3)
                    }

                    captures.document {
                        for $story in $ | select("li.story") {
                            emit Story {
                                title ← $story | select("a.title") | text
                                url   ← $story | select("a.title") | attr("href")
                            }
                        }
                    }
                }

                expect { records.where(typeName == "Story").count >= 2 }
            }
            """
        let recipe = try Parser.parse(source: source)
        #expect(!Validator.validate(recipe).hasErrors)

        // Build a synthetic document capture, run it through the
        // replayer-driven BrowserEngine path.
        let html = """
            <ul>
              <li class="story"><a class="title" href="/a">Story A</a></li>
              <li class="story"><a class="title" href="/b">Story B</a></li>
              <li class="other"><a class="title" href="/c">Other</a></li>
            </ul>
            """
        let documentCapture = Capture(
            timestamp: Date(),
            kind: .document,
            method: "GET",
            requestUrl: "https://example.com/news",
            responseUrl: "https://example.com/news",
            requestBody: "",
            status: 200,
            bodyLength: html.utf8.count,
            body: html
        )

        let replayer = BrowserReplayer(captures: [documentCapture])
        let engine = BrowserEngine(
            recipe: recipe,
            inputs: [:],
            visible: false,
            replayer: replayer
        )
        let result = try await engine.run()

        #expect(result.report.stallReason == "settled")
        #expect(result.snapshot.records.count == 2)
        #expect(result.snapshot.records[0].fields["title"] == .string("Story A"))
        #expect(result.snapshot.records[0].fields["url"] == .string("/a"))
        #expect(result.snapshot.records[1].fields["title"] == .string("Story B"))
    }
}
