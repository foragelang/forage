# GitHub Releases — typed inputs + cursor pagination

A recipe that takes consumer inputs (`owner`, `repo`) and walks the
GitHub releases API.

```forage
recipe "github-releases" {
    engine http

    type Release {
        tag:        String
        name:       String?
        publishedAt: String?
        prerelease: Bool
        url:        String
    }

    input owner: String
    input repo:  String

    step list {
        method "GET"
        url    "https://api.github.com/repos/{$input.owner}/{$input.repo}/releases?per_page=100"
        headers {
            "Accept": "application/vnd.github+json"
            "User-Agent": "forage"
        }
        paginate cursor {
            items:       $.
            cursorPath:  $.<next-from-Link-header>     // (see below)
            cursorParam: "page"
        }
    }

    for $r in $list[*] {
        emit Release {
            tag         ← $r.tag_name
            name        ← $r.name
            publishedAt ← $r.published_at
            prerelease  ← $r.prerelease
            url         ← $r.html_url
        }
    }
}
```

## Inputs

`fixtures/inputs.json`:

```json
{
    "owner": "rust-lang",
    "repo":  "rust"
}
```

The URL template interpolates them at request time:

```
https://api.github.com/repos/rust-lang/rust/releases?per_page=100
```

The hub releases endpoint has well-documented behavior: returns up to
`per_page` items, with a `Link` header for the next page. Forage's
cursor strategy follows the link until exhausted.

## Notes

- The validator catches a typo in `$input.owner` or `$input.repo` at
  parse time — try changing one to `$input.ower` to see the error.
- `Accept: application/vnd.github+json` opts into GitHub's stable v3
  API representation. The `User-Agent: forage` header is recommended
  by GitHub for rate-limit attribution; without it you get 60
  requests/hour shared with every other anonymous client on your IP.
- The current pagination strategy approximates `Link`-header-driven
  pagination as a cursor. A future engine release will add a
  `linkHeader` strategy that consumes the `Link` directly.

## Authenticated

GitHub raises the rate limit to 5000/hour with a bearer token. Add
auth.staticHeader:

```forage
secret githubToken

auth.staticHeader {
    name:  "Authorization"
    value: "Bearer {$secret.githubToken}"
}
```

Then run with the token in the environment:

```sh
FORAGE_SECRET_GITHUBTOKEN=ghp_xxx forage run recipes/github-releases
```

The token never appears in the recipe text, never appears in
diagnostics, never persists to disk.
