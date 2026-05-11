import Foundation

/// Evaluates a recipe's `expect { … }` clauses against a produced `Snapshot`.
///
/// The parsed AST (`Expectation.kind`) currently carries only the structural
/// form `recordCount(typeName, op, value)` — i.e.
/// `records.where(typeName == "X").count <op> N`. The evaluator counts
/// records of `typeName` in the snapshot, applies the comparison, and
/// renders any failing expectation back into recipe-source-like text with
/// the actual count appended: e.g.
///
///     records.where(typeName == "Product").count >= 500 (got 247)
///
/// Pure: no I/O, no logging, no engine state — engines are the only callers.
/// If the AST grows new forms in the future (predicates on field values,
/// boolean combinators, etc.), this evaluator should render them as
/// `unsupported: <description>` until it learns to interpret them, rather
/// than crash.
public enum ExpectationEvaluator {
    public static func evaluate(_ expectations: [Expectation], against snapshot: Snapshot) -> [String] {
        var failures: [String] = []
        for expectation in expectations {
            if let failure = evaluate(expectation, against: snapshot) {
                failures.append(failure)
            }
        }
        return failures
    }

    private static func evaluate(_ expectation: Expectation, against snapshot: Snapshot) -> String? {
        switch expectation.kind {
        case let .recordCount(typeName, op, value):
            let actual = snapshot.records.reduce(0) { acc, r in
                r.typeName == typeName ? acc + 1 : acc
            }
            if compare(actual, op, value) {
                return nil
            }
            return "records.where(typeName == \"\(typeName)\").count \(op.rawValue) \(value) (got \(actual))"
        }
    }

    private static func compare(_ lhs: Int, _ op: ComparisonOp, _ rhs: Int) -> Bool {
        switch op {
        case .ge: return lhs >= rhs
        case .gt: return lhs >  rhs
        case .le: return lhs <= rhs
        case .lt: return lhs <  rhs
        case .eq: return lhs == rhs
        case .ne: return lhs != rhs
        }
    }
}
