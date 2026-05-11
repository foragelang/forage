import Testing
import Foundation
@testable import Forage

// `Forage.Expectation` (recipe-author `expect { … }`) collides with
// `Testing.Expectation` (Swift Testing's `#expect` machinery). The
// selective `import struct` makes the recipe one win when written
// unqualified inside this file.
import struct Forage.Expectation

// MARK: - Helpers

private func snapshot(_ records: [(String, Int)]) -> Snapshot {
    // Expand `(typeName, count)` pairs into that many minimal ScrapedRecords.
    var out: [ScrapedRecord] = []
    for (typeName, count) in records {
        for _ in 0..<count {
            out.append(ScrapedRecord(typeName: typeName, fields: [:]))
        }
    }
    return Snapshot(records: out, observedAt: Date(timeIntervalSince1970: 0))
}

private func makeExpect(_ typeName: String, _ op: ComparisonOp, _ value: Int) -> Expectation {
    Expectation(.recordCount(typeName: typeName, op: op, value: value))
}

// MARK: - Single expectation

@Test
func singleExpectationSatisfiedReturnsNoFailures() {
    let s = snapshot([("Product", 600)])
    let failures = ExpectationEvaluator.evaluate(
        [makeExpect("Product", .ge, 500)],
        against: s
    )
    #expect(failures.isEmpty)
}

@Test
func singleExpectationUnsatisfiedRendersFormattedFailureWithGotN() {
    let s = snapshot([("Product", 247)])
    let failures = ExpectationEvaluator.evaluate(
        [makeExpect("Product", .ge, 500)],
        against: s
    )
    #expect(failures == [
        "records.where(typeName == \"Product\").count >= 500 (got 247)"
    ])
}

// MARK: - Multiple expectations preserve source order, only failures rendered

@Test
func multipleExpectationsRenderOnlyFailuresInSourceOrder() {
    let s = snapshot([("Product", 600), ("Variant", 0), ("PriceObservation", 0)])
    let failures = ExpectationEvaluator.evaluate(
        [
            makeExpect("Product", .ge, 500),           // pass
            makeExpect("Variant", .gt, 0),             // fail
            makeExpect("PriceObservation", .gt, 0),    // fail
        ],
        against: s
    )
    #expect(failures == [
        "records.where(typeName == \"Variant\").count > 0 (got 0)",
        "records.where(typeName == \"PriceObservation\").count > 0 (got 0)",
    ])
}

// MARK: - Each comparison op flips across its boundary

@Test
func opGreaterEqualBoundary() {
    let pass = snapshot([("T", 10)])
    let fail = snapshot([("T", 9)])
    #expect(ExpectationEvaluator.evaluate([makeExpect("T", .ge, 10)], against: pass).isEmpty)
    #expect(ExpectationEvaluator.evaluate([makeExpect("T", .ge, 10)], against: fail) ==
            ["records.where(typeName == \"T\").count >= 10 (got 9)"])
}

@Test
func opGreaterBoundary() {
    let pass = snapshot([("T", 11)])
    let fail = snapshot([("T", 10)])
    #expect(ExpectationEvaluator.evaluate([makeExpect("T", .gt, 10)], against: pass).isEmpty)
    #expect(ExpectationEvaluator.evaluate([makeExpect("T", .gt, 10)], against: fail) ==
            ["records.where(typeName == \"T\").count > 10 (got 10)"])
}

@Test
func opEqualBoundary() {
    let pass = snapshot([("T", 5)])
    let fail = snapshot([("T", 6)])
    #expect(ExpectationEvaluator.evaluate([makeExpect("T", .eq, 5)], against: pass).isEmpty)
    #expect(ExpectationEvaluator.evaluate([makeExpect("T", .eq, 5)], against: fail) ==
            ["records.where(typeName == \"T\").count == 5 (got 6)"])
}

@Test
func opNotEqualBoundary() {
    let pass = snapshot([("T", 6)])
    let fail = snapshot([("T", 5)])
    #expect(ExpectationEvaluator.evaluate([makeExpect("T", .ne, 5)], against: pass).isEmpty)
    #expect(ExpectationEvaluator.evaluate([makeExpect("T", .ne, 5)], against: fail) ==
            ["records.where(typeName == \"T\").count != 5 (got 5)"])
}

@Test
func opLessBoundary() {
    let pass = snapshot([("T", 9)])
    let fail = snapshot([("T", 10)])
    #expect(ExpectationEvaluator.evaluate([makeExpect("T", .lt, 10)], against: pass).isEmpty)
    #expect(ExpectationEvaluator.evaluate([makeExpect("T", .lt, 10)], against: fail) ==
            ["records.where(typeName == \"T\").count < 10 (got 10)"])
}

@Test
func opLessEqualBoundary() {
    let pass = snapshot([("T", 10)])
    let fail = snapshot([("T", 11)])
    #expect(ExpectationEvaluator.evaluate([makeExpect("T", .le, 10)], against: pass).isEmpty)
    #expect(ExpectationEvaluator.evaluate([makeExpect("T", .le, 10)], against: fail) ==
            ["records.where(typeName == \"T\").count <= 10 (got 11)"])
}

// MARK: - typeName filter is exact

@Test
func typeNameFilterIgnoresOtherTypes() {
    // 2 Products, 100 of other types. `Product.count == 2` should hold.
    let s = snapshot([("Product", 2), ("Variant", 50), ("PriceObservation", 50)])
    #expect(ExpectationEvaluator.evaluate([makeExpect("Product", .eq, 2)], against: s).isEmpty)
    #expect(ExpectationEvaluator.evaluate([makeExpect("Product", .ge, 100)], against: s) ==
            ["records.where(typeName == \"Product\").count >= 100 (got 2)"])
}

// MARK: - Empty inputs

@Test
func emptyExpectationsListYieldsNoFailures() {
    let s = snapshot([("Product", 0)])
    #expect(ExpectationEvaluator.evaluate([], against: s).isEmpty)
}

@Test
func emptySnapshotCountsAsZero() {
    let s = snapshot([])
    let failures = ExpectationEvaluator.evaluate(
        [makeExpect("Product", .ge, 1)],
        against: s
    )
    #expect(failures == [
        "records.where(typeName == \"Product\").count >= 1 (got 0)"
    ])
}
