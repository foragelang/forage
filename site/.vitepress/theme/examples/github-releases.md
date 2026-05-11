```forage
recipe "github-releases" {
    engine http

    type Release {
        tag:       String
        name:      String?
        published: String
        url:       String
    }

    input owner: String
    input repo:  String

    step releases {
        method "GET"
        url    "https://api.github.com/repos/{$input.owner}/{$input.repo}/releases?per_page=15"
    }

    for $r in $releases[*] {
        emit Release {
            tag       ← $r.tag_name
            name      ← $r.name
            published ← $r.published_at
            url       ← $r.html_url
        }
    }
}
```

```sh
forage run recipes/github-releases --input owner=swiftlang --input repo=swift
```

```json
{
  "observedAt": "2026-05-11T15:11:18Z",
  "records": [
    {
      "_typeName": "Release",
      "fields": {
        "tag": "swift-6.3.1-RELEASE",
        "name": "Swift 6.3.1 Release",
        "published": "2026-04-17T02:25:40Z",
        "url": "https://github.com/swiftlang/swift/releases/tag/swift-6.3.1-RELEASE"
      }
    },
    {
      "_typeName": "Release",
      "fields": {
        "tag": "swift-6.3-RELEASE",
        "name": "Swift 6.3 Release",
        "published": "2026-03-27T18:12:14Z",
        "url": "https://github.com/swiftlang/swift/releases/tag/swift-6.3-RELEASE"
      }
    }
  ]
}
```
