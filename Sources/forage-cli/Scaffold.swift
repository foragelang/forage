import ArgumentParser
import Foundation
import Forage

struct ScaffoldCommand: AsyncParsableCommand {
    static let configuration = CommandConfiguration(
        commandName: "scaffold",
        abstract: "Build a starter .forage recipe from a captures JSONL file."
    )

    @Argument(help: "Path to a captures JSONL produced by `forage capture`.")
    var capturesFile: String

    @Option(name: .customLong("host"),
            help: "Optional host substring filter — only captures whose responseUrl host contains this string are considered.")
    var host: String?

    @Option(name: .customLong("out"), help: "Write recipe to this path instead of stdout.")
    var out: String?

    func run() async throws {
        let url = URL(fileURLWithPath: capturesFile)
        let data = try Data(contentsOf: url)
        let captures = try Scaffolder.parseJSONL(data)
        let recipe = Scaffolder.scaffold(captures: captures, hostFilter: host)

        if let out {
            try recipe.write(toFile: out, atomically: true, encoding: .utf8)
        } else {
            print(recipe)
        }
    }
}

/// Captures-JSONL → starter `.forage` recipe heuristic synthesizer.
///
/// The output is deliberately a 50%-right skeleton — the recipe author
/// hand-edits it. We group captures by URL pattern (numeric IDs stripped,
/// query stripped), pick the most-frequent JSON-content endpoint, find the
/// longest array nested anywhere in the body, infer a `Product` type from
/// the keys on the first array element, and emit a browser-engine recipe
/// (or http if no JS-needed signals).
public enum Scaffolder {
    public struct Capture: Sendable {
        public let method: String
        public let requestUrl: String
        public let responseUrl: String
        public let status: Int
        public let body: String
        public let contentType: String

        public init(method: String, requestUrl: String, responseUrl: String, status: Int, body: String, contentType: String) {
            self.method = method
            self.requestUrl = requestUrl
            self.responseUrl = responseUrl
            self.status = status
            self.body = body
            self.contentType = contentType
        }
    }

    public struct InferredField: Sendable, Hashable {
        public let name: String
        public let type: String
    }

    public struct Inference: Sendable {
        public let urlPattern: String
        public let iterPath: String
        public let fields: [InferredField]
        public let bodyIsJSON: Bool
        public let host: String
    }

    // MARK: - JSONL parsing

    public static func parseJSONL(_ data: Data) throws -> [Capture] {
        guard !data.isEmpty else { return [] }
        var out: [Capture] = []
        var lineStart = data.startIndex
        for i in data.indices {
            if data[i] == 0x0A {
                if i > lineStart {
                    if let cap = decodeLine(data[lineStart..<i]) { out.append(cap) }
                }
                lineStart = data.index(after: i)
            }
        }
        if lineStart < data.endIndex {
            if let cap = decodeLine(data[lineStart..<data.endIndex]) { out.append(cap) }
        }
        return out
    }

    private static func decodeLine(_ slice: Data) -> Capture? {
        guard let obj = try? JSONSerialization.jsonObject(with: slice) as? [String: Any] else { return nil }
        // Content-type isn't always present in the JSONL. Sniff the body for {/[ to decide.
        let body = (obj["body"] as? String) ?? ""
        let ct: String = {
            if let ct = obj["contentType"] as? String { return ct }
            let trimmed = body.trimmingCharacters(in: .whitespacesAndNewlines)
            if trimmed.hasPrefix("{") || trimmed.hasPrefix("[") { return "application/json" }
            if trimmed.hasPrefix("<") { return "text/html" }
            return "application/octet-stream"
        }()
        return Capture(
            method: (obj["method"] as? String) ?? "GET",
            requestUrl: (obj["requestUrl"] as? String) ?? "",
            responseUrl: (obj["responseUrl"] as? String) ?? "",
            status: (obj["status"] as? Int) ?? 0,
            body: body,
            contentType: ct
        )
    }

    // MARK: - URL pattern grouping

    /// Strip query string and numeric path segments to reveal the
    /// *structural* shape of a URL. `https://x.com/api/products/123?page=2`
    /// → `https://x.com/api/products/{id}`.
    public static func canonicalize(_ url: String) -> String {
        guard let parsed = URL(string: url) else { return url }
        let host = parsed.host ?? ""
        let scheme = parsed.scheme ?? "https"
        let segments = parsed.path.split(separator: "/").map { seg -> String in
            let s = String(seg)
            if Int(s) != nil { return "{id}" }
            // UUID-ish segments (32+ hex chars w/ dashes)
            let hexChars = s.unicodeScalars.allSatisfy { c in
                ("0"..."9").contains(c) || ("a"..."f").contains(c) || ("A"..."F").contains(c) || c == "-"
            }
            if hexChars && s.count >= 16 { return "{id}" }
            return s
        }
        let path = "/" + segments.joined(separator: "/")
        return "\(scheme)://\(host)\(path)"
    }

    // MARK: - Body shape inference

