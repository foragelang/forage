# Hacker News — the smallest end-to-end recipe

A one-step JSON-API recipe with no auth, no inputs, no pagination.

```forage
recipe "hacker-news"
engine http

type Story {
    title:    String
    url:      String?
    points:   Int
    author:   String
    comments: Int
}

step front {
    method "GET"
    url    "https://hn.algolia.com/api/v1/search?tags=front_page&hitsPerPage=30"
}

for $hit in $front.hits[*] {
    emit Story {
        title    ← $hit.title
        url      ← $hit.url
        points   ← $hit.points
        author   ← $hit.author
        comments ← $hit.num_comments
    }
}

expect { records.where(typeName == "Story").count >= 20 }
```

## What's happening

1. `engine http` picks the HTTP engine; the runtime drives `reqwest`
   directly.
2. The `Story` type declares the output shape. `title`, `points`,
   `author`, `comments` are required; `url` is optional because some HN
   stories are "Ask HN" with no link.
3. `step front` does one GET. The Algolia search endpoint returns
   `{hits: [{title, url, points, author, num_comments, …}, …]}` and
   the response binds to `$front`.
4. `for $hit in $front.hits[*]` iterates the array; `[*]` widens it.
5. The `emit Story { … }` block maps each hit to a `Story` record.
   `←` is the binding operator; the LHS is a field on the type, the
   RHS is the expression.
6. `expect` is a postcondition — if HN returns fewer than 20 stories,
   the diagnostic flags it. The records still land in the snapshot.

## Run it

```sh
forage run recipes/hacker-news

• Story (30 records)
  [0] title: "Hardware Attestation as Monopoly Enabler", url: …, points: 2095, author: "ChuckMcM", comments: 708
  [1] title: "Postmortem: TanStack npm supply-chain compromise", url: …, points: 624, …
  27 more …
```

## Replay

Record once:

```sh
mkdir -p recipes/hacker-news/fixtures
curl 'https://hn.algolia.com/api/v1/search?tags=front_page&hitsPerPage=30' > /tmp/hn.json
jq -c '{kind: "http", url: "https://hn.algolia.com/api/v1/search?tags=front_page&hitsPerPage=30", method: "GET", status: 200, body: tojson}' /tmp/hn.json > recipes/hacker-news/fixtures/captures.jsonl
```

Then:

```sh
forage run recipes/hacker-news --replay     # no network
forage test recipes/hacker-news --update    # write expected.snapshot.json
forage test recipes/hacker-news             # exit-0 means the recipe still matches the snapshot
```

## Why this is the smallest example

- One step, one emit type.
- No auth.
- No pagination — `hitsPerPage=30` covers what we need.
- No inputs — the URL is hardcoded.
- No browser engine.
- One expectation.

Every other in-tree recipe layers on at least one extra piece —
pagination (`scotus-opinions`), auth (`leafbridge`), browser engine
(`letterboxd-popular`), interactive bootstrap (`ebay-sold`).
