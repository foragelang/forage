# Web IDE

`hub.foragelang.com/edit` hosts an in-browser Monaco editor that parses
and validates recipes, runs HTTP-engine recipes against fixtures, and
publishes to the hub — all without installing anything.

The IDE is a VitePress + Vue component backed by `forage-wasm`: the
same Rust parser, validator, and HTTP runner the CLI uses, compiled to
WebAssembly. Switching between the IDE and the CLI doesn't switch
implementations — they're literally the same code.

## What it can do

- **Edit** in Monaco with Forage syntax highlighting, bracket
  matching, comment toggle, and validation markers driven by
  `forage-wasm::parse_and_validate`.
- **Validate live** as you type. The Worker-spawned web worker calls
  the wasm core; errors land in a panel below the editor with line
  numbers + spans.
- **Run HTTP-engine recipes** against any CORS-friendly endpoint via
  the browser's `fetch`. The runner walks the recipe exactly the same
  way the native engine does (auth flavors that don't require
  filesystem state, pagination, pipelines, transforms).
- **Publish** to `api.foragelang.com` after signing in with GitHub.

## What it can't do

- **Browser-engine recipes** — they need a real WebKit + a host-side
  capture pipeline, which a webpage running inside a tab can't host.
  The IDE shows an **Open in Forage Studio** deep link for these.
- **CORS-blocked APIs** — `fetch` from a browser tab respects CORS;
  recipes targeting private APIs that don't whitelist the IDE's origin
  need to run via the CLI or Studio.
- **`auth.session.*`** — sessioned recipes refuse to run in the IDE,
  even when an in-browser fetch could technically succeed. Persisting
  credentials to a browser tab's localStorage isn't a viable model.

## Sign-in

Bearer-token auth, matching the CLI's `FORAGE_HUB_TOKEN`:

1. Go to the Publish panel.
2. Click **Sign in with GitHub**. The OAuth web flow starts.
3. After redirect, the httpOnly cookie identifies you to the hub-api;
   subsequent publishes are owner-checked.

You can also paste a hub API key directly for service automation.

## Why bother with the IDE

- **No install for newcomers.** A would-be recipe author can iterate
  on a recipe end-to-end before touching the CLI or Studio.
- **Hub-side review.** Reviewers can paste a recipe in the IDE and
  watch it parse + validate + run against the recipe's bundled
  fixtures before approving a publish.
- **Tight feedback loop.** Validation runs entirely in the browser; no
  round-trips to a backend.
