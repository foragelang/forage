```forage
recipe "nasa-apod" {
    engine http

    type Picture {
        date:        String
        title:       String
        url:         String
        explanation: String
        copyright:   String?
    }

    input start: String
    input end:   String

    step archive {
        method "GET"
        url    "https://api.nasa.gov/planetary/apod?api_key=DEMO_KEY&start_date={$input.start}&end_date={$input.end}"
    }

    for $a in $archive[*] {
        emit Picture {
            date        ← $a.date
            title       ← $a.title
            url         ← $a.url
            explanation ← $a.explanation
            copyright   ← $a.copyright
        }
    }
}
```

```sh
forage run recipes/nasa-apod --input start=2025-05-01 --input end=2025-05-03
```

```json
{
  "observedAt": "2026-05-11T15:11:18Z",
  "records": [
    {
      "_typeName": "Picture",
      "fields": {
        "date": "2025-05-01",
        "title": "MESSENGER's Last Day on Mercury",
        "url": "https://apod.nasa.gov/apod/image/2505/messengerImpactSite_black600.jpg",
        "explanation": "The first to orbit inner planet Mercury, the MESSENGER spacecraft came to rest on this region of Mercury's surface on April 30, 2015...",
        "copyright": null
      }
    },
    {
      "_typeName": "Picture",
      "fields": {
        "date": "2025-05-02",
        "title": "Young Star Cluster NGC 346",
        "url": "https://apod.nasa.gov/apod/image/2505/jwst-ngc346-800.png",
        "explanation": "The most massive young star cluster in the Small Magellanic Cloud is NGC 346, embedded in our small satellite galaxy's largest star forming region...",
        "copyright": null
      }
    }
  ]
}
```
