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
```

```sh
forage run recipes/hacker-news
```

```json
{
  "observedAt": "2026-05-11T15:11:11Z",
  "records": [
    {
      "_typeName": "Story",
      "fields": {
        "title": "Hardware Attestation as Monopoly Enabler",
        "url": "https://grapheneos.social/@GrapheneOS/116550899908879585",
        "points": 1879,
        "author": "ChuckMcM",
        "comments": 617
      }
    },
    {
      "_typeName": "Story",
      "fields": {
        "title": "Local AI needs to be the norm",
        "url": "https://unix.foo/posts/local-ai-needs-to-be-norm/",
        "points": 1498,
        "author": "cylo",
        "comments": 580
      }
    }
  ]
}
```
