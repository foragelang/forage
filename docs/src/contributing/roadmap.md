# Roadmap

The canonical roadmap lives in
[`ROADMAP.md`](../../../ROADMAP.md) at the repo root. It tracks the
Rust rewrite + product completion across R1–R13:

- **R1** — `forage-core` (AST, parser, validator, evaluator,
  transforms, snapshot).
- **R2** — `forage-http` (live + replay engine, auth flavors,
  pagination, session cache).
- **R3** — CLI run/test wiring.
- **R4** — `forage-browser` (wry-based, captures, M10 interactive).
- **R5** — CLI capture/scaffold.
- **R6** — `forage-hub` + `forage-keychain` + auth.
- **R7** — `forage-lsp`.
- **R8** — `forage-wasm` + hub-site swap.
- **R9** — Forage Studio (Tauri rewrite).
- **R10** — hub-api polish (errors, OAuth, rate limit).
- **R11** — release pipeline.
- **R12** — cross-platform verification.
- **R13** — this docs site.

Every milestone listed there has either a complete landing or a
documented first cut shipped to `main`. The remaining work tends to
be polish:

- The hub IDE's "Run" button no longer executes recipes in-browser;
  the Rust HTTP engine doesn't compile to WASM in the current build,
  so execution lives in Studio. Either move the engine to a
  WASM-friendly transport or accept the split.
- The LSP advertises `definition` but doesn't yet resolve — depends
  on threading spans through the validator.
- The Studio Capture sheet (live recording from the embedded WebView)
  is wired on the backend; the frontend modal lands next.
- Apple Developer ID secrets + the GitHub OAuth App registration are
  manual user steps the release pipeline depends on.

## How to propose work

1. Open an issue or a draft PR with a sketch.
2. Reference the closest existing roadmap milestone (or propose a new
   one).
3. Keep PRs scoped to one milestone increment when reasonable — the
   release pipeline already handles partial milestones.

## Out-of-scope (today)

- **Recipe-author-defined transforms.** The closed registry is a
  validation feature, not an oversight; we'd lose
  `UnknownTransform` errors at parse time. If a real use case
  appears we'd reopen this.
- **Generic scraping framework.** Output types are recipe-declared,
  not user-defined. Designed to be liftable later, not yet lifted.
- **Defeating real anti-bot systems.** The runtime stays an honest
  browser engine. M10 hands off to a person when sites require it.
