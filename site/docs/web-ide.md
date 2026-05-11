# Web IDE

The Forage Hub ships a browser-based recipe editor at
[hub.foragelang.com/edit](https://hub.foragelang.com/edit). It lets you
author, validate, run, and publish HTTP-engine recipes without installing
anything.

The IDE is a Vue component on the hub site backed by a TypeScript
reimplementation of the Forage parser, validator, and HTTP runner — kept in
sync with the Swift runtime via a shared set of test vectors at
`tests/shared-recipes/`. The IDE is a peer of the CLI and Toolkit, not a
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
  can't run in the web IDE — they need a real `WKWebView`, which only the
  Toolkit has. The IDE shows an "Open in Toolkit" deep link for these.
- **CORS-blocked APIs** can't be hit from the browser. If the recipe targets
  a private API that doesn't set CORS headers for `hub.foragelang.com`, use
  the Toolkit or CLI instead. The IDE does not proxy requests.
- **`auth.htmlPrime`** is not implemented in the web runner. Use
  `auth.staticHeader` recipes in the IDE; HTML-priming flows belong in the
  Toolkit.

## Sign-in flow

The IDE uses bearer-token auth, matching the CLI's `FORAGE_HUB_TOKEN`. Get a
token from the hub admin, paste it into the IDE's Publish tab, and check
"remember me" to persist it in `localStorage`.

## Sources of drift

The web IDE's parser+validator is a TypeScript port at
[`hub-site/forage-ts/`](https://github.com/foragelang/forage/tree/main/hub-site/forage-ts).
Both implementations share `tests/shared-recipes/` and assert against a single
`expected.json`. If a grammar or rule change in one implementation isn't
mirrored in the other, the corresponding test fails first.

If you're working on the runtime: any change that adds a new parser
production, validator rule, or transform should also land in the TS port.
If the work is large and the port can wait, leave a TODO referencing the
relevant test vector and a feature flag in the IDE.
