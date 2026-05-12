# Captures

Captures are the bridge between "what the browser sees" and "what the
recipe emits." Two flavors, declared inside `browser { … }`:

## `captures.match`

Fires per fetch/XHR response whose URL matches a regex.

```forage
captures.match {
    urlPattern: "iheartjane.com/v2/smartpage"
    for $product in $.products[*] {
        emit Product {
            externalId       ← $product.search_attributes.objectID
            name             ← $product.search_attributes.name
            description      ← $product.search_attributes.description
            strainPrevalence ← $product.search_attributes.category | prevalenceNormalize
            images           ← $product.search_attributes.image_urls
        }
        for $w in $product.search_attributes.available_weights[*] {
            emit Variant {
                externalId ← "{$product.search_attributes.objectID}:{$w}"
                name       ← $w
                sizeValue  ← $w | parseJaneWeight
                sizeUnit   ← $w | janeWeightUnit
            }
        }
    }
}
```

- `urlPattern` is a regex anchored anywhere in the URL.
- The response body parses as JSON (with a fallback to raw string).
- The capture's "current value" (`$.`) and the loop variable both bind
  to the parsed body.
- The body is the same statement surface as the HTTP engine's body —
  `for`, `emit`, nested loops freely compose.

Multiple `captures.match` blocks coexist; each fires on its own pattern.
A response that matches several patterns runs each rule once.

## `captures.document`

Fires once after the engine reports settle. The current value is the
parsed document HTML.

```forage
captures.document {
    for $poster in $ | select("div.poster") {
        emit Film {
            title ← $poster | select("span.frame-title") | text
            url   ← $poster | select("a.frame") | attr("href")
        }
    }
}
```

- `$ | select(selector)` walks the document with the `scraper` CSS
  engine (the same library that powers Rust web scrapers; equivalent
  shape to the browser's `document.querySelectorAll`).
- Subsequent `select` calls inside the loop are nested selectors.
- `text` extracts the trimmed text content; `attr("name")` extracts an
  attribute value; `html` / `innerHtml` return the markup.

Only one `captures.document` rule per recipe.

## Replay vs live

In **replay mode** (`forage run --replay`), captures come from
`fixtures/captures.jsonl`. Each line is one capture:

```jsonl
{"kind":"browser","subkind":"match","url":"https://iheartjane.com/v2/smartpage?page=1","method":"GET","status":200,"body":"{\"products\":[…]}"}
{"kind":"browser","subkind":"document","url":"https://letterboxd.com/films/popular","html":"<html>…</html>"}
```

In **live mode** (Forage Studio), the same captures are produced by the
injected fetch/XHR shim, routed through the same evaluator. Replay and
live runs are bit-for-bit equivalent against the same capture stream;
the only difference is where the captures come from.
