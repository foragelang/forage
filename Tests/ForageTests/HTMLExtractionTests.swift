import Testing
import Foundation
@testable import Forage

/// Tests for the HTML/DOM extraction primitive: `JSONValue.node`,
/// `parseHtml`, `parseJson`, `select`, `text`, `attr`, `html`,
/// `innerHtml`, `first`, plus the `for $x in <ExtractionExpr>` grammar
/// extension that lets pipelines drive iteration.
struct HTMLExtractionTests {

    private let transforms = TransformImpls()

    private func parse(_ html: String) -> JSONValue {
        try! transforms.apply("parseHtml", value: .string(html), args: [])
    }

    // MARK: - parseHtml

    @Test func parseHtmlProducesNode() {
        let result = parse("<p>hello</p>")
        guard case .node = result else {
            Issue.record("expected .node, got \(result)")
            return
        }
    }

    @Test func parseHtmlPassesNonStringThrough() throws {
        let result = try transforms.apply("parseHtml", value: .int(42), args: [])
        #expect(result == .int(42))
    }

    // MARK: - parseJson

    @Test func parseJsonRoundTripsObject() throws {
        // Avoid 0/1 ints: JSONValue's NSNumber-to-Bool coercion treats
        // those as `.bool` (pre-existing, documented in JSONValue.swift).
        let result = try transforms.apply(
            "parseJson",
            value: .string("{\"x\": 42, \"y\": [10, 20]}"),
            args: []
        )
        guard case .object(let o) = result else {
            Issue.record("expected .object, got \(result)")
            return
        }
        #expect(o["x"] == .int(42))
        if case .array(let xs) = o["y"] {
            #expect(xs == [.int(10), .int(20)])
        } else {
            Issue.record("expected .y to be array")
        }
    }

    @Test func parseJsonReturnsNullOnMalformed() throws {
        let result = try transforms.apply("parseJson", value: .string("not json"), args: [])
        #expect(result == .null)
    }

    // MARK: - select

    @Test func selectReturnsMatchingNodesAsArray() throws {
        let doc = parse("""
            <ul>
              <li class="a">one</li>
              <li class="a">two</li>
              <li class="b">three</li>
            </ul>
            """)
        let result = try transforms.apply("select", value: doc, args: [.string("li.a")])
        guard case .array(let xs) = result else {
            Issue.record("expected .array, got \(result)")
            return
        }
        #expect(xs.count == 2)
        for x in xs {
            guard case .node = x else {
                Issue.record("expected .node in array, got \(x)")
                return
            }
        }
    }

    @Test func selectOnNonNodeReturnsEmptyArray() throws {
        let result = try transforms.apply("select", value: .string("not a node"), args: [.string("li")])
        #expect(result == .array([]))
    }

    // MARK: - text

    @Test func textExtractsTextFromNode() throws {
        let doc = parse("<p>hello <b>world</b></p>")
        let p = try transforms.apply("select", value: doc, args: [.string("p")])
        let text = try transforms.apply("text", value: p, args: [])
        #expect(text == .string("hello world"))
    }

    @Test func textAutoFlattensSingleElementArray() throws {
        // jQuery convention: `select(...) | text` works without explicit `first`.
        let doc = parse("<h1>Title</h1>")
        let pipeline = try transforms.apply("select", value: doc, args: [.string("h1")])
        let result = try transforms.apply("text", value: pipeline, args: [])
        #expect(result == .string("Title"))
    }

    @Test func textOnEmptyArrayReturnsNull() throws {
        let result = try transforms.apply("text", value: .array([]), args: [])
        #expect(result == .null)
    }

    // MARK: - attr

    @Test func attrReturnsAttributeValue() throws {
        let doc = parse("<a href=\"/x\">link</a>")
        let a = try transforms.apply("select", value: doc, args: [.string("a")])
        let href = try transforms.apply("attr", value: a, args: [.string("href")])
        #expect(href == .string("/x"))
    }

    @Test func attrReturnsNullForMissingAttribute() throws {
        let doc = parse("<a>link</a>")
        let a = try transforms.apply("select", value: doc, args: [.string("a")])
        let href = try transforms.apply("attr", value: a, args: [.string("href")])
        #expect(href == .null)
    }

    // MARK: - first

    @Test func firstReturnsHeadOfArray() throws {
        let result = try transforms.apply("first", value: .array([.int(1), .int(2)]), args: [])
        #expect(result == .int(1))
    }

    @Test func firstOnEmptyArrayReturnsNull() throws {
        let result = try transforms.apply("first", value: .array([]), args: [])
        #expect(result == .null)
    }

    // MARK: - End-to-end via recipe parser + runtime

    @Test func recipeExtractsListingsFromHTMLViaForLoopOverPipeline() async throws {
        let source = """
            recipe "html-listings" {
                engine http

                type Item {
                    title: String
                    url:   String?
                }

                input page: String

                step fetch {
                    method "GET"
                    url    "{$input.page}"
                }

                for $li in $fetch | parseHtml | select("li.story") {
                    emit Item {
                        title ← $li | select("a") | text
                        url   ← $li | select("a") | attr("href")
                    }
                }
            }
            """
        let recipe = try Parser.parse(source: source)
        let issues = Validator.validate(recipe)
        #expect(!issues.hasErrors)

        // Synthetic HTTP transport that returns a small HTML page.
        let html = """
            <ul>
              <li class="story"><a href="/a">Story A</a></li>
              <li class="story"><a href="/b">Story B</a></li>
              <li class="other"><a href="/c">Other</a></li>
            </ul>
            """
        let transport = HTMLFixtureTransport(body: html)
        let runner = RecipeRunner(httpClient: HTTPClient(transport: transport))
        let result = try await runner.run(recipe: recipe, inputs: ["page": .string("http://example.com/")])

        #expect(result.report.stallReason == "completed")
        #expect(result.snapshot.records.count == 2)
        #expect(result.snapshot.records[0].fields["title"] == .string("Story A"))
        #expect(result.snapshot.records[0].fields["url"] == .string("/a"))
        #expect(result.snapshot.records[1].fields["title"] == .string("Story B"))
        #expect(result.snapshot.records[1].fields["url"] == .string("/b"))
    }
}

/// Stub transport that responds to any GET with a fixed HTML body
/// (Content-Type: text/html) — exercises the response-body fallback
/// that makes non-JSON responses available as a string for `parseHtml`.
private struct HTMLFixtureTransport: Transport, @unchecked Sendable {
    let body: String
    func send(_ request: URLRequest) async throws -> (Data, HTTPURLResponse) {
        let data = body.data(using: .utf8)!
        let response = HTTPURLResponse(
            url: request.url!,
            statusCode: 200,
            httpVersion: "HTTP/1.1",
            headerFields: ["Content-Type": "text/html; charset=utf-8"]
        )!
        return (data, response)
    }
}
