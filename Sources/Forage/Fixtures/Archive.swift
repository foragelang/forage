import Foundation

/// Per-run metadata persisted alongside the snapshot and diagnostic so a
/// later consumer (replayer, inspector, diff tool) can reconstruct what
/// invocation produced the artifacts. `recipeName` and `inputs` identify
/// the call; `runtimeSeconds` and `observedAt` describe the execution.
public struct ArchiveMeta: Sendable, Hashable, Codable {
    public let recipeName: String
    public let inputs: [String: JSONValue]
    public let runtimeSeconds: Double
    public let observedAt: Date

    public init(
        recipeName: String,
        inputs: [String: JSONValue],
        runtimeSeconds: Double,
        observedAt: Date
    ) {
        self.recipeName = recipeName
        self.inputs = inputs
        self.runtimeSeconds = runtimeSeconds
        self.observedAt = observedAt
    }
}

/// Reference to a single on-disk run within an archive root. Returned by
/// `Archive.write` and `Archive.list`; consumed by `Archive.read`. The
/// directory name is `observedAt` rendered as filesystem-safe ISO8601-Z;
/// `slug` is the scraper / recipe identifier the caller supplied to
/// `write`.
public struct ArchiveRunHandle: Sendable, Hashable {
    public let slug: String
    public let observedAt: Date
    public let directory: URL

    public init(slug: String, observedAt: Date, directory: URL) {
        self.slug = slug
        self.observedAt = observedAt
        self.directory = directory
    }
}

/// Durable on-disk format for a single scrape run.
///
/// Layout (caller supplies `root`):
/// ```
/// <root>/<slug>/<ISO8601-Z>/
///     snapshot.json
///     diagnostic.json
///     captures.jsonl     # browser-engine only; omitted when nil or empty
///     meta.json
/// ```
///
/// The `<ISO8601-Z>` directory is filesystem-safe: colons in the timestamp
/// are replaced with dashes (e.g. `2026-05-10T15-22-03Z`). The
/// substitution preserves lexical ordering, so `list` can sort directory
/// names directly and return newest-first.
///
/// `captures.jsonl` is one JSON object per line — the same shape
/// `BrowserReplayer` reads in Phase 7. When the caller passes `nil` or an
/// empty array for `captures`, the file is not written at all; `read`
/// returns `nil` in that case.
public enum Archive {
    public static func write(
        root: URL,
        slug: String,
        snapshot: Snapshot,
        report: DiagnosticReport,
        captures: [Capture]?,
        meta: ArchiveMeta
    ) throws -> ArchiveRunHandle {
        let observedAt = meta.observedAt
        let dirName = filesystemISO8601(observedAt)
        let runDir = root
            .appendingPathComponent(slug, isDirectory: true)
            .appendingPathComponent(dirName, isDirectory: true)

        try FileManager.default.createDirectory(
            at: runDir, withIntermediateDirectories: true
        )

        let snapshotData = try SnapshotIO.encode(snapshot, pretty: true)
        try snapshotData.write(to: runDir.appendingPathComponent("snapshot.json"))

        let diagnosticData = try jsonEncoder().encode(report)
        try diagnosticData.write(to: runDir.appendingPathComponent("diagnostic.json"))

        let metaData = try jsonEncoder().encode(meta)
        try metaData.write(to: runDir.appendingPathComponent("meta.json"))

        if let caps = captures, !caps.isEmpty {
            let jsonl = try encodeCapturesJSONL(caps)
            try jsonl.write(to: runDir.appendingPathComponent("captures.jsonl"))
        }

        return ArchiveRunHandle(slug: slug, observedAt: observedAt, directory: runDir)
    }

    public static func list(root: URL, slug: String) -> [ArchiveRunHandle] {
        let slugDir = root.appendingPathComponent(slug, isDirectory: true)
        let fm = FileManager.default
        var isDir: ObjCBool = false
        guard fm.fileExists(atPath: slugDir.path, isDirectory: &isDir), isDir.boolValue else {
            return []
        }
        let entries: [URL]
        do {
            entries = try fm.contentsOfDirectory(
                at: slugDir,
                includingPropertiesForKeys: nil,
                options: [.skipsHiddenFiles]
            )
        } catch {
            return []
        }
        return entries
            .compactMap { url -> ArchiveRunHandle? in
                guard let observedAt = parseFilesystemISO8601(url.lastPathComponent) else {
                    return nil
                }
                return ArchiveRunHandle(
                    slug: slug, observedAt: observedAt, directory: url
                )
            }
            .sorted { $0.directory.lastPathComponent > $1.directory.lastPathComponent }
    }

