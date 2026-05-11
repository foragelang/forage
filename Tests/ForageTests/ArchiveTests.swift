import Testing
import Foundation
@testable import Forage

// MARK: - Fixtures

private func sampleSnapshot(observedAt: Date = Date(timeIntervalSince1970: 1_715_000_000)) -> Snapshot {
    Snapshot(
        records: [
            ScrapedRecord(typeName: "Item", fields: ["id": .string("a"), "price": .double(9.99)]),
            ScrapedRecord(typeName: "Item", fields: ["id": .string("b"), "price": .double(14.50)]),
        ],
        observedAt: observedAt
    )
}

private func sampleReport() -> DiagnosticReport {
    DiagnosticReport(
        stallReason: "settled",
        unmatchedCaptures: [
            UnmatchedCapture(url: "https://x.test/foo", method: "GET", status: 200, bodyBytes: 42),
            UnmatchedCapture(url: "https://x.test/bar", method: "POST", status: 204, bodyBytes: 0),
        ],
        unfiredRules: ["api/missing"],
        unmetExpectations: ["records.where(typeName == \"Item\").count >= 100 (got 2)"],
        unhandledAffordances: ["Load more"]
    )
}

private func sampleCaptures(count: Int = 3) -> [Capture] {
    (0..<count).map { i in
        Capture(
            timestamp: Date(timeIntervalSince1970: 1_715_000_000 + Double(i)),
            kind: .fetch,
            method: "GET",
            requestUrl: "https://x.test/req/\(i)",
            responseUrl: "https://x.test/resp/\(i)",
            requestBody: "",
            status: 200,
            bodyLength: 5,
            body: "hello"
        )
    }
}

private func sampleMeta(observedAt: Date = Date(timeIntervalSince1970: 1_715_000_000)) -> ArchiveMeta {
    ArchiveMeta(
        recipeName: "jane",
        inputs: [
            "host": .string("example.iheartjane.com"),
            "menuId": .int(1234),
            "flags": .array([.string("a"), .string("b")]),
            "nested": .object(["k": .bool(true)]),
        ],
        runtimeSeconds: 12.5,
        observedAt: observedAt
    )
}

private func makeTempDir() throws -> URL {
    let url = FileManager.default.temporaryDirectory
        .appendingPathComponent("forage-archive-tests", isDirectory: true)
        .appendingPathComponent(UUID().uuidString, isDirectory: true)
    try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
    return url
}

// MARK: - Round-trip

@Test
func archiveRoundTripsAllArtifacts() throws {
    // ISO8601 strategy truncates sub-second precision; build the fixture with
    // a whole-second date so round-trip equality holds.
    let observedAt = Date(timeIntervalSince1970: 1_715_000_000)
    let snapshot = sampleSnapshot(observedAt: observedAt)
    let report = sampleReport()
    let captures = sampleCaptures()
    let meta = sampleMeta(observedAt: observedAt)
    let root = try makeTempDir()

    let handle = try Archive.write(
        root: root,
        slug: "jane-curio",
        snapshot: snapshot,
        report: report,
        captures: captures,
        meta: meta
    )

    #expect(handle.slug == "jane-curio")
    #expect(handle.observedAt == observedAt)
    #expect(handle.directory.lastPathComponent == "2024-05-06T12-53-20Z")

    let (rSnap, rReport, rCaptures, rMeta) = try Archive.read(handle)
    #expect(rSnap == snapshot)
    #expect(rReport == report)
    #expect(rCaptures == captures)
    #expect(rMeta == meta)
}

// MARK: - Captures variants

@Test
func archiveOmitsCapturesFileWhenNil() throws {
    let observedAt = Date(timeIntervalSince1970: 1_715_000_000)
    let root = try makeTempDir()
    let handle = try Archive.write(
        root: root,
        slug: "no-caps",
        snapshot: sampleSnapshot(observedAt: observedAt),
        report: sampleReport(),
        captures: nil,
        meta: sampleMeta(observedAt: observedAt)
    )

    let capturesFile = handle.directory.appendingPathComponent("captures.jsonl")
    #expect(!FileManager.default.fileExists(atPath: capturesFile.path))

    let (_, _, rCaptures, _) = try Archive.read(handle)
    #expect(rCaptures == nil)
}

