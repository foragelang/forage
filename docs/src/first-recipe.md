# Your first recipe

A Forage recipe is a `.forage` file at the workspace root with a
`recipe "<name>"` header. The simplest possible recipe is one HTTP step
plus an emit loop:

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

Save this as `hacker-news.forage` at your workspace root
(`~/Library/Forage/Recipes/` on macOS by default). Then run it live:

```sh
$ cd ~/Library/Forage/Recipes
$ forage run hacker-news
• Story (30 records)
  [0] title: "Hardware Attestation as Monopoly Enabler", points: 2095, author: "ChuckMcM", comments: 708
  [1] title: "Postmortem: TanStack npm supply-chain compromise", points: 624, author: "varunsharma07", comments: 238
  …
```

`forage run <recipe>` resolves `<recipe>` to the file declaring
`recipe "<recipe>"` in the surrounding workspace, parses, validates,
then dispatches to the HTTP engine. The engine fetches `front`, binds
the JSON body as `$front`, walks `$front.hits[*]` and emits one `Story`
per hit. After the body finishes, the runtime evaluates expectations
against the snapshot.

## Anatomy

- `recipe "<name>"` — top-level recipe header. The header name is the
  recipe's identity — the daemon, output stores, fixtures, snapshots,
  and hub publishes all key on it. File basenames are organizational.
- `engine http` — pick `http` for JSON APIs, `browser` for SPAs.
- `type <Name> { field: T, … }` — recipe-declared record types. Required
  fields use bare `T`; optional fields use `T?`. Prefix with `share` to
  make the declaration workspace-visible; without `share`, it's
  file-scoped.
- `input <name>: T` — consumer-supplied inputs. `forage run` takes
  `--inputs <path>` pointing at a JSON object of bindings.
- `secret <name>` — referenced via `$secret.<name>`; resolved from
  `FORAGE_SECRET_<NAME>` env vars (CLI) or the OS keychain (Studio).
- `step <name> { method, url, headers?, body?, paginate? }` — one HTTP
  request. The response body is bound to `$<stepName>`.
- `for $x in <expression> { … }` — iterate a list, with `$x` and `$.`
  bound to each item.
- `emit <Type> { field ← <expression>, … }` — produce one record.
- `expect { records.where(typeName == "X").count >= N }` — declarative
  postcondition; the runtime reports unmet ones via the diagnostic.

## Next

- [HTTP engine](./lang/http.md) — pagination strategies, auth flavors,
  request bodies.
- [Browser engine](./lang/browser.md) — for SPAs and Cloudflare-gated pages.
- [Transforms](./lang/transforms.md) — the `lower`, `dedup`, `parseHtml`,
  `select`, `text`, … family.
- [Forage Studio](./studio.md) — interactive authoring with live captures.
