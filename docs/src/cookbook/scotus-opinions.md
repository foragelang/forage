# SCOTUS opinions — HTML extraction in an HTTP recipe

The Supreme Court publishes slip opinions as an HTML table at
`supremecourt.gov/opinions/slipopinion/<term>`. No JSON API, but the
data lives in the initial document body — perfect for the HTTP engine
+ HTML transforms.

```forage
recipe "scotus-opinions"
engine http

type Opinion {
    date:        String
    docket:      String
    caseName:    String
    pdfUrl:      String
    holdingText: String?
}

input term: String   // "24" for OT24, etc.

step term_page {
    method "GET"
    url    "https://www.supremecourt.gov/opinions/slipopinion/{$input.term}"
}

for $row in $term_page | parseHtml | select("table#OpinionsTable tbody tr") {
    emit Opinion {
        date     ← $row | select("td:nth-child(1)") | text
        docket   ← $row | select("td:nth-child(2)") | text
        caseName ← $row | select("td:nth-child(3) a") | text
        pdfUrl   ← $row | select("td:nth-child(3) a") | attr("href")
        holdingText ← $row | select("td:nth-child(4)") | text
    }
}

expect { records.where(typeName == "Opinion").count >= 1 }
```

## What's new vs Hacker News

- **HTML response.** `$term_page` is the raw HTML the GET returned —
  the runtime detected it's not JSON and bound it as a string.
- **`parseHtml`** wraps the string as a node for CSS-selector
  navigation. Without it, `select` on a plain string would error.
- **`select(...)`** is the CSS-selector transform; `text` and
  `attr(name)` extract content.
- **Nested selectors.** `$row | select("td:nth-child(3) a") | attr("href")`
  is a transform pipeline. Each pipe stage transforms the previous
  value; the same operator works for JSON paths, HTML walks, and
  scalar coercions.

## Run it

Save the recipe as `scotus-opinions.forage` at the workspace root,
then run with an inputs file:

```sh
echo '{"term":"24"}' > /tmp/scotus-inputs.json
forage run scotus-opinions --inputs /tmp/scotus-inputs.json

• Opinion (42 records)
  [0] date: "11/07/24", docket: "23-715", caseName: "City of …", pdfUrl: "/opinions/24pdf/23-715_…"
  …
```

## Why HTML extraction matters

A surprising number of civic-data sites have no API. The HTTP + HTML
combination handles every one we've come across without resorting to
the browser engine (which is more expensive and harder to replay):

- `hacker-news-html` — HN's `news.ycombinator.com` listing.
- `scotus-opinions` — this one.
- `onthisday` — Wikipedia's "on this day" calendar.

The browser engine is reserved for SPAs that *do* render in JS or
sites behind bot management.
