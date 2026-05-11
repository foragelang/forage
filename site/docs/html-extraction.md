# HTML / DOM extraction

Forage treats HTML the same way it treats JSON: a parsed value queryable by path expressions and pipes. There's no second grammar for DOM — the recipe-author skills you already have for JSON apply unchanged. A handful of transforms (`parseHtml`, `select`, `text`, `attr`, …) and one grammar extension (for-loops accept pipelines) are all that distinguish HTML extraction from JSON extraction in a recipe.

## The shape

```forage
recipe "example" {
    engine http

    type Story {
        title: String
        url:   String?
    }

    step front {
        method "GET"
        url    "https://news.ycombinator.com"
    }

    for $title in $front | parseHtml | select(".titleline") {
        emit Story {
            title ← $title | select("a") | text
            url   ← $title | select("a") | attr("href")
        }
    }
}
```

What's happening:

1. **`$front`** is the response body. When the server returned `Content-Type: text/html` the body comes through as a string instead of a JSON-parse failure.
2. **`parseHtml`** turns the string into a queryable node.
3. **`select(".titleline")`** returns an array of matching nodes (CSS selectors, jQuery-style).
4. **`for $title in <pipeline>`** iterates over that array. Each `$title` is bound to one matched node.
5. Inside the loop, **`$title | select("a") | text`** chains: get the `<a>` descendants, take the first one's text. (`text` / `attr` / `html` auto-flatten a single-element array — the jQuery convention.)

## The transforms

| Transform | Receives | Returns | Purpose |
|---|---|---|---|
| `parseHtml` | string | node | Parse an HTML/XML document. Lenient — malformed markup works. |
| `parseJson` | string | JSON | The companion for the "data is embedded in a `<script>`" pattern. |
| `select(sel)` | node | [node] | CSS selector match. Returns an array, even for one match. |
| `text` | node \| [node] | string | Whitespace-collapsed text content. Auto-flattens single-element array. |
| `attr(name)` | node \| [node] | string? | Attribute value, or null if missing/empty. |
| `html` | node \| [node] | string | Outer HTML (the wrapping tag and everything inside). |
| `innerHtml` | node \| [node] | string | Inner HTML (children only). |
| `first` | array | element \| null | Explicit head-of-list. |

`select` always returns an array because most CSS selectors match more than one element. When you only want the first match's text/attr, the auto-flatten on `text`/`attr`/`html` saves you a `| first` call. When you want all matches, drive a `for $x in ...` loop.

## When recipes need HTML extraction

The native fit is **server-rendered HTML pages with no public API.** Three common shapes:

1. **Classic server-rendered sites.** Wikipedia, news.ycombinator.com, government data portals, Craigslist, public records databases. The data is in the HTML; there's no JSON endpoint.
2. **SSR with embedded JSON.** Modern Next.js / Remix sites often render a `<script id="__NEXT_DATA__">{…}</script>` blob containing the data the React tree was hydrated from. Pattern: `$page | parseHtml | select("script#__NEXT_DATA__") | text | parseJson | $.props.pageProps.results[*]`.
3. **Hybrid pages with both.** Some pages render the first batch as HTML and subsequent batches via XHR. The HTML-extraction primitive handles the first; existing `captures.match` (browser engine) handles the rest. Same recipe, both shapes.

For sites that need **Cloudflare-gated** access or are fully JS-rendered with no useful initial HTML (eBay search results, Datadome-protected sites), you'll want the **browser engine**. M9 (browser-engine document capture) adds the missing piece — extracting from the rendered document body the same way HTTP recipes extract from a static response.

## Content-type dispatch

`step` HTTP responses are decoded by content type:

- `application/json` (or no content-type with parseable JSON) → response is a JSON value.
- `text/html`, `text/xml`, `text/plain`, etc. → response is a string. Pipe through `parseHtml` to query.

The fallback is intentional: an HTML response doesn't crash the recipe; it just lands as a string the recipe explicitly chooses to parse. This makes the parsing step legible at the call site rather than implicit.

## Recipe inventory

- **`recipes/hacker-news-html/`** — HN front page scraped from the rendered HTML, as a companion to the JSON-API version in `recipes/hacker-news/`. Same record shape, different data source.
- **`recipes/scotus-opinions/`** — US Supreme Court slip opinions for a given term, extracted from supremecourt.gov's HTML table. Typed `Opinion` records with date, docket number, case name, PDF URL, and holding text.

Both are the smallest recipes that exercise the primitive end-to-end; copy either as a starting template.
