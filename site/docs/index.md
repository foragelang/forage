# Overview

Forage is a declarative DSL for web scraping. You write a recipe; the engine runs it.

## The big idea

Most scrapers are bespoke code: a script per site, each one a snowflake with its own pagination loop, retry logic, and parsing quirks. That's expressive — and impossible to review, audit, or hand off to anyone who didn't write it.

A Forage recipe is pure data. It declares an HTTP graph, a pagination strategy, and how each response binds into typed records. The engine is the only thing that runs HTTP, parses responses, applies transforms, or produces output. That trade — narrower expression for a much smaller trusted surface — is the entire design.

## What's in a recipe

Every recipe has four parts. None of them are optional in spirit, though small recipes will use light versions of each:

| Part                              | Purpose                                                                            |
| --------------------------------- | ---------------------------------------------------------------------------------- |
| `engine`                          | Pick `http` for documented APIs, `browser` for JS-rendered SPAs.                   |
| `type` / `enum`                   | Declare the shape of records the recipe will emit.                                 |
| `input` / `auth`                  | Per-run parameters and authentication strategy.                                    |
| `step` + `for` + `emit`           | The HTTP graph, iteration over responses, and binding fields to extraction expressions. |

The [syntax reference](/docs/syntax) walks through each. If you'd rather see one running first, start with the [quickstart](/docs/getting-started).

## Two engines, one DSL

Forage ships two engines that share the same recipe shape:

- **HTTP engine** — drives recipes against documented JSON/HTML APIs over `URLSession`. Cheap and fast. The right choice when the site exposes its data through requests a client can replay.
- **Browser engine** — drives recipes against a real `WKWebView` on macOS, capturing in-flight requests the page makes. The right choice when the data sits behind a JS SPA, bot-management gates, or auth flows that demand a real browser.

Both engines target the same record types. Downstream code doesn't care which engine ran. See [Engines & pagination](/docs/engines) for the full picture.

## Test, replay, refresh

A recipe lives in a directory: the `.forage` file, a folder of captured HTTP fixtures, and a snapshot of the records the recipe is expected to produce. Three modes share one execution path:

- **Replay** — HTTP is served from fixtures. Sub-second tests, no network.
- **Record** — engine hits the live site, overwrites fixtures with fresh responses, then re-runs in replay. The one-command repair flow when a site changes shape.
- **Live** — production. Engine hits the live site, records flow to wherever the consumer wires them.

## Out of scope

What Forage *doesn't* do, by design:

- **Substantive access controls.** Recipes don't bypass login walls, paywalls, real CAPTCHAs, or account-required pages. Generic bot-management gates on otherwise-public pages are cleared by the browser engine and are not in this category.
- **Recipes-as-code.** Recipes can't run arbitrary expressions. Transforms come from a fixed vocabulary; pagination from a named set of strategies. When a recipe needs something the DSL can't express, the fix is to extend the engine in Swift — not to leak a new escape hatch into recipes.
- **Generic-purpose scraping framework.** Output types are currently scoped to the host consumer's schema. Designed to be lifted later, not yet lifted.
