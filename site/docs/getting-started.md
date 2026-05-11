# Getting started

Write your first recipe and run it end-to-end in a few minutes.

## Requirements

- macOS 14 (Sonoma) or newer
- Swift 6.0 or newer (Xcode 16+)

The browser engine uses `WKWebView` and is macOS-only. The HTTP engine is platform-portable in principle, but the current package targets macOS 14+.

## Install

Forage isn't yet published to a registry. Clone the repo and build locally:

```sh
git clone https://github.com/foragelang/forage.git
cd forage
swift build
swift test
```

That produces `.build/debug/forage-probe`, the CLI you'll use to run recipes.

To use Forage as a library in your own Swift package, point at the local checkout while we work toward a tagged release:

```swift
.package(path: "../forage")
```

```swift
.product(name: "Forage", package: "forage")
```

## Write a recipe

Make a new directory under `recipes/` and drop in a `recipe.forage` file:

```sh
mkdir -p recipes/hello
$EDITOR recipes/hello/recipe.forage
```

A minimal recipe against a documented JSON endpoint:

```forage
// recipes/hello/recipe.forage

recipe "hello" {
    engine http

    type Post {
        externalId: String
        title:      String
        body:       String?
    }

    input userId: Int

    step posts {
        method "GET"
        url    "https://jsonplaceholder.typicode.com/posts?userId={$input.userId}"
    }

    for $p in $posts[*] {
        emit Post {
            externalId ← $p.id | toString
            title      ← $p.title
            body       ← $p.body
        }
    }
}
```

Four things to notice:

- `engine http` — this recipe will run on the HTTP engine, not the browser engine.
- `type Post { … }` — declares the shape of the records this recipe emits.
- `input userId: Int` — declares a per-run parameter. You'll supply it on the command line.
- `step posts { … }` — names an HTTP request whose response becomes addressable as `$posts`.

## Run it

Use `forage-probe` to parse, validate, and run the recipe:

```sh
.build/debug/forage-probe run recipes/hello/recipe.forage --input userId=1
```

The CLI parses the recipe, validates it against the type catalog, runs the HTTP graph, and prints the resulting snapshot — a JSON list of every emitted record — to stdout.

::: tip Validation runs first
Unknown types, unbound path variables, missing required fields, and unknown transforms are caught before any HTTP request fires. The error format speaks the DSL's own terms — no stack traces from extraction code.
:::

## From here

- Read the [syntax reference](/docs/syntax) for the full set of constructs: `enum`, `auth` strategies, `case` expressions, optional chaining, transforms.
- Read [engines & pagination](/docs/engines) when you need to scrape a paginated API or a JS-rendered SPA.
- Look at the bundled [reference recipes](https://github.com/foragelang/forage/tree/main/recipes) for end-to-end examples that exercise the full DSL.