    /// Walk a JSON body and find the longest array, returning its dotted
    /// path from the root. `{products: [...]}` → `("$.products", arr)`.
    /// Top-level array → `("$", arr)`. Returns nil if no array found.
    static func findLongestArray(in json: Any, currentPath: String = "$") -> (path: String, array: [Any])? {
        if let arr = json as? [Any] {
            // recurse into elements too, but prefer this level if it's "long enough"
            var best: (path: String, array: [Any])? = (currentPath, arr)
            for (i, item) in arr.enumerated() {
                if let nested = findLongestArray(in: item, currentPath: "\(currentPath)[\(i)]") {
                    if nested.array.count > (best?.array.count ?? 0) {
                        best = nested
                    }
                }
            }
            return best
        }
        if let obj = json as? [String: Any] {
            var best: (path: String, array: [Any])?
            for (k, v) in obj {
                if let nested = findLongestArray(in: v, currentPath: "\(currentPath).\(k)") {
                    if nested.array.count > (best?.array.count ?? 0) {
                        best = nested
                    }
                }
            }
            return best
        }
        return nil
    }

    /// Map a single JSON object element of the products array to a list of
    /// recipe fields. Common product keys are aliased to descriptive names.
    static func inferFields(from element: Any) -> [InferredField] {
        guard let obj = element as? [String: Any] else { return [] }
        var fields: [InferredField] = []
        // Keep field iteration order stable so the scaffold output is deterministic.
        for key in obj.keys.sorted() {
            let value = obj[key]!
            let type = swiftType(for: value)
            let recipeName = recipeFieldName(for: key)
            fields.append(InferredField(name: recipeName, type: type))
        }
        return fields
    }

    static func swiftType(for value: Any) -> String {
        if value is NSNull { return "String?" }
        if let n = value as? NSNumber {
            if CFGetTypeID(n) == CFBooleanGetTypeID() { return "Bool" }
            if n.stringValue.contains(".") || n.stringValue.contains("e") { return "Double" }
            return "Int"
        }
        if value is String { return "String" }
        if value is [Any] { return "[String]" }
        if value is [String: Any] { return "String" }   // nested objects we flatten as opaque
        return "String?"
    }

    /// Map raw key names to descriptive recipe-field names. Stable
    /// alias table — keys not in the table pass through verbatim.
    static func recipeFieldName(for key: String) -> String {
        let lower = key.lowercased()
        switch lower {
        case "id", "objectid", "productid", "sku":
            return "externalId"
        case "name", "title", "displayname":
            return "name"
        case "description", "desc":
            return "description"
        case "price":
            return "price"
        case "image", "imageurl", "thumbnail":
            return "image"
        case "brand":
            return "brand"
        default:
            return key
        }
    }

    // MARK: - Top-level scaffold

    public static func inferProductsEndpoint(_ captures: [Capture]) -> Inference? {
        guard !captures.isEmpty else { return nil }

        // Group JSON-body captures by canonical URL.
        var groups: [String: [Capture]] = [:]
        for cap in captures {
            guard cap.contentType.contains("json") else { continue }
            let pat = canonicalize(cap.responseUrl.isEmpty ? cap.requestUrl : cap.responseUrl)
            groups[pat, default: []].append(cap)
        }

        // Sort groups by:
        // 1) capture count (more = more likely the paginated endpoint),
        // 2) longest-array size in any member (filters out endpoints that
        //    return single objects).
        let scored: [(pattern: String, count: Int, bestArray: Int, sample: Capture, longestPath: String)] =
            groups.compactMap { (pattern, members) in
                var bestSize = 0
                var bestPath = "$"
                var bestSample: Capture?
                for m in members {
                    guard let bodyData = m.body.data(using: .utf8),
                          let json = try? JSONSerialization.jsonObject(with: bodyData) else { continue }
                    if let found = findLongestArray(in: json) {
                        if found.array.count > bestSize {
                            bestSize = found.array.count
                            bestPath = found.path
                            bestSample = m
                        }
                    }
                }
                guard let bestSample else { return nil }
                return (pattern, members.count, bestSize, bestSample, bestPath)
            }
            .sorted { (a, b) in
                if a.count != b.count { return a.count > b.count }
                return a.bestArray > b.bestArray
            }

        guard let winner = scored.first else { return nil }

        // Decode the sample again to grab the first element of the longest array.
        guard let bodyData = winner.sample.body.data(using: .utf8),
              let json = try? JSONSerialization.jsonObject(with: bodyData),
              let found = findLongestArray(in: json),
              let first = found.array.first else {
            return nil
        }

        let fields = inferFields(from: first)
        let host = URL(string: winner.sample.responseUrl)?.host ?? URL(string: winner.sample.requestUrl)?.host ?? "unknown.example"
        return Inference(
            urlPattern: winner.pattern,
            iterPath: winner.longestPath,
            fields: fields,
            bodyIsJSON: true,
            host: host
        )
    }

