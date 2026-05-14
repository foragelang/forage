# Web IDE

The Forage Hub ships a browser-based recipe editor at
[hub.foragelang.com/edit](https://hub.foragelang.com/edit). It lets you
author, validate, run, and publish HTTP-engine recipes without installing
anything.

The IDE is a Vue component on the hub site backed by a TypeScript
reimplementation of the Forage parser, validator, and HTTP runner — kept in
sync with the Swift runtime via a shared set of test vectors at
`tests/shared-recipes/`. The IDE is a peer of the CLI and Studio, not a
replacement.

## What it can do

- **Edit** in a Monaco-powered editor with Forage syntax highlighting.
- **Validate live** — as you type, the parser+validator runs in-browser and
  surfaces errors inline (Monaco markers + a Validation panel).
- **Run HTTP-engine recipes** against any CORS-friendly endpoint, using the
  browser's `fetch`. The runner walks the recipe exactly the same way the
  Swift `HTTPEngine` does (pagination, case-of, pipelines, transforms).
- **Publish** to `api.foragelang.com` with a bearer token (stored in
  localStorage when you check "remember me").

## What it can't do

- **Browser-engine recipes** (anything with `engine browser` + `captures.match`)
  can't run in the web IDE — they need a real `WKWebView`, which only
  Studio has. The IDE shows an "Open in Studio" deep link for these.
- **CORS-blocked APIs** can't be hit from the browser. If the recipe targets
  a private API that doesn't set CORS headers for `hub.foragelang.com`, use
  Studio or the CLI instead. The IDE does not proxy requests.
- **`auth.htmlPrime`** is not implemented in the web runner. Use
  `auth.staticHeader` recipes in the IDE; HTML-priming flows belong in
  Studio.

## Sign-in flow

The IDE uses bearer-token auth, matching the CLI's `FORAGE_HUB_TOKEN`. Get a
token from the hub admin, paste it into the IDE's Publish tab, and check
"remember me" to persist it in `localStorage`.

## Single source of truth

The web IDE consumes the Rust core compiled to WebAssembly through
`hub-site/forage-wasm/adapter.ts`. There is no parallel TypeScript
implementation. `Tests/shared-recipes/` is still the contract for any
future implementation (a reborn TS port, a Python port, etc.); today
the only consumer is the Rust core that powers the wasm bundle.
