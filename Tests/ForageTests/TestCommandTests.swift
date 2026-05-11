import Testing
import Foundation
@testable import Forage

// MARK: - Recipe + fixture authoring helpers

/// Minimal HTTP recipe used by these tests: one type, one step, one
/// emit. Runs against a single fixture line.
private let minimalRecipe = """
recipe "th-min" {
    engine http

    type Item {
        id: String
        name: String
    }

    step items {
        method "GET"
        url    "https://example.com/items"
    }

    for $item in $items.data[*] {
        emit Item {
            id   ← $item.id
            name ← $item.name
        }
    }

    expect { records.where(typeName == "Item").count >= 1 }
}
"""

private let minimalFixture = """
{"method":"GET","url":"example.com/items","status":200,"response":{"data":[{"id":"1","name":"Alpha"},{"id":"2","name":"Beta"}]}}
"""

private func makeRecipeDir() throws -> URL {
    let url = FileManager.default.temporaryDirectory
        .appendingPathComponent("forage-test-harness", isDirectory: true)
        .appendingPathComponent(UUID().uuidString, isDirectory: true)
    try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
    try FileManager.default.createDirectory(
        at: url.appendingPathComponent("fixtures"),
        withIntermediateDirectories: true
    )
    return url
}

private func writeRecipe(
    _ source: String = minimalRecipe,
    fixture: String = minimalFixture,
    expected: Snapshot? = nil
) throws -> URL {
    let dir = try makeRecipeDir()
    try source.write(
        to: dir.appendingPathComponent("recipe.forage"),
        atomically: true,
        encoding: .utf8
    )
    try fixture.write(
        to: dir.appendingPathComponent("fixtures").appendingPathComponent("captures.jsonl"),
        atomically: true,
        encoding: .utf8
    )
    if let expected {
        let data = try SnapshotIO.encode(expected)
        try data.write(to: dir.appendingPathComponent("expected.snapshot.json"))
    }
    return dir
}

private func sampleSnapshot() -> Snapshot {
    Snapshot(
        records: [
            ScrapedRecord(typeName: "Item", fields: ["id": .string("1"), "name": .string("Alpha")]),
            ScrapedRecord(typeName: "Item", fields: ["id": .string("2"), "name": .string("Beta")]),
        ],
        observedAt: Date(timeIntervalSince1970: 1_715_000_000)
    )
}

// MARK: - Happy path

@Test
func testHarnessExitsOkOnSnapshotMatch() async throws {
    let dir = try writeRecipe(expected: sampleSnapshot())
    let outcome = await TestHarness.run(recipeDir: dir.path, update: false)
    if case .ok = outcome.exit {} else {
        Issue.record("expected .ok exit, got \(outcome.exit). stderr=\(outcome.stderr)")
    }
    #expect(outcome.stdout == "ok")
}

@Test
func testHarnessPrintsSnapshotAndHintWhenNoExpectedFile() async throws {
    let dir = try writeRecipe()
    let outcome = await TestHarness.run(recipeDir: dir.path, update: false)
    if case .ok = outcome.exit {} else {
        Issue.record("expected .ok exit, got \(outcome.exit)")
    }
    #expect(outcome.stdout.contains("_typeName"))
    #expect(outcome.stderr.contains("--update"))
}

// MARK: - Diff path

@Test
func testHarnessExitsDiffOnSnapshotMismatch() async throws {
    // Expected snapshot has the wrong second record's name.
    let perturbed = Snapshot(
        records: [
            ScrapedRecord(typeName: "Item", fields: ["id": .string("1"), "name": .string("Alpha")]),
            ScrapedRecord(typeName: "Item", fields: ["id": .string("2"), "name": .string("WRONG")]),
        ],
        observedAt: Date(timeIntervalSince1970: 1_715_000_000)
    )
    let dir = try writeRecipe(expected: perturbed)
    let outcome = await TestHarness.run(recipeDir: dir.path, update: false)
    if case .diff = outcome.exit {} else {
        Issue.record("expected .diff exit, got \(outcome.exit). stderr=\(outcome.stderr)")
    }
    #expect(outcome.stderr.contains("snapshot mismatch"))
    #expect(outcome.stderr.contains("missing"))
    #expect(outcome.stderr.contains("extra"))
    // The actual record (Beta) is "extra"; the perturbed one (WRONG) is "missing".
    #expect(outcome.stderr.contains("WRONG"))
    #expect(outcome.stderr.contains("Beta"))
}