@Test
func archiveOmitsCapturesFileWhenEmpty() throws {
    let observedAt = Date(timeIntervalSince1970: 1_715_000_000)
    let root = try makeTempDir()
    let handle = try Archive.write(
        root: root,
        slug: "empty-caps",
        snapshot: sampleSnapshot(observedAt: observedAt),
        report: sampleReport(),
        captures: [],
        meta: sampleMeta(observedAt: observedAt)
    )

    let capturesFile = handle.directory.appendingPathComponent("captures.jsonl")
    #expect(!FileManager.default.fileExists(atPath: capturesFile.path))

    let (_, _, rCaptures, _) = try Archive.read(handle)
    #expect(rCaptures == nil)
}

// MARK: - JSONL shape

@Test
func capturesJSONLIsOneObjectPerLine() throws {
    let observedAt = Date(timeIntervalSince1970: 1_715_000_000)
    let captures = sampleCaptures(count: 4)
    let root = try makeTempDir()
    let handle = try Archive.write(
        root: root,
        slug: "jsonl-shape",
        snapshot: sampleSnapshot(observedAt: observedAt),
        report: sampleReport(),
        captures: captures,
        meta: sampleMeta(observedAt: observedAt)
    )

    let url = handle.directory.appendingPathComponent("captures.jsonl")
    let raw = try String(contentsOf: url, encoding: .utf8)
    // 4 captures → 4 lines + trailing newline. `split` on `\n` with
    // omittingEmptySubsequences drops the trailing empty piece.
    let lines = raw.split(separator: "\n", omittingEmptySubsequences: true)
    #expect(lines.count == 4)

    let decoder = JSONDecoder()
    decoder.dateDecodingStrategy = .iso8601
    for (i, line) in lines.enumerated() {
        let data = Data(line.utf8)
        let cap = try decoder.decode(Capture.self, from: data)
        #expect(cap.responseUrl == captures[i].responseUrl)
    }
}

// MARK: - List ordering

@Test
func listReturnsRunsNewestFirst() throws {
    let root = try makeTempDir()
    let slug = "ordered"
    let dates = [
        Date(timeIntervalSince1970: 1_710_000_000),
        Date(timeIntervalSince1970: 1_715_000_000),
        Date(timeIntervalSince1970: 1_720_000_000),
    ]
    for d in dates {
        _ = try Archive.write(
            root: root, slug: slug,
            snapshot: sampleSnapshot(observedAt: d),
            report: sampleReport(),
            captures: nil,
            meta: sampleMeta(observedAt: d)
        )
    }

    let handles = Archive.list(root: root, slug: slug)
    #expect(handles.count == 3)
    #expect(handles.map(\.observedAt) == dates.reversed())
}

// MARK: - Atomic write

@Test
func listSkipsInProgressWritingDirectories() throws {
    let root = try makeTempDir()
    let slug = "atomic"
    let observedAt = Date(timeIntervalSince1970: 1_715_000_000)

    let handle = try Archive.write(
        root: root, slug: slug,
        snapshot: sampleSnapshot(observedAt: observedAt),
        report: sampleReport(),
        captures: nil,
        meta: sampleMeta(observedAt: observedAt)
    )

    let crashedDir = root
        .appendingPathComponent(slug, isDirectory: true)
        .appendingPathComponent("2025-01-01T00-00-00Z.writing", isDirectory: true)
    try FileManager.default.createDirectory(at: crashedDir, withIntermediateDirectories: true)
    try Data("partial".utf8).write(to: crashedDir.appendingPathComponent("snapshot.json"))

    let handles = Archive.list(root: root, slug: slug)
    #expect(handles.count == 1)
    #expect(handles.first?.directory.lastPathComponent == handle.directory.lastPathComponent)
}

