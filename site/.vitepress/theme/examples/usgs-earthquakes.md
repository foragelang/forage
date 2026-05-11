```forage
recipe "usgs-earthquakes" {
    engine http

    type Quake {
        magnitude: Double
        place:     String?
        time:      Int
        depth:     Double
        url:       String
    }

    step feed {
        method "GET"
        url    "https://earthquake.usgs.gov/earthquakes/feed/v1.0/summary/4.5_week.geojson"
    }

    for $f in $feed.features[*] {
        emit Quake {
            magnitude ← $f.properties.mag
            place     ← $f.properties.place
            time      ← $f.properties.time
            depth     ← $f.geometry.coordinates[2]
            url       ← $f.properties.url
        }
    }
}
```

```sh
forage run recipes/usgs-earthquakes
```

```json
{
  "observedAt": "2026-05-11T15:11:18Z",
  "records": [
    {
      "_typeName": "Quake",
      "fields": {
        "magnitude": 5.2,
        "place": "72 km NW of Malango, Solomon Islands",
        "time": 1778492931604,
        "depth": 10,
        "url": "https://earthquake.usgs.gov/earthquakes/eventpage/us6000swvm"
      }
    },
    {
      "_typeName": "Quake",
      "fields": {
        "magnitude": 4.6,
        "place": "46 km WSW of San Pedro de Atacama, Chile",
        "time": 1778477752128,
        "depth": 112.298,
        "url": "https://earthquake.usgs.gov/earthquakes/eventpage/us6000swuu"
      }
    }
  ]
}
```
