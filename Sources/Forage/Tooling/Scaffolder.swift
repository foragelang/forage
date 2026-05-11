import Foundation

/// Captures-JSONL → starter `.forage` recipe heuristic synthesizer.
///
/// The output is a starting point — the author hand-edits before
/// shipping. Heuristics aim for "right shape, plausible types," not
/// "ready to run on a live site."
///
/// 1. Group captures by structural URL pattern (lowercase host + path
///    with numeric / hex segments collapsed to `:id`, query stripped).
/// 2. The dominant pattern (most captures) wins; `--host` substring
///    filter narrows candidates first.
/// 3. For the dominant group, decode JSON bodies and find the "biggest
///    array of homogeneous objects" — the array whose elements share the
///    most field names across the most rows.
/// 4. Walk the inferred item shape; per-key type inference picks `String
///    / Int / Double / Bool / [String]` from name patterns plus
///    observed-value sniffing. Nested objects get a TODO comment.
/// 5. Engine selection: JSON content-type on the dominant pattern →
///    `http` + `step + paginate.untilEmpty`. Otherwise → `browser` +
///    `captures.match`.
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
        public let urlSubstring: String
        public let itemsPath: String
        public let typeName: String
        public let fields: [InferredField]
        public let skippedNestedKeys: [String]
        public let engine: EngineChoice
        public let host: String

        public enum EngineChoice: Sendable {
            case http
            case browser
        }
    }

    // MARK: - JSONL parsing

    public static func parseJSONL(_ data: Data) throws -> [Capture] {
        guard !data.isEmpty else { return [] }
        var out: [Capture] = []
        var lineStart = data.startIndex
        for i in data.indices {
            if data[i] == 0x0A {
                if i > lineStart, let cap = decodeLine(data[lineStart..<i]) {
                    out.append(cap)
                }
                lineStart = data.index(after: i)
            }
        }
        if lineStart < data.endIndex, let cap = decodeLine(data[lineStart..<data.endIndex]) {
            out.append(cap)
        }
        return out
    }

    private static func decodeLine(_ slice: Data) -> Capture? {
        guard let obj = try? JSONSerialization.jsonObject(with: slice) as? [String: Any] else { return nil }
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

    /// Strip query and collapse numeric / long-hex path segments to `:id`.
    /// Lowercases the host so subtle case differences across captures
    /// don't fragment a single endpoint into multiple groups.
    /// `https://X.Com/api/products/123?page=2` →
    /// `https://x.com/api/products/:id`.
    public static func canonicalize(_ url: String) -> String {
        guard let parsed = URL(string: url) else { return url }
        let host = (parsed.host ?? "").lowercased()
        let scheme = parsed.scheme?.lowercased() ?? "https"
        let segments = parsed.path.split(separator: "/").map { collapseSegment(String($0)) }
        let path = "/" + segments.joined(separator: "/")
        return "\(scheme)://\(host)\(path)"
    }

    private static func collapseSegment(_ s: String) -> String {
        if !s.isEmpty, s.allSatisfy({ $0.isASCII && $0.isNumber }) { return ":id" }
        if s.count >= 8, s.allSatisfy({ c in
            c.isASCII && (c.isNumber || ("a"..."f").contains(c) || ("A"..."F").contains(c) || c == "-")
        }) { return ":id" }
        return s
    }

    // MARK: - Body shape inference

    struct ArrayFind {
        let path: String
        let array: [Any]
    }

    /// Walk a JSON tree and pick the most-likely "items array."
    /// Scoring: more elements wins; tied length, more *shared* keys wins
    /// (homogeneity is a strong signal that the array is the items list
    /// rather than an incidental tuple of dissimilar things).
    static func findBestArray(in json: Any, currentPath: String = "$") -> ArrayFind? {
        var best: ArrayFind? = arrayCandidate(json, path: currentPath)

        if let arr = json as? [Any] {
            for (i, item) in arr.enumerated() {
                if let nested = findBestArray(in: item, currentPath: "\(currentPath)[\(i)]") {
                    if isBetter(nested, than: best) { best = nested }
                }
            }
        }
        if let obj = json as? [String: Any] {
            for k in obj.keys.sorted() {
                if let nested = findBestArray(in: obj[k]!, currentPath: "\(currentPath).\(k)") {
                    if isBetter(nested, than: best) { best = nested }
                }
            }
        }
        return best
    }

    /// An array qualifies as a *candidate items list* only if every element
    /// is an object and there are ≥2 elements. Mixed arrays and singletons
    /// are too noisy / not informative enough to type from.
    private static func arrayCandidate(_ json: Any, path: String) -> ArrayFind? {
        guard let arr = json as? [Any] else { return nil }
        let objects = arr.compactMap { $0 as? [String: Any] }
        guard objects.count >= 2, objects.count == arr.count else { return nil }
        return ArrayFind(path: path, array: arr)
    }

    private static func isBetter(_ a: ArrayFind, than b: ArrayFind?) -> Bool {
        guard let b else { return true }
        if a.array.count != b.array.count { return a.array.count > b.array.count }
        return sharedKeyScore(a.array) > sharedKeyScore(b.array)
    }

    private static func sharedKeyScore(_ arr: [Any]) -> Int {
        let objs = arr.compactMap { $0 as? [String: Any] }
        guard let first = objs.first else { return 0 }
        var shared = Set(first.keys)
        for obj in objs.dropFirst() { shared.formIntersection(obj.keys) }
        return shared.count
    }

    public struct ItemShape: Sendable {
        public let fields: [InferredField]
        public let skippedNestedKeys: [String]
    }

    /// Build the inferred type from the union of keys across the array's
    /// elements. Keys missing on some elements become optional (`T?`).
    /// Nested-object values get skipped and reported separately so the
    /// renderer can emit a TODO comment instead of inventing a child type.
    public static func buildItemShape(from array: [Any]) -> ItemShape {
        let objects = array.compactMap { $0 as? [String: Any] }
        guard !objects.isEmpty else { return ItemShape(fields: [], skippedNestedKeys: []) }

        var allKeys = Set<String>()
        for obj in objects { allKeys.formUnion(obj.keys) }

        let total = objects.count
        var fields: [InferredField] = []
        var skipped: [String] = []
        for key in allKeys.sorted() {
            let values = objects.compactMap { $0[key] }
            if values.contains(where: { $0 is [String: Any] }) {
                skipped.append(key)
                continue
            }
            let baseType = inferType(forKey: key, values: values)
            let present = values.count
            let optional = present < total || values.contains(where: { $0 is NSNull })
            // Array types don't carry `?` in this DSL — empty array is the absence.
            let resolved = baseType.hasPrefix("[") ? baseType : (optional ? "\(baseType)?" : baseType)
            fields.append(InferredField(name: key, type: resolved))
        }
        return ItemShape(fields: fields, skippedNestedKeys: skipped)
    }

    /// Type inference for a single field. Key-name patterns win when they're
    /// unambiguous; observed-value shape is the fallback. The brief's table:
    ///   String   for `*[Ii]d$`, `name`, `title`, `description`, `slug`,
    ///            `url`, `*[Uu]rl$`, `sku`, `_id$`
    ///   Double   for `price`, `*[Pp]rice$`, numeric values w/ decimals
    ///   Int      for pure integers
    ///   Bool     for keys ending `able`, `is_*`, or `true/false` values
    ///   [String] for arrays of strings
    public static func inferType(forKey key: String, values: [Any]) -> String {
        let lower = key.lowercased()

        // Bool: name signal first (handles "1"/"0"-encoded flags too).
        if lower.hasPrefix("is_") || lower.hasPrefix("has_") || lower.hasSuffix("able") {
            return "Bool"
        }
        // String name signals.
        if ["name", "title", "description", "slug", "url", "sku"].contains(lower) { return "String" }
        if lower.hasSuffix("id") || lower.hasSuffix("_id") { return "String" }
        if lower.hasSuffix("url") { return "String" }
        // Double name signals (price-shaped).
        if lower == "price" || lower.hasSuffix("price") || lower.hasSuffix("_price") {
            return "Double"
        }

        let nonNull = values.filter { !($0 is NSNull) }
        if nonNull.isEmpty { return "String" }

        // Arrays of (presumed string) elements — emit [String]. Per the
        // brief, nested arrays of objects don't get auto-typed; the
        // recipe author handles those by hand.
        if nonNull.allSatisfy({ $0 is [Any] }) {
            return "[String]"
        }

        // Bool values (NSNumber boolean toll-free bridging).
        if nonNull.allSatisfy({ ($0 as? NSNumber).map { CFGetTypeID($0) == CFBooleanGetTypeID() } ?? false }) {
            return "Bool"
        }

        // Numbers — Int if every observation is integral, Double if any decimal.
        if nonNull.allSatisfy({ $0 is NSNumber }) {
            let anyFloat = nonNull.contains { v in
                guard let n = v as? NSNumber, CFGetTypeID(n) != CFBooleanGetTypeID() else { return false }
                return n.stringValue.contains(".") || n.stringValue.contains("e")
            }
            return anyFloat ? "Double" : "Int"
        }

        return "String"
    }

    // MARK: - Top-level scaffold

    public static func inferProductsEndpoint(_ captures: [Capture]) -> Inference? {
        guard !captures.isEmpty else { return nil }

        // Bucket every capture by canonical URL. JSON-content captures are
        // separately tracked so we walk only their bodies for arrays.
        var jsonGroups: [String: [Capture]] = [:]
        var allGroups: [String: [Capture]] = [:]
        for cap in captures {
            let urlForCanon = cap.responseUrl.isEmpty ? cap.requestUrl : cap.responseUrl
            let pat = canonicalize(urlForCanon)
            allGroups[pat, default: []].append(cap)
            if cap.contentType.contains("json") {
                jsonGroups[pat, default: []].append(cap)
            }
        }

        struct Scored {
            let pattern: String
            let allCount: Int
            let bestArrayCount: Int
            let sample: Capture
            let bestPath: String
        }
        var scored: [Scored] = []
        for (pattern, members) in jsonGroups {
            var bestSize = 0
            var bestPath = "$"
            var bestSample: Capture?
            for m in members {
                guard let bodyData = m.body.data(using: .utf8),
                      let json = try? JSONSerialization.jsonObject(with: bodyData),
                      let found = findBestArray(in: json) else { continue }
                if found.array.count > bestSize {
                    bestSize = found.array.count
                    bestPath = found.path
                    bestSample = m
                }
            }
            guard let bestSample else { continue }
            scored.append(Scored(
                pattern: pattern,
                allCount: allGroups[pattern]?.count ?? members.count,
                bestArrayCount: bestSize,
                sample: bestSample,
                bestPath: bestPath
            ))
        }

        scored.sort { a, b in
            if a.allCount != b.allCount { return a.allCount > b.allCount }
            return a.bestArrayCount > b.bestArrayCount
        }
        guard let winner = scored.first else { return nil }

        guard let bodyData = winner.sample.body.data(using: .utf8),
              let json = try? JSONSerialization.jsonObject(with: bodyData),
              let found = findBestArray(in: json) else {
            return nil
        }

        let shape = buildItemShape(from: found.array)
        let host = URL(string: winner.sample.responseUrl)?.host?.lowercased()
            ?? URL(string: winner.sample.requestUrl)?.host?.lowercased()
            ?? "unknown.example"

        // JSON content-type on the dominant pattern → http engine. The
        // alternative is text/html shells or no clear JSON, which means
        // the page renders data client-side and we need a browser.
        let engine: Inference.EngineChoice =
            (jsonGroups[winner.pattern]?.isEmpty == false) ? .http : .browser

        return Inference(
            urlSubstring: pickURLSubstring(winner.pattern),
            itemsPath: found.path,
            typeName: "Product",
            fields: shape.fields,
            skippedNestedKeys: shape.skippedNestedKeys,
            engine: engine,
            host: host
        )
    }

    public static func scaffold(captures: [Capture], hostFilter: String?) -> String {
        let filtered: [Capture]
        if let hostFilter, !hostFilter.isEmpty {
            filtered = captures.filter { cap in
                let host = URL(string: cap.responseUrl)?.host
                    ?? URL(string: cap.requestUrl)?.host
                    ?? ""
                return host.contains(hostFilter)
            }
        } else {
            filtered = captures
        }

        guard let inference = inferProductsEndpoint(filtered) else {
            return emptyStub()
        }

        switch inference.engine {
        case .http: return renderHTTPRecipe(inference)
        case .browser: return renderBrowserRecipe(inference)
        }
    }

    /// Pick the URL-match substring used by `observe` / `urlPattern` /
    /// `urlSubstring`. The recipe matches via `String.contains`, so we
    /// want something distinctive but not so long it breaks on minor
    /// variants (e.g., different query params, CDN suffixes). Host +
    /// first 1-2 non-`:id` segments lands in the right spot in practice.
    private static func pickURLSubstring(_ canonical: String) -> String {
        guard let parsed = URL(string: canonical) else { return canonical }
        let host = (parsed.host ?? "").lowercased()
        let segs = parsed.path.split(separator: "/")
            .filter { $0 != ":id" }
            .prefix(2)
            .joined(separator: "/")
        return segs.isEmpty ? host : "\(host)/\(segs)"
    }

    // MARK: - Rendering

    private static func recipeSlug(_ host: String) -> String {
        let cleaned = host
            .replacingOccurrences(of: ".", with: "-")
            .replacingOccurrences(of: ":", with: "-")
        return cleaned.isEmpty ? "scaffold" : "scaffold-\(cleaned)"
    }

    /// Strip trailing `[*]` from a path. Renderers re-add it inside the
    /// for-loop head.
    private static func stripWildcard(_ path: String) -> String {
        path.hasSuffix("[*]") ? String(path.dropLast(3)) : path
    }

    private static func renderTypeBlock(_ inf: Inference) -> String {
        var lines: [String] = ["    type \(inf.typeName) {"]
        if inf.fields.isEmpty {
            lines.append("        // TODO: no scalar fields inferred from captures")
            lines.append("        id: String")
        } else {
            for f in inf.fields {
                lines.append("        \(f.name): \(f.type)")
            }
        }
        for skipped in inf.skippedNestedKeys {
            lines.append("        // TODO: \(skipped) — nested object, handle by hand")
        }
        lines.append("    }")
        return lines.joined(separator: "\n")
    }

    private static func renderEmitBindings(loopVar: String, fields: [InferredField], indent: String) -> String {
        guard !fields.isEmpty else { return "\(indent)// TODO: no fields inferred" }
        let widest = fields.map(\.name.count).max() ?? 0
        return fields.map { f in
            let padded = f.name.padding(toLength: widest, withPad: " ", startingAt: 0)
            return "\(indent)\(padded) ← $\(loopVar).\(f.name)"
        }.joined(separator: "\n")
    }

    private static func renderBrowserRecipe(_ inf: Inference) -> String {
        let slug = recipeSlug(inf.host)
        let typeBlock = renderTypeBlock(inf)
        let iterPath = "\(stripWildcard(inf.itemsPath))[*]"
        let bindings = renderEmitBindings(loopVar: "item", fields: inf.fields, indent: "                        ")
        return """
        // Scaffolded from captures. Hand-edit before running.
        recipe "\(slug)" {
            engine browser

        \(typeBlock)

            input dispensarySlug: String
            input dispensaryName: String
            input siteOrigin: String

            browser {
                initialURL: "{$input.siteOrigin}/"

                observe: "\(inf.urlSubstring)"

                paginate browserPaginate.scroll {
                    until: noProgressFor(3)
                    maxIterations: 30
                }

                captures.match {
                    urlPattern: "\(inf.urlSubstring)"
                    for $item in \(iterPath) {
                        emit \(inf.typeName) {
        \(bindings)
                        }
                    }
                }
            }

            expect { records.where(typeName == "\(inf.typeName)").count >= 1 }
        }
        """
    }

    /// HTTP scaffold: a single paginated `step`. After pagination, the
    /// step name binds to the flat list of items, so the for-loop is
    /// `for $item in $<step>[*]` regardless of the in-response items
    /// path (which only feeds `paginate.untilEmpty.items`).
    private static func renderHTTPRecipe(_ inf: Inference) -> String {
        let slug = recipeSlug(inf.host)
        let typeBlock = renderTypeBlock(inf)
        let stepName = "items"
        let bindings = renderEmitBindings(loopVar: "item", fields: inf.fields, indent: "            ")
        let urlGuess = "https://\(inf.urlSubstring)"
        return """
        // Scaffolded from captures. Hand-edit before running.
        recipe "\(slug)" {
            engine http

        \(typeBlock)

            input dispensarySlug: String
            input dispensaryName: String
            input siteOrigin: String

            step \(stepName) {
                method "GET"
                url    "\(urlGuess)"
                paginate untilEmpty {
                    items:     \(stripWildcard(inf.itemsPath))
                    pageParam: "page"
                }
            }

            for $item in $\(stepName)[*] {
                emit \(inf.typeName) {
        \(bindings)
                }
            }

            expect { records.where(typeName == "\(inf.typeName)").count >= 1 }
        }
        """
    }

    public static func emptyStub() -> String {
        return """
        // Scaffolded from captures (no JSON items endpoint detected).
        // Hand-edit to point at the right URL pattern and fields.
        recipe "scaffold-empty" {
            engine browser

            type Product {
                id: String
                name: String
            }

            input dispensarySlug: String
            input dispensaryName: String
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
                    for $item in $.products[*] {
                        emit Product {
                            id   ← $item.id
                            name ← $item.name
                        }
                    }
                }
            }

            expect { records.where(typeName == "Product").count >= 1 }
        }
        """
    }
}
