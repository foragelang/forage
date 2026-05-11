## A recipe, end to end

Hit Wikipedia's REST API and emit one typed `Article`.

```forage
recipe "wikipedia" {
    engine http

    type Article {
        title:   String
        extract: String
        url:     String
    }

    input topic: String

    step page {
        method "GET"
        url    "https://en.wikipedia.org/api/rest_v1/page/summary/{$input.topic}"
    }

    emit Article {
        title   ← $page.title
        extract ← $page.extract
        url     ← $page.content_urls.desktop.page
    }
}
```

Run it:

```sh
forage run recipes/wikipedia --input topic=Foraging
```

## What happens during a run

<div class="run-flow">
  <div class="run-step">
    <div class="run-step-num">1</div>
    <div class="run-step-title">Parse</div>
    <div class="run-step-body">Recipe text becomes a typed AST. Schema-checked against the type catalog before anything runs.</div>
  </div>
  <div class="run-arrow">→</div>
  <div class="run-step">
    <div class="run-step-num">2</div>
    <div class="run-step-title">Fetch</div>
    <div class="run-step-body">Engine runs the HTTP graph or drives a headless browser. Pagination and rate-limits are applied.</div>
  </div>
  <div class="run-arrow">→</div>
  <div class="run-step">
    <div class="run-step-num">3</div>
    <div class="run-step-title">Extract</div>
    <div class="run-step-body">Walks responses, evaluates path expressions, builds typed records.</div>
  </div>
  <div class="run-arrow">→</div>
  <div class="run-step">
    <div class="run-step-num">4</div>
    <div class="run-step-title">Report</div>
    <div class="run-step-body">Returns a snapshot plus a diagnostic report. Both can be archived to disk for replay.</div>
  </div>
</div>

The snapshot prints to stdout, a short termination tag to stderr:

```json
{
  "observedAt": "2026-05-10T15:23:11Z",
  "records": [
    {
      "_typeName": "Article",
      "fields": {
        "title": "Foraging",
        "extract": "Foraging is searching for wild food resources...",
        "url": "https://en.wikipedia.org/wiki/Foraging"
      }
    }
  ]
}
```

```
stallReason: completed
```