    public static func scaffold(captures: [Capture], hostFilter: String?) -> String {
        let filtered: [Capture]
        if let hostFilter, !hostFilter.isEmpty {
            filtered = captures.filter {
                $0.responseUrl.contains(hostFilter) || $0.requestUrl.contains(hostFilter)
            }
        } else {
            filtered = captures
        }

        guard let inference = inferProductsEndpoint(filtered) else {
            return emptyStub()
        }

        let urlPattern = pickEngineMatchPattern(inference.urlPattern)
        let slugFromHost = inference.host
            .replacingOccurrences(of: ".", with: "-")
            .replacingOccurrences(of: ":", with: "-")
        let safeHost = slugFromHost.isEmpty ? "scaffold" : slugFromHost
        let typeBlock = renderTypeBlock(fields: inference.fields)
        let emitBlock = renderEmitBlock(fields: inference.fields)
        return """
        // Scaffolded from captures. Hand-edit before running.
        recipe "scaffold-\(safeHost)" {
            engine browser

        \(typeBlock)

            input dispensarySlug: String
            input dispensaryName: String
            input siteOrigin: String

            browser {
                initialURL: "{$input.siteOrigin}/"

                observe: "\(urlPattern)"

                paginate browserPaginate.scroll {
                    until: noProgressFor(3)
                    maxIterations: 30
                }

                captures.match {
                    urlPattern: "\(urlPattern)"
                    for $product in \(inference.iterPath)[*] {
        \(emitBlock)
                    }
                }
            }

            expect { records.where(typeName == "Product").count >= 1 }
        }
        """
    }

    /// Turn the canonical URL into the substring used by `observe` and
    /// `urlPattern`. The recipe matches via `String.contains`, so we want
    /// the most-distinctive substring — host + first path segment is plenty.
    private static func pickEngineMatchPattern(_ canonical: String) -> String {
        guard let parsed = URL(string: canonical) else { return canonical }
        let host = parsed.host ?? ""
        let segments = parsed.path.split(separator: "/").prefix(2).joined(separator: "/")
        if segments.isEmpty { return host }
        return "\(host)/\(segments)"
    }

    private static func renderTypeBlock(fields: [InferredField]) -> String {
        var lines: [String] = ["    type Product {"]
        if fields.isEmpty {
            lines.append("        externalId: String")
            lines.append("        name: String")
        } else {
            // Promote externalId and name to the top, preserve the rest.
            let ordered = orderFields(fields)
            for f in ordered {
                lines.append("        \(f.name): \(f.type)")
            }
        }
        lines.append("    }")
        return lines.joined(separator: "\n")
    }

    private static func renderEmitBlock(fields: [InferredField]) -> String {
        var lines: [String] = ["                        emit Product {"]
        if fields.isEmpty {
            lines.append("                            externalId ← \"\"")
            lines.append("                            name       ← \"\"")
        } else {
            let ordered = orderFields(fields)
            // Pad the field-name column to the widest field for alignment.
            let widest = ordered.map(\.name.count).max() ?? 0
            for f in ordered {
                let padded = f.name.padding(toLength: widest, withPad: " ", startingAt: 0)
                let access = "$product.\(originalKey(for: f.name)) \(coercionPipe(for: f))"
                lines.append("                            \(padded) ← \(access.trimmingCharacters(in: .whitespaces))")
            }
        }
        lines.append("                        }")
        return lines.joined(separator: "\n")
    }

    private static func orderFields(_ fields: [InferredField]) -> [InferredField] {
        let priority = ["externalId", "name", "description", "brand", "price"]
        let priIdx = Dictionary(uniqueKeysWithValues: priority.enumerated().map { ($1, $0) })
        return fields.sorted { a, b in
            let ai = priIdx[a.name] ?? Int.max
            let bi = priIdx[b.name] ?? Int.max
            if ai != bi { return ai < bi }
            return a.name < b.name
        }
    }

    /// Reverse the recipe-field alias to get back the original JSON key.
    /// We don't have a 1:1 map (multiple keys can alias to "externalId"),
    /// so this is a best-effort default that matches the most common.
    private static func originalKey(for recipeName: String) -> String {
        switch recipeName {
        case "externalId": return "id"
        case "name": return "name"
        case "description": return "description"
        case "image": return "image"
        case "brand": return "brand"
        case "price": return "price"
        default: return recipeName
        }
    }

    private static func coercionPipe(for field: InferredField) -> String {
        switch field.name {
        case "externalId":
            return "| toString"
        default:
            return ""
        }
    }

    public static func emptyStub() -> String {
        return """
        // Scaffolded from captures (no JSON product endpoint detected).
        // Hand-edit to point at the right URL pattern and fields.
        recipe "scaffold-empty" {
            engine browser

            type Product {
                externalId: String
                name: String
            }

            input siteOrigin: String

            browser {
                initialURL: "{$input.siteOrigin}/"

                observe: "example.com/products"

                paginate browserPaginate.scroll {
                    until: noProgressFor(3)
                    maxIterations: 30
                }

                captures.match {
                    urlPattern: "example.com/products"
                    for $product in $.products[*] {
                        emit Product {
                            externalId ← $product.id | toString
                            name       ← $product.name
                        }
                    }
                }
            }

            expect { records.where(typeName == "Product").count >= 1 }
        }
        """
    }
}