@Test
func writeLeavesNoTracesWhenDestinationExists() throws {
    let root = try makeTempDir()
    let slug = "collide"
    let observedAt = Date(timeIntervalSince1970: 1_715_000_000)

    _ = try Archive.write(
        root: root, slug: slug,
        snapshot: sampleSnapshot(observedAt: observedAt),
        report: sampleReport(),
        captures: nil,
        meta: sampleMeta(observedAt: observedAt)
    )

    #expect(throws: ArchiveError.self) {
        _ = try Archive.write(
            root: root, slug: slug,
            snapshot: sampleSnapshot(observedAt: observedAt),
            report: sampleReport(),
            captures: nil,
            meta: sampleMeta(observedAt: observedAt)
        )
    }

    let slugDir = root.appendingPathComponent(slug, isDirectory: true)
    let entries = try FileManager.default.contentsOfDirectory(at: slugDir, includingPropertiesForKeys: nil)
    let stagingLeftovers = entries.filter { $0.lastPathComponent.hasSuffix(".writing") }
    #expect(stagingLeftovers.isEmpty)
}

// MARK: - List robustness

@Test
func listForMissingSlugIsEmpty() throws {
    let root = try makeTempDir()
    _ = try Archive.write(
        root: root, slug: "present",
        snapshot: sampleSnapshot(),
        report: sampleReport(),
        captures: nil,
        meta: sampleMeta()
    )
    #expect(Archive.list(root: root, slug: "missing-slug").isEmpty)
}

@Test
func listForMissingRootIsEmpty() {
    let nonexistent = FileManager.default.temporaryDirectory
        .appendingPathComponent("forage-archive-does-not-exist-\(UUID().uuidString)", isDirectory: true)
    #expect(Archive.list(root: nonexistent, slug: "anything").isEmpty)
}

@Test
func readMissingHandleThrows() {
    let phantom = ArchiveRunHandle(
        slug: "ghost",
        observedAt: Date(),
        directory: FileManager.default.temporaryDirectory
            .appendingPathComponent("forage-archive-phantom-\(UUID().uuidString)", isDirectory: true)
    )
    #expect(throws: (any Error).self) {
        _ = try Archive.read(phantom)
    }
}

// MARK: - Value-type conformances

@Test
func archiveMetaIsHashableAndCodable() throws {
    let meta = sampleMeta()
    let dup = sampleMeta()
    #expect(meta == dup)
    #expect(meta.hashValue == dup.hashValue)

    let encoder = JSONEncoder()
    encoder.dateEncodingStrategy = .iso8601
    let data = try encoder.encode(meta)
    let decoder = JSONDecoder()
    decoder.dateDecodingStrategy = .iso8601
    let decoded = try decoder.decode(ArchiveMeta.self, from: data)
    #expect(decoded == meta)
}

@Test
func archiveRunHandleIsHashable() {
    let dir = URL(fileURLWithPath: "/tmp/whatever")
    let a = ArchiveRunHandle(slug: "x", observedAt: Date(timeIntervalSince1970: 0), directory: dir)
    let b = ArchiveRunHandle(slug: "x", observedAt: Date(timeIntervalSince1970: 0), directory: dir)
    #expect(a == b)
    #expect(a.hashValue == b.hashValue)

    let c = ArchiveRunHandle(slug: "y", observedAt: Date(timeIntervalSince1970: 0), directory: dir)
    #expect(a != c)
}

// MARK: - Sendable conformance smoke test

@Test
func archiveTypesAreSendable() async {
    let meta = sampleMeta()
    let handle = ArchiveRunHandle(
        slug: "s", observedAt: Date(timeIntervalSince1970: 0),
        directory: URL(fileURLWithPath: "/tmp/x")
    )
    // Crossing an async boundary forces a Sendable check at compile time.
    await Task.detached {
        _ = meta
        _ = handle
    }.value
}
