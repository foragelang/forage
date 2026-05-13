```forage
recipe "onthisday"
engine http

type Event {
    year: Int
    text: String
    page: String?
}

input month: String
input day:   String

step feed {
    method "GET"
    url    "https://api.wikimedia.org/feed/v1/wikipedia/en/onthisday/events/{$input.month}/{$input.day}"
}

for $e in $feed.events[*] {
    emit Event {
        year ← $e.year
        text ← $e.text
        page ← $e.pages[0]?.titles?.normalized
    }
}
```

```sh
forage run recipes/onthisday --input month=05 --input day=10
```

```json
{
  "observedAt": "2026-05-11T15:11:17Z",
  "records": [
    {
      "_typeName": "Event",
      "fields": {
        "year": 2024,
        "text": "Start of the May 2024 solar storms, the most powerful set of geomagnetic storms since the 2003 Halloween solar storms.",
        "page": "May 2024 solar storms"
      }
    },
    {
      "_typeName": "Event",
      "fields": {
        "year": 1869,
        "text": "The First Transcontinental Railroad is completed in the United States with the driving of the golden spike at Promontory Summit, Utah.",
        "page": "First Transcontinental Railroad"
      }
    }
  ]
}
```
