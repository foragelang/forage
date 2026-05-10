import Foundation

/// HTTP-engine pagination strategies (the catch-all browser-engine
/// pagination is `BrowserPaginate`, which lives in its own module file).
///
/// Each variant declares the request-side param + response-side paths the
/// engine needs to drive the loop and recognize termination.
public enum Pagination: Hashable, Sendable {
    /// Send a page-number param (1-based or 0-based per `pageZeroIndexed`),
    /// response carries an `items` list and a `total` count. Stop when
    /// accumulated items ≥ total. Sweed.
    case pageWithTotal(
        itemsPath: PathExpr,
        totalPath: PathExpr,
        pageParam: String,
        pageSize: Int,
        pageZeroIndexed: Bool = false
    )

    /// Send a page-number param, response carries an `items` list. Stop when
    /// items shorter than page-size or empty. Leafbridge.
    case untilEmpty(
        itemsPath: PathExpr,
        pageParam: String,
        pageZeroIndexed: Bool = false
    )

    /// Server returns a continuation token in each response that gets sent
    /// back on the next request. Empty/nil cursor terminates.
    case cursor(
        itemsPath: PathExpr,
        cursorPath: PathExpr,
        cursorParam: String
    )
}