    public static func read(_ handle: ArchiveRunHandle) throws
        -> (snapshot: Snapshot, report: DiagnosticReport,
            captures: [Capture]?, meta: ArchiveMeta)
    {
        let snapshotData = try Data(contentsOf: handle.directory.appendingPathComponent("snapshot.json"))
        let snapshot = try SnapshotIO.decode(snapshotData)

        let diagnosticData = try Data(contentsOf: handle.directory.appendingPathComponent("diagnostic.json"))
        let report = try jsonDecoder().decode(DiagnosticReport.self, from: diagnosticData)

        let metaData = try Data(contentsOf: handle.directory.appendingPathComponent("meta.json"))
        let meta = try jsonDecoder().decode(ArchiveMeta.self, from: metaData)

        let capturesURL = handle.directory.appendingPathComponent("captures.jsonl")
        var captures: [Capture]?
        if FileManager.default.fileExists(atPath: capturesURL.path) {
            let data = try Data(contentsOf: capturesURL)
            captures = try decodeCapturesJSONL(data)
        }

        return (snapshot, report, captures, meta)
    }

    // MARK: - Internals

    private static func jsonEncoder() -> JSONEncoder {
        let e = JSONEncoder()
        e.dateEncodingStrategy = .iso8601
        e.outputFormatting = [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes]
        return e
    }

    private static func jsonDecoder() -> JSONDecoder {
        let d = JSONDecoder()
        d.dateDecodingStrategy = .iso8601
        return d
    }

    private static func encodeCapturesJSONL(_ captures: [Capture]) throws -> Data {
        // JSONL: each Capture on its own line, no pretty-printing, sorted
        // keys for deterministic output. Newline-terminated so appending is
        // safe even if a future writer streams rather than dumps in bulk.
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        encoder.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
        var out = Data()
        for capture in captures {
            let line = try encoder.encode(capture)
            out.append(line)
            out.append(0x0A) // \n
        }
        return out
    }

    private static func decodeCapturesJSONL(_ data: Data) throws -> [Capture] {
        guard !data.isEmpty else { return [] }
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        var captures: [Capture] = []
        var lineStart = data.startIndex
        for i in data.indices {
            if data[i] == 0x0A {
                if i > lineStart {
                    let line = data[lineStart..<i]
                    captures.append(try decoder.decode(Capture.self, from: line))
                }
                lineStart = data.index(after: i)
            }
        }
        if lineStart < data.endIndex {
            let tail = data[lineStart..<data.endIndex]
            captures.append(try decoder.decode(Capture.self, from: tail))
        }
        return captures
    }

    private static func filesystemFormatter() -> ISO8601DateFormatter {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime]
        f.timeZone = TimeZone(secondsFromGMT: 0)
        return f
    }

    private static func filesystemISO8601(_ date: Date) -> String {
        // ISO8601 with `:` replaced by `-` so the string is a valid path
        // component on every filesystem we care about. The substitution is
        // unambiguous (ISO8601 never uses `-` outside the date portion in a
        // way that conflicts with the time portion's `:` slots), so we can
        // reverse it for parsing.
        filesystemFormatter().string(from: date).replacingOccurrences(of: ":", with: "-")
    }

    private static func parseFilesystemISO8601(_ s: String) -> Date? {
        // Reverse the `:` → `-` substitution on the time portion only. ISO8601
        // shape: `YYYY-MM-DDTHH-MM-SSZ`. The first 10 chars are the date
        // (`YYYY-MM-DD`) — keep their dashes. Anything after position 10 was
        // a colon in the original.
        guard s.count >= 11 else { return nil }
        let splitIndex = s.index(s.startIndex, offsetBy: 10)
        let datePart = s[..<splitIndex]
        let timePart = s[splitIndex...].replacingOccurrences(of: "-", with: ":")
        return filesystemFormatter().date(from: String(datePart) + timePart)
    }
}
