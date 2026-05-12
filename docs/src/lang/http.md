# HTTP engine

Pick `engine http` when the data sits behind a documented JSON API. The
HTTP engine drives `reqwest` directly: ~1 req/sec rate limit, honest UA
(`Forage/x.y.z (+https://foragelang.com)`), exponential-backoff retry on
429/5xx (honors `Retry-After`), cookie jar shared across steps.

## Steps

```forage
step list {
    method "GET"
    url    "https://api.example.com/items?store={$input.storeId}"
    headers {
        "Accept": "application/json"
        "Origin": "{$input.siteOrigin}"
    }
}
```

After the request fires, the response body is bound to `$<stepName>`
(here `$list`) and stays in scope for the rest of the recipe.

## Bodies

Three body kinds:

```forage
// JSON body — recipe-side keys become a JSON object literal.
body.json {
    saleType:   "Recreational"
    platformOs: "web"
    page:       1
    filters: {
        category: [$catId]
    }
}

// Form body — application/x-www-form-urlencoded.
body.form {
    "action":            "wizard_show_products"
    "nonce_ajax":        "{$ajaxNonce}"
    "wizard_data[retailer_id]": "{$input.retailerId}"
}

// Raw body — string template, Content-Type defaults to text/plain.
body.raw "<custom-xml>{$input.query}</custom-xml>"
```

Body values support paths (`$input.x`), templates (`"{$x}"`), nested
objects/arrays, and `case`-of branches for enum-conditional fields.

## Pagination

Three strategies, each declared inside the step:

```forage
// Page + total count. Stop when accumulated ≥ total. (Sweed-style.)
paginate pageWithTotal {
    items: $.list, total: $.total,
    pageParam: "page", pageSize: 200
}

// Page until response is empty. (Leafbridge-style.)
paginate untilEmpty {
    items: $.data.products_list, pageParam: "prods_pageNumber"
}

// Cursor / continuation token.
paginate cursor {
    items:       $.results
    cursorPath:  $.next_cursor
    cursorParam: "cursor"
}
```

Each iteration appends the items to the bound step result; the engine
exits when the strategy says stop or when `maxIterations` (default 500)
is hit.

## Auth

The HTTP engine supports five auth flavors, declared as
`auth.<kind> { … }` at the recipe top level:

- `staticHeader` — name/value header injected on every request.
- `htmlPrime` — fetch an HTML page once, extract values via regex into
  scope variables, then run the rest of the body with those variables
  available.
- `session.formLogin` — POST credentials, capture cookies, thread them
  on subsequent requests.
- `session.bearerLogin` — POST credentials, extract a bearer token,
  inject it as a header.
- `session.cookiePersist` — load cookies from a file (escape hatch).

See [Auth](./auth.md) for the full options surface.

## Extract

A step can extract named groups out of its response body via a regex:

```forage
step prime {
    method "GET"
    url    "{$input.menuPageURL}"
    extract.regex {
        pattern: "leafbridge_public_ajax_obj\\s*=\\s*\\{\"ajaxurl\":\"([^\"]+)\",\"nonce\":\"([a-f0-9]+)\"\\}"
        groups:  [ajaxUrl, ajaxNonce]
    }
}
```

After the step runs, `$ajaxUrl` and `$ajaxNonce` are in scope for every
subsequent step. The leafbridge recipe uses this with `auth.htmlPrime`
to pull the nonce off the menu page before any AJAX call.
