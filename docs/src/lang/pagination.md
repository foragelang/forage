# Pagination

Pagination is declared inside a step (HTTP) or inside `browser { … }`
(browser-engine). The runtime drives the strategy; the recipe stays
declarative.

## HTTP

Three HTTP strategies cover essentially every documented API:

### `pageWithTotal`

Sites that paginate by page number and tell you the total count.

```forage
paginate pageWithTotal {
    items:           $.list           // path to the page's items in the response
    total:           $.total          // path to the total count
    pageParam:       "page"           // query param the engine increments
    pageSize:        200
    pageZeroIndexed: false            // optional; true for 0-based APIs
}
```

Stops when `accumulated_items >= total`, when a page is empty, or when
the engine's `max_requests` safety cap (default 500) is hit.

### `untilEmpty`

Sites that don't tell you the total — paginate until a page is empty.

```forage
paginate untilEmpty {
    items:     $.data.products_list
    pageParam: "prods_pageNumber"
}
```

### `cursor`

Sites that hand you a continuation token in each response.

```forage
paginate cursor {
    items:       $.results
    cursorPath:  $.next_cursor
    cursorParam: "cursor"
}
```

Stops on null/empty cursor.

## Browser

Browser pagination is part of the `browser { … }` block:

```forage
paginate browserPaginate.scroll {
    until:          noProgressFor(3)
    maxIterations:  30
    iterationDelay: 1.8
}
```

The engine scrolls to the bottom, waits `iterationDelay` seconds, then
checks for new captures. After `noProgressFor(N)` consecutive idle
windows, it considers the page settled. `maxIterations: 0` means
unbounded (the until-rule decides).

`browserPaginate.replay` lets a recipe deterministically iterate a
captured sequence — useful when the page's pagination shape is hard to
reproduce headlessly.