@Test
func snapshotDiffIgnoresRecordOrdering() {
    let a = Snapshot(records: [
        ScrapedRecord(typeName: "Item", fields: ["id": .string("1")]),
        ScrapedRecord(typeName: "Item", fields: ["id": .string("2")]),
    ])
    let b = Snapshot(records: [
        ScrapedRecord(typeName: "Item", fields: ["id": .string("2")]),
        ScrapedRecord(typeName: "Item", fields: ["id": .string("1")]),
    ])
    let diff = SnapshotDiff.compare(expected: a, actual: b)
    #expect(diff.isMatch)
}

// MARK: - --update flow

@Test
func testHarnessUpdateWritesExpectedSnapshot() async throws {
    let dir = try writeRecipe()
    let expectedPath = dir.appendingPathComponent("expected.snapshot.json")
    #expect(!FileManager.default.fileExists(atPath: expectedPath.path))

    let outcome = await TestHarness.run(recipeDir: dir.path, update: true)
    if case .ok = outcome.exit {} else {
        Issue.record("expected .ok exit on --update, got \(outcome.exit). stderr=\(outcome.stderr)")
    }
    #expect(FileManager.default.fileExists(atPath: expectedPath.path))

    // Round-trip: subsequent run without --update should be a clean match.
    let outcome2 = await TestHarness.run(recipeDir: dir.path, update: false)
    if case .ok = outcome2.exit {} else {
        Issue.record("expected .ok on round-trip run, got \(outcome2.exit). stderr=\(outcome2.stderr)")
    }
}

// MARK: - Setup errors

@Test
func testHarnessReportsSetupErrorOnMissingRecipe() async {
    let outcome = await TestHarness.run(
        recipeDir: "/tmp/forage-this-path-does-not-exist-\(UUID().uuidString)",
        update: false
    )
    if case .setupError = outcome.exit {} else {
        Issue.record("expected .setupError, got \(outcome.exit)")
    }
    #expect(outcome.stderr.contains("missing recipe.forage"))
}

@Test
func testHarnessReportsSetupErrorOnParseFailure() async throws {
    let dir = try makeRecipeDir()
    try "this is not a valid forage recipe".write(
        to: dir.appendingPathComponent("recipe.forage"),
        atomically: true,
        encoding: .utf8
    )
    let outcome = await TestHarness.run(recipeDir: dir.path, update: false)
    if case .setupError = outcome.exit {} else {
        Issue.record("expected .setupError, got \(outcome.exit)")
    }
    #expect(outcome.stderr.contains("parse failed") || outcome.stderr.contains("validation failed"))
}

// MARK: - Unmet expectations

@Test
func testHarnessExitsDiffOnUnmetExpectations() async throws {
    // Recipe expects ≥ 100 records but fixture only yields 2.
    let strict = minimalRecipe.replacingOccurrences(
        of: "count >= 1",
        with: "count >= 100"
    )
    let dir = try makeRecipeDir()
    try strict.write(
        to: dir.appendingPathComponent("recipe.forage"),
        atomically: true,
        encoding: .utf8
    )
    try minimalFixture.write(
        to: dir.appendingPathComponent("fixtures").appendingPathComponent("captures.jsonl"),
        atomically: true,
        encoding: .utf8
    )
    // Pin a matching snapshot so the only failure is the expectation.
    let data = try SnapshotIO.encode(sampleSnapshot())
    try data.write(to: dir.appendingPathComponent("expected.snapshot.json"))

    let outcome = await TestHarness.run(recipeDir: dir.path, update: false)
    if case .diff = outcome.exit {} else {
        Issue.record("expected .diff exit, got \(outcome.exit). stderr=\(outcome.stderr)")
    }
    #expect(outcome.stderr.contains("unmet expectations"))
    #expect(outcome.stderr.contains("count >= 100"))
}
