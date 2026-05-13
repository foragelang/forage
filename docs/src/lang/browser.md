# Browser engine

Pick `engine browser` when the data only exists after JS rendering, when
the page sits behind a JS-execution bot check (Cloudflare's basic
challenge, Akamai's basic fingerprint check), or when the SPA pulls
data via fetch/XHR that's easier to capture than to reverse.

A browser recipe declares one `browser { … }` block:

```forage
recipe "letterboxd-popular"
engine browser

type Film { title: String, url: String?, posterUrl: String? }

browser {
    initialURL: "https://letterboxd.com/films/popular/this/week/"
    observe:    "letterboxd.com"

    ageGate.autoFill { dob: 1990-01-01, reloadAfter: true }
    dismissals { maxIterations: 4 }
    warmupClicks: [".cookie-banner button.dismiss"]

    paginate browserPaginate.scroll {
        until:          noProgressFor(2)
        maxIterations:  0           // 0 = unbounded, until-rule decides
        iterationDelay: 1.8
    }

    captures.document {
        for $poster in $ | select("div.poster.film-poster") {
            emit Film {
                title     ← $poster | select("span.frame-title") | text
                url       ← $poster | select("a.frame") | attr("href")
                posterUrl ← $poster | select("img") | attr("src")
            }
        }
    }
}

expect { records.where(typeName == "Film").count >= 50 }
```

## Fields

- **`initialURL`** — first navigation. Templated, so it can interpolate
  inputs.
- **`observe`** — substring of the URL the engine watches for settle.
- **`ageGate.autoFill { dob, reloadAfter }`** — handles the
  date-of-birth form many cannabis dispensary SPAs put in front of the
  menu.
- **`dismissals { maxIterations, extraLabels: […] }`** — clicks
  cookie-banner / "I'm 21" / "Accept all" buttons automatically.
- **`warmupClicks: ["selector", …]`** — clicks selectors before
  scraping, in order.
- **`paginate`** — `browserPaginate.scroll` or `browserPaginate.replay`.
  `until: noProgressFor(N)` says "stop when N consecutive scrolls
  produce no new captures."
- **`interactive { bootstrapURL?, cookieDomains, sessionExpiredPattern }`**
  — M10 interactive bootstrap (see [Interactive bootstrap](../runtime/interactive.md)).

## Captures

Two capture-rule kinds, both nest emit blocks the same way the HTTP
engine does:

### `captures.match`

Fires once per fetch/XHR response whose URL matches `urlPattern` (a
regex). The matched body becomes the iteration source:

```forage
captures.match {
    urlPattern: "iheartjane.com/v2/smartpage"
    for $product in $.products[*] {
        emit Product {
            externalId ← $product.search_attributes.objectID
            name       ← $product.search_attributes.name
            // …
        }
    }
}
```

### `captures.document`

Fires once after the engine reports settle. The "current value" inside
the block is the parsed document node, walkable with `select` / `text` /
`attr`:

```forage
captures.document {
    for $row in $ | select("table.menu tbody tr") {
        emit Item {
            name  ← $row | select(".name") | text
            price ← $row | select(".price") | text | parseFloat
        }
    }
}
```

## How it runs

- **CLI (`forage run --replay`)** drives the recipe against
  `fixtures/captures.jsonl` — no webview needed. Useful for snapshot
  diffing and CI.
- **CLI (`forage run`)** for browser-engine recipes needs a webview
  event loop the CLI doesn't host on its own; use Forage Studio for
  live runs.
- **Forage Studio** opens a Tauri `WebviewWindow`, injects a fetch/XHR
  shim, scrolls until settle (or until `noProgressFor(N)`), routes the
  captures through the same evaluator as replay mode.

The engine doesn't try to defeat real anti-bot challenges. JS-execution
checks succeed against us *because we're a real browser engine* —
WKWebView on macOS, WebView2 on Windows, WebKitGTK on Linux. CAPTCHA
and human-verification gates need [M10 interactive bootstrap](../runtime/interactive.md):
a person solves the challenge once, the session is reused headlessly
until expiry.
