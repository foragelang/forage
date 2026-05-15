# Getting started

Write your first recipe and run it end-to-end in a few minutes.

## Requirements

- macOS 14, Linux, or Windows 10+
- Rust 1.85 or newer

The browser engine runs through `wry`: WKWebView on macOS, WebView2 on
Windows, WebKitGTK on Linux.

## Install

Once releases ship, `brew install foragelang/forage/forage` or
`curl -fsSL https://foragelang.com/install.sh | sh`. Until then, build
from source:

```sh
git clone https://github.com/foragelang/forage.git
cd forage
cargo build --release --bin forage
./target/release/forage --version
```

See the [CLI reference](/docs/cli) for the full subcommand surface.

To use Forage as a library in your own Rust crate, point your
`Cargo.toml` at the local checkout while we work toward a tagged
release:

```toml
[dependencies]
forage-core = { path = "../forage/crates/forage-core" }
forage-http = { path = "../forage/crates/forage-http" }
```

## Write a recipe

Your workspace is the directory marked by `forage.toml` (typically
`~/Library/Forage/Recipes/` on macOS). Recipe files sit flat at the
workspace root — one `.forage` file per recipe, named however you like.
Scaffold one with `forage new`:

```sh
cd ~/Library/Forage/Recipes
forage new hello
$EDITOR hello.forage
```

`forage new <name>` creates `<workspace>/<name>.forage` with a minimal
`recipe "<name>" engine http` header. Edit it to match a documented JSON
endpoint:

```forage
// hello.forage

recipe "hello"
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
```

Four things to notice:

- `engine http` — this recipe will run on the HTTP engine, not the browser engine.
- `type Post { … }` — declares the shape of the records this recipe emits. File-scoped by default; prefix with `share` to publish to the workspace catalog.
- `input userId: Int` — declares a per-run parameter.
- `step posts { … }` — names an HTTP request whose response becomes addressable as `$posts`.

## Run it

`forage run` takes a recipe header name and resolves it against the
surrounding workspace:

```sh
echo '{"userId":1}' > /tmp/hello-inputs.json
forage run hello --inputs /tmp/hello-inputs.json
```

The CLI parses the recipe, validates it against the type catalog, runs the HTTP graph, and prints the resulting snapshot — a JSON list of every emitted record — to stdout. `--inputs <path>` points at a JSON object of bindings; omit it for recipes without inputs.

::: tip Validation runs first
Unknown types, unbound path variables, missing required fields, and unknown transforms are caught before any HTTP request fires. The error format speaks the DSL's own terms — no stack traces from extraction code.
:::

## From here

- Read the [syntax reference](/docs/syntax) for the full set of constructs: `enum`, `share`-visibility, `auth` strategies, `case` expressions, optional chaining, transforms.
- Read [engines & pagination](/docs/engines) when you need to scrape a paginated API or a JS-rendered SPA.
- Browse the canonical recipes on [hub.foragelang.com](https://hub.foragelang.com) for end-to-end examples that exercise the full DSL.
