# Docs audit findings

Scope: two doc sources walked on 2026-05-14.

1. **Published site** at https://foragelang.com/docs/ — built from
   `site/docs/` (VitePress), 13 pages walked from sidebar (Overview,
   Getting started, Syntax reference, Engines & pagination, HTML
   extraction, Authenticated sessions, Expectations, Diagnostics,
   Archive & replay, CLI reference, Studio, Hub, Web IDE, Install).
2. **In-repo mdBook** at `docs/src/` (not currently rendered at any
   public URL; sibling doc set) — 31 pages walked per SUMMARY.md.

Verified against repo at main SHA `2fc90972b97fedf8adae587177fc94797f6b95a9`.

Both doc sets describe the same product; the published `site/docs/` set
is a Swift-era artefact that predates the Rust rewrite and now collides
with reality on most pages. The mdBook in `docs/src/` is a newer
Rust-era rewrite and is mostly correct, with localized drift in a
handful of places.

## Verified clean

mdBook source pages that match the current code with no findings I could
isolate:

- `docs/src/introduction.md`
- `docs/src/lang/overview.md`
- `docs/src/lang/inputs-secrets.md`
- `docs/src/lang/captures.md` (modulo `_id` not shown in JSONL example)
- `docs/src/lang/expressions.md`
- `docs/src/lang/transforms.md` (table matches `BUILTIN_TRANSFORMS` at
  `crates/forage-core/src/validate/mod.rs:87` and registry at
  `crates/forage-core/src/eval/transforms.rs:37`)
- `docs/src/lang/expectations.md` (matches `Snapshot::evaluate_one` at
  `crates/forage-core/src/snapshot/mod.rs:99`)
- `docs/src/lang/progress.md` (matches `infer_progress_unit` at
  `crates/forage-core/src/progress.rs:58`)
- `docs/src/runtime/snapshot.md` (modulo missing `_id` field in the JSON
  example — see finding)
- `docs/src/runtime/replay.md`
- `docs/src/contributing/build.md`
- `docs/src/contributing/transforms.md`
- `docs/src/contributing/auth.md`
- `docs/src/contributing/roadmap.md`
- `docs/src/hub/concepts.md` (with one drift — see finding)
- `docs/src/hub/publishing.md`
- `docs/src/hub/auth.md`
- `docs/src/hub/errors.md`
- `docs/src/cookbook/hacker-news.md`

## Findings

### 🔴 Wrong / broken

#### F1. Published site at foragelang.com/docs/ describes a Swift implementation that doesn't exist

The entire `site/docs/` set (deployed at foragelang.com/docs/) is
written against the Swift / SwiftUI implementation that was deleted
recently (commit `9af43ca` "Kill SwiftUI Studio remnant"). The repo
is Rust + Tauri.

Specific pages and the API surfaces they hallucinate:

- `site/docs/index.md:28` — "HTTP engine — drives recipes against
  documented JSON/HTML APIs over `URLSession`." Code uses
  `reqwest::Client` (`crates/forage-http/src/client.rs:53`).
- `site/docs/engines.md:16,27,42-48,56-86,90` — `URLSession`,
  `WKWebView`, `Task.cancel()`, `RunResult`, `@MainActor @Observable`,
  `HTTPProgress`, `BrowserProgress` Swift types in code samples.
  None exist in the codebase; engines run on tokio
  (`crates/forage-http/src/engine.rs:83`) and the browser engine
  drives `wry` (`crates/forage-browser/Cargo.toml:22-23`).
- `site/docs/diagnostics.md:7-15,36-41,73-91` — `DiagnosticReport`
  shown as a Swift `struct` with `stallReason: String` (non-optional),
  `unmatchedCaptures: [UnmatchedCapture]`, `unfiredRules: [String]`.
  Real shape at `crates/forage-core/src/snapshot/mod.rs:145-155` is a
  Rust struct with `stall_reason: Option<RuntimeDiagnostic>`,
  `unmatched_captures: Vec<RuntimeDiagnostic>`,
  `unfired_capture_rules: Vec<RuntimeDiagnostic>`. Field names differ
  (`unfired_capture_rules` vs `unfiredRules`); shape differs
  (`RuntimeDiagnostic` carries `{message, line: Option<u32>}`, not
  `UnmatchedCapture { url, method, status, bodyBytes }`); `body_bytes`
  isn't tracked at all.
- `site/docs/archive-replay.md` — entire page documents an
  `Archive.write(...)` / `Archive.list(...)` / `Archive.read(...)`
  Swift API and a `BrowserReplayer(capturesFile:)` initializer. None
  of this exists in the Rust codebase. The actual replay flow is
  `forage_browser::run_browser_replay(recipe, captures, inputs,
  secrets)` (`crates/forage-browser/src/lib.rs:21`) plus
  `forage run --replay` (`apps/cli/src/main.rs:200-204`); fixtures are
  read from `fixtures/captures.jsonl` next to the recipe
  (`apps/cli/src/main.rs:333`). No archive subsystem exists; no
  `<root>/<slug>/<ISO8601-Z>/` directory shape.
- `site/docs/auth-sessions.md:151-162` — "Caching … `cache: <seconds>`
  persists the resolved session … `chmod 600` … SHA-256 fingerprint …
  mid-run `401`/`403` evicts the cache". The AST carries
  `cache_duration_secs` (`crates/forage-core/src/ast/auth.rs:33-37`)
  but the HTTP engine never reads it. `crates/forage-http/src/auth.rs`
  has zero references to `cache`, no fingerprint hashing, no chmod,
  no AES-GCM. `forage-keychain` is listed as a Cargo dep of
  `forage-http` but never `use`d in any source file. The page lies
  about the feature.
- `site/docs/auth-sessions.md:66` and `site/docs/auth-sessions.md:142`
  — claim "If `items` returns `401`/`403`, the engine drops the
  cached session, re-runs the login, and retries the original
  request once. A second `401`/`403` becomes `stallReason:
  "auth-failed: HTTP 401 after re-auth"`". No 401/403 re-auth logic
  exists; `crates/forage-http/src/auth.rs` and
  `crates/forage-http/src/engine.rs` have no retry-on-401 path and no
  `auth-failed` stall reason is emitted anywhere. The AST's
  `max_reauth_retries: u32` field (default 1) is parsed and ignored.
- `site/docs/auth-sessions.md:126-149` — MFA flow ("the engine
  pauses, asks the host for a code, and re-sends the login with
  `<mfaFieldName>: <code>` appended"). Zero references to MFA / OTP
  in `crates/forage-http/src/`. The AST's `requires_mfa: bool` and
  `mfa_field_name: String` fields (`crates/forage-core/src/ast/auth.rs:38-40`)
  are parsed and ignored. No MFAProvider trait exists.
- `site/docs/auth-sessions.md:107-123` — `cookiePersist` is
  described as a working feature that loads cookies from a file.
  Reality: `crates/forage-http/src/auth.rs:48-53` has the comment
  "Wire in once the use case lands" and returns
  `Ok(AuthState::default())` without doing anything.
- `site/docs/studio.md:1-3,18-27,57-61,68-83,140-148` — Studio is
  described as a "SwiftUI macOS app" built via `xcodegen` + Xcode
  (`./open-studio.sh`) with a Source / Fixtures / Snapshot /
  Diagnostic / Publish tab layout. Real Studio is Tauri + React
  (`apps/studio/src-tauri/Cargo.toml:21`, `apps/studio/ui/src/App.tsx`)
  with an Editor view + Deployment view shell (`apps/studio/ui/src/App.tsx:8-19`).
  Shortcuts `Cmd-K` (Capture from URL) and `Cmd-,` (Preferences)
  documented on `site/docs/studio.md:159-160` do not exist in
  `apps/studio/src-tauri/src/menu.rs`. The Capture flow, the Publish
  tab UI, the Preferences sheet, the macOS Keychain API-key store,
  the `~/Library/Forage/Recipes/<slug>/` layout — none of it lines up
  with the current Tauri Studio.
- `site/docs/web-ide.md:8-12,30-35` — claims the IDE is "a Vue
  component on the hub site backed by a TypeScript reimplementation
  of the Forage parser, validator, and HTTP runner — kept in sync
  with the Swift runtime via a shared set of test vectors at
  `tests/shared-recipes/`". `hub-site/forage-ts/` does exist
  (TypeScript port), but the canonical implementation is Rust
  compiled to wasm (`crates/forage-wasm/`). The roadmap doc
  `docs/src/contributing/roadmap.md:28-29` even calls out that the
  web IDE "still imports `forage-ts` instead of `forage-wasm`; flipping
  the import + benchmarking is a small PR." The published page also
  says `auth.htmlPrime` is unsupported in the IDE; the mdBook
  `docs/src/web-ide.md:34` only excludes `auth.session.*`.
- `site/docs/cli.md:35-41` — claims `forage run` has flags `--mfa` /
  `--no-mfa` and `--input k=v`. Neither exists. Real flags:
  `--replay`, `--output {pretty|json}` (`apps/cli/src/main.rs:42-46`).
- `site/docs/cli.md:67-87` — claims `forage capture <url>` takes
  `--out`, `--settle`, `--timeout` and produces JSONL with fields
  `timestamp`, `kind: "fetch"`, `requestUrl`, `responseUrl`,
  `bodyLength`. Real subcommand at `apps/cli/src/main.rs:57` has no
  arguments and just prints "use Studio for now".
- `site/docs/cli.md:121-170` — `forage scaffold` claimed flags
  `--host SUBSTRING` and `--out PATH`. Real flags at
  `apps/cli/src/main.rs:62-64` are just `--name`. The page also
  describes elaborate inference heuristics (`[Ii]d` regex, content-
  type dispatch, "biggest homogeneous array of objects") that aren't
  in `do_scaffold` (`apps/cli/src/main.rs:392-457`) — actual scaffold
  just groups URLs by stripped-query-string path and emits a single
  `Item` placeholder type.
- `site/docs/cli.md:49-50` — exit codes claimed `0`/`1` only.
  Real exit codes are `0` (clean), `1` (runtime/transport),
  `2` (parse/validate), `3` (unmet expectations)
  (`apps/cli/src/main.rs:221-227`).
- `site/docs/install.md:35-40` — "Forage Studio is an interactive
  recipe authoring app that wraps a WKWebView…" Studio uses `wry`
  (Tauri's webview crate), not WKWebView directly.

Fix: nuke the entire `site/docs/` tree and replace it with whatever
the published doc story should be. If the intent is for foragelang.com
to render the mdBook in `docs/src/`, the VitePress at `site/docs/` is
dead weight; either delete it or remove it from the production deploy.
Pointing the site at the mdBook output instead is the cleanest cut.

#### F2. mdBook `docs/src/lang/imports.md` documents an `import` directive that doesn't exist in the parser

`docs/src/lang/imports.md:5-11` shows recipes with an `import alice/zen-leaf v2`
top-level directive, and `docs/src/lang/imports.md:17-21` documents the
syntax variants (`import <name>`, `import <author>/<name>`, `import
… v<N>`, `import …@v<N>`).

`crates/forage-core/src/parse/token.rs:103-212` lists every reserved
word; `import` is not in it. `crates/forage-core/src/parse/parser.rs:310-342`
is the recipe-body top-level dispatch — there is no `"import"` arm; an
`import` token would surface as `Token::Ident("import")` and trigger
`unexpected token at top level`.

The real import mechanism is the workspace manifest's `[deps]` table
(`crates/forage-core/src/workspace/manifest.rs:30-32`, e.g. `"alice/shared-types" = 3`),
resolved by `forage update` (`apps/cli/src/main.rs:517-562`) and folded
into the catalog by `Workspace::catalog`
(`crates/forage-core/src/workspace/mod.rs:396-401`). Even
`docs/src/hub/publishing.md:1-6` correctly says "the unit of
publication is a workspace". The `import` keyword is a fiction.

Fix: rewrite `docs/src/lang/imports.md` to describe `forage.toml [deps]`
+ `forage update`, mirroring `site/docs/hub.md:99-119` (which actually
explains the workspace mechanism correctly) and
`docs/src/hub/concepts.md:36-47` (which currently has an `import <slug>`
directive that doesn't exist — same fix). Remove the `import alice/zen-leaf v2`
example from `docs/src/lang/overview.md:13`.

#### F3. mdBook `docs/src/runtime/sessions.md` and `docs/src/lang/auth.md` document unimplemented session features

Same issues as F1's `site/docs/auth-sessions.md` finding, but in the
mdBook source:

- `docs/src/runtime/sessions.md:23-46` — describes `cache: <seconds>`
  as a working on-disk session cache at
  `~/Library/Forage/Cache/sessions/<recipe-slug>/<fingerprint>.json`
  with chmod 600 and SHA-256 fingerprinting. Not implemented (see F1).
- `docs/src/runtime/sessions.md:48-53` — AES-GCM encryption via
  `forage-keychain`. `forage-http` doesn't `use forage_keychain` at
  all.
- `docs/src/runtime/sessions.md:54-67` — `SecretRedactor` for
  scrubbing credentials from diagnostics. `grep -rn SecretRedactor`
  in `crates/forage-http/` returns nothing.
- `docs/src/lang/auth.md:60-68` — `cache: 600` in the example. The
  field is parsed (`crates/forage-core/src/ast/auth.rs:35`) but
  ignored by the runtime.
- `docs/src/lang/auth.md:102-123` — MFA section with stdin prompt,
  modal sheet, etc. None of this exists in either CLI
  (`apps/cli/src/main.rs` has no MFA prompt) or
  `crates/forage-http/`.
- `docs/src/lang/auth.md:91-100` and `docs/src/runtime/sessions.md:15-19`
  — `cookiePersist` is described as a working escape hatch. Reality
  at `crates/forage-http/src/auth.rs:48-53`: the `CookiePersist`
  branch returns `AuthState::default()` with the comment "Wire in
  once the use case lands."

Fix: either implement these features (the AST + parser are wired,
runtime is the missing half), or restate the docs as "future work" /
"R10 followup" alongside the `roadmap.md`-style framing. The
sessions.md / auth.md pages currently read as documented behavior.

#### F4. mdBook `docs/src/runtime/interactive.md` documents nonexistent CLI flags

`docs/src/runtime/interactive.md:49`:

```
forage run --interactive recipes/ebay-sold --input query=polaroid+sx-70
```

Neither `--interactive` nor `--input k=v` exist on
`forage run` (`apps/cli/src/main.rs:36-47`). Real flags:
`--replay`, `--output`. Inputs come from `fixtures/inputs.json`
(`apps/cli/src/main.rs:306-320`), not CLI flags.

The same line is repeated for the subsequent-run example at
`docs/src/runtime/interactive.md:70`.

Studio's "Recipe → Bootstrap session…" menu item
(`docs/src/runtime/interactive.md:53`) also does not exist; check
`apps/studio/src-tauri/src/menu.rs:6-64` — only File / Edit / Recipe
(with Run Live / Run Replay / Validate) / View menus are wired. No
Bootstrap session menu item.

Fix: drop the `--interactive` flag and `--input` flag references; show
the `fixtures/inputs.json` path for input setup. State that
interactive bootstrap is roadmap work (M10 is listed as roadmap by the
file title itself but the body reads as shipped).

#### F5. mdBook `docs/src/cookbook/scotus-opinions.md` doesn't match the in-tree recipe

The doc page recipe (`docs/src/cookbook/scotus-opinions.md:8-38`) has:

```forage
type Opinion {
    date:        String
    docket:      String
    caseName:    String
    pdfUrl:      String
    holdingText: String?
}
```

with selector `table#OpinionsTable tbody tr` and date in `td:nth-child(1)`.

The actual recipe at `recipes/scotus-opinions/recipe.forage:13-38`:

```forage
type Opinion {
    date:         String
    docketNumber: String
    caseName:     String
    pdfUrl:       String
    holding:      String?
}
```

with selector `table.table-bordered tr:has(a)` and date in
`td:nth-child(2)`. Field renames: `docket → docketNumber`,
`holdingText → holding`. The doc's `td:nth-child(N)` indices are all
shifted one column compared to the live recipe.

The page also has a misleading run example at
`docs/src/cookbook/scotus-opinions.md:56`:

```sh
FORAGE_SECRET_TERM=24 forage run recipes/scotus-opinions
```

`term` is declared `input term: String`
(`recipes/scotus-opinions/recipe.forage:21`), not a secret. The CLI's
`load_secrets_from_env` (`apps/cli/src/main.rs:322-331`) iterates only
declared `secrets`, so `FORAGE_SECRET_TERM` is silently ignored. The
input comes from `fixtures/inputs.json` per the convention shown lower
on the page.

Fix: regenerate the cookbook page from the live recipe. Drop the
`FORAGE_SECRET_TERM` line.

#### F6. mdBook `docs/src/cookbook/github-releases.md` doesn't match the in-tree recipe

Doc page recipe (`docs/src/cookbook/github-releases.md:7-44`):

```forage
type Release {
    tag:        String
    name:       String?
    publishedAt: String?
    prerelease: Bool
    url:        String
}
…
paginate cursor {
    items:       $.
    cursorPath:  $.<next-from-Link-header>     // (see below)
    cursorParam: "page"
}
```

Live recipe (`recipes/github-releases/recipe.forage`):

```forage
type Release {
    tag:       String
    name:      String?
    published: String
    url:       String
}
```

with no `paginate` block at all, no `prerelease` field, field renamed
from `publishedAt` to `published`. The doc's `cursorPath: $.<next-from-Link-header>`
placeholder is not valid `.forage` syntax — `<next-from-Link-header>`
isn't a path; it's prose. The recipe wouldn't parse if anyone copy-
pasted it.

Fix: either align the doc with the live recipe (one-step, 15-item
fetch) or update the live recipe to match the doc's cursor-pagination
shape. Resolve the inconsistency; don't ship `<placeholder>` as code.

#### F7. mdBook `docs/src/lang/http.md` claims a `maxIterations` pagination ceiling that's actually an engine-config field

`docs/src/lang/http.md:78-79`:

> Each iteration appends the items to the bound step result; the engine
> exits when the strategy says stop or when `maxIterations` (default 500)
> is hit.

There is no `maxIterations` field on a `step` or `paginate` block. The
actual ceiling is `EngineConfig::max_requests` (default 500,
`crates/forage-http/src/engine.rs:28-42`), which is not recipe-
declarable. The pagination loop checks it at
`crates/forage-http/src/engine.rs:308-314` and surfaces it as
`exceeded max_requests (500)` on overflow.

Fix: rename the doc reference to `max_requests` (engine config) and
make clear it's not authorable from the recipe. If the right call is
to expose it per-recipe, that's a code change first.

#### F8. mdBook `docs/src/cli.md` is missing `forage init` and `forage update` subcommands

`docs/src/cli.md:7-15` lists `run`, `test`, `capture`, `scaffold`,
`publish`, `auth`, `lsp`. Missing: `init` (`apps/cli/src/main.rs:66-72`,
drops a `forage.toml` skeleton) and `update`
(`apps/cli/src/main.rs:73-82`, resolves `[deps]` and writes
`forage.lock`). Both are real subcommands the user has to know about
to operate a workspace; they're necessary precursors to anything in
`docs/src/lang/imports.md` / `docs/src/hub/publishing.md`.

Fix: add sections for both, with the dir-positional + flag surface.

#### F9. mdBook `docs/src/cli.md` overstates `forage auth logout --revoke`

`docs/src/cli.md:91`:

> `forage auth logout --revoke  # also POST /v1/oauth/revoke (server-side invalidate)`

`apps/cli/src/main.rs:683-691` parses `--revoke` and ignores it
(`revoke: _`); the inline TODO at line 689 reads
`// TODO(R6 followup): if `revoke`, POST /v1/oauth/revoke.` So the flag
silently does nothing server-side; only the local auth-store file
is deleted.

Fix: either implement the revoke call or document that `--revoke` is
currently a no-op pending R6 follow-up.

#### F10. mdBook `docs/src/studio.md` claims keyboard shortcuts that aren't wired

`docs/src/studio.md:64-71`:

| Shortcut | Action |
|---|---|
| ⌘N | New recipe |
| ⌘S | Save + validate the current recipe |
| ⌘R | Run live |
| ⇧⌘R | Run replay |
| ⌘K | Capture from URL (R9 followup) |
| ⌘, | Preferences |

`apps/studio/src-tauri/src/menu.rs:6-21` only wires `CmdOrCtrl+N`,
`CmdOrCtrl+S`, `CmdOrCtrl+R`, `CmdOrCtrl+Shift+R`, and
`CmdOrCtrl+Shift+V` (for Validate). `⌘K` (Capture from URL) is
explicitly marked as "R9 followup" in the doc table, so it's at least
honest there — but `⌘,` (Preferences) is not in the menu and there
is no Preferences sheet anywhere in `apps/studio/ui/src/`.

The Studio UI shape on `docs/src/studio.md:33-43` describes
Source / Fixtures / Snapshot / Diagnostic / Publish tabs;
`apps/studio/ui/src/App.tsx:8-19` only routes between an
`EditorView` and `DeploymentView`. The "Snapshot" / "Diagnostic"
tabs are folded into the Inspector
(`apps/studio/ui/src/components/Inspector/`), and there is no
"Publish" tab at all in the current UI tree. The "+ New" sidebar item
at `docs/src/studio.md:46-47` likewise doesn't exist as described —
sidebar contents are in `apps/studio/ui/src/components/Sidebar.tsx`.

Fix: regenerate the Studio doc from the current Tauri UI, since the
shell layout, tab structure, and Preferences flow are all stale.

#### F11. mdBook `docs/src/contributing/auth.md` references a `BUILTIN_TRANSFORMS`-style allowlist for auth keywords

`docs/src/contributing/auth.md:56-62` (step 4 — Validation):

> - A `BUILTIN_TRANSFORMS`-style allowlist if it introduces a new pseudo-
>   transform.

This sounds plausible but there's no auth-keyword allowlist parallel
to `BUILTIN_TRANSFORMS`. The lexer's `KEYWORDS` list
(`crates/forage-core/src/parse/token.rs:103-212`) is the single source
of truth; the validator dispatches on AST variants, not strings. The
LSP keyword list lives in `crates/forage-lsp/src/server.rs:166`
(`for t in BUILTIN_TRANSFORMS`) and pulls from the validator's
transforms list, not from any auth allowlist.

The same file at line 68-69 ("Add them to
`crates/forage-lsp/src/server.rs::KEYWORDS`") references a
`KEYWORDS` constant on the LSP side. `crates/forage-lsp/src/server.rs`
does not export a `KEYWORDS` constant; it pulls from
`forage_core::validate::BUILTIN_TRANSFORMS` and otherwise iterates
recipe-derived names.

Fix: drop the validator-allowlist bullet (or rename to the
parser-keyword list at `token.rs`); fix the LSP reference to match
the real source layout.

### 🟡 Stale / drift

#### F12. `docs/src/runtime/snapshot.md` JSON example omits `_id`

`docs/src/runtime/snapshot.md:6-19` shows a snapshot example where
each record is `{ "typeName": "Story", "fields": {…} }`. The real
serialized shape carries an `_id` field on every record
(`crates/forage-core/src/snapshot/mod.rs:27-38`,
`#[serde(rename = "_id")]`), assigned synthetically as `rec-N`. The
Ref<T> story documented at `docs/src/lang/types.md:90-94` depends on
this field being present.

Fix: add `"_id": "rec-0"` etc. to the example so readers see the wire
shape they'll actually get.

#### F13. `docs/src/lang/captures.md` JSONL example omits the `_id` and timestamp fields

`docs/src/lang/captures.md:73-76` shows two capture lines. These
match `forage_replay::Capture` serialization broadly, but the example
omits the `kind` discriminator tag for HTTP captures (the doc shows
`"kind":"http"` and `"kind":"browser"` with `"subkind":"match"`).
This is technically the real shape per `crates/forage-replay/`, but
the example is the only documented place capture-file authors will
look — worth double-checking against `Capture` (`apps/cli/src/main.rs:345`
uses `serde_json::from_str::<Capture>`) so the wire format is fully
explicit, including any optional fields.

Fix: check the actual `Capture` enum's serde representation and
ensure the example shows every field a fixture must carry.

#### F14. `docs/src/lang/types.md` describes Ref<T> as recently landed; not flagged as such

The whole "Typed references — `Ref<T>`" section at
`docs/src/lang/types.md:43-94` landed yesterday via commit `f75a08a`
("feat(lang): typed references (Ref<T>) and emit-binding (as $v)").
That's fine — the docs do match the code — but the section reads
as established behavior. Worth verifying the in-tree recipes have
been migrated to use Ref<T> rather than the prior string-FK pattern
the doc says is "obsoleted". The author may have intended a flag /
note, but right now the doc doesn't single this out.

Note: the user noted "typed-refs landed yesterday; deployed-store
landed today" as expected drift — the deployed-store / daemon (commit
`257d075` "daemon: own deployed versions; drafts stay in Studio") is
**completely** absent from `docs/src/`. The `forage-daemon` crate
(`crates/forage-daemon/src/lib.rs:1-90`) exposes a full scheduling
runtime, deployment store, output store, scheduler, drift derivation,
etc. None of this surfaces in the docs (Studio doc, runtime doc, or
contributing doc). See F19.

#### F15. `docs/src/hub/concepts.md` documents an `import <slug>` directive (same fiction as F2)

`docs/src/hub/concepts.md:36-47`:

```forage
import forage/cannabis     // shared schema
import alice/zen-leaf v2   // a specific dispensary recipe
```

Same root cause as F2 — no `import` keyword. Fix: rewrite to point at
`forage.toml [deps]`. The `site/docs/hub.md:103` page actually does
this correctly: "the unit of distribution is a package … recipes
never write `import` directives" — copy that posture.

#### F16. `docs/src/contributing/transforms.md` references a Studio path that's correct but cross-cuts with F1's deletion

`docs/src/contributing/transforms.md:38-39`:

> - `apps/studio/ui/src/lib/monaco-forage.ts::BUILTIN_TRANSFORMS` — same
>   list for the static Monaco completion items.

That path exists (`apps/studio/ui/src/lib/monaco-forage.ts` is real).
Verified clean — flagging only because the same instructions in
`site/docs/` reference Swift Studio paths that don't exist. The mdBook
version is correct.

#### F17. `docs/src/cli.md` claims `forage publish` is a working publish path; the actual default is dry-run

`docs/src/cli.md:73-81`:

```sh
forage publish recipes/hacker-news                    # dry-run
forage publish recipes/hacker-news --publish          # actually POST
```

This is correct. But the surrounding prose at `cli.md:72-86` doesn't
mention the env-var precedence ordering for the bearer source. The
actual code (`apps/cli/src/main.rs:583-591`) is CLI arg → env →
auth store, which is the correct order — the doc just doesn't show the
env-var name in the example. Minor — readers might miss
`FORAGE_HUB_TOKEN`.

Fix: add an env-var line to the example block; ordering is otherwise
right.

#### F18. mdBook `docs/src/lang/http.md` lists "five auth flavors" but the AST has only three top-level variants

`docs/src/lang/http.md:83-94` claims "five auth flavors": `staticHeader`,
`htmlPrime`, `session.formLogin`, `session.bearerLogin`,
`session.cookiePersist`. The `AuthStrategy` enum
(`crates/forage-core/src/ast/auth.rs:9-19`) has three variants:
`StaticHeader`, `HtmlPrime`, `Session(SessionAuth)`. `SessionAuth`
then has a `SessionKind` sub-enum
(`crates/forage-core/src/ast/auth.rs:56-60`) with three variants.

This is mostly nomenclature — recipes do write the five forms at the
syntax level — but the doc could be tighter. More important: as noted
in F1/F3, the three session variants don't fully work; the doc reads
them as equally supported.

Fix: split into "two header/regex flavors (live), three session
flavors (parsed; partial runtime — cache / MFA / 401-reauth / cookie-
persist are not implemented)." Then file follow-up issues for the
real gaps so the doc can claim them later.

### 🔵 Missing coverage

#### F19. The `forage-daemon` crate is invisible in mdBook

`crates/forage-daemon/` is a real crate (`crates/forage-daemon/src/lib.rs:22-29`
declares modules `db`, `deployments`, `error`, `health`, `model`,
`output`, `run`, `scheduler`). It exposes a full scheduling runtime:
`Run`, `RunConfig`, `Cadence` (Interval/Cron/Manual)
(`crates/forage-daemon/src/model.rs:34-65`), `DeployedVersion`,
`Outcome`, `Trigger`, drift-based `Health` derivation
(`crates/forage-daemon/src/lib.rs:53-56`), an output store with
schemas + records (`crates/forage-daemon/src/lib.rs:62-63`), and a
scheduler with `next_fire_for` / `advance_next_run` / cron validation
(`crates/forage-daemon/src/lib.rs:68-69`).

None of this is documented anywhere in `docs/src/`. The Studio
doc (`docs/src/studio.md`) doesn't mention scheduled runs, cadence,
deployments, drift health, or output stores. The runtime section
(`docs/src/runtime/*.md`) only covers snapshot / sessions / replay /
interactive.

The user explicitly called this out as "recently landed":
"daemon's deployed-store + deploy operation (commit `257d075` and
follow-ups)." Treating as expected drift — but flagging the missing
surface for the next docs pass.

Fix: add `docs/src/runtime/daemon.md` (or `studio.md` companion) that
covers the deploy flow, cadence, scheduled runs, output store, and
drift health. Currently a recipe author has zero documented path to
understand Run history / scheduled execution.

#### F20. The `forage-lsp` crate's full capability list is sketched but undertested in docs

`docs/src/lsp.md:9-28` lists the LSP capabilities at a high level
(`didOpen` / `didChange`, `publishDiagnostics`, `completion`, `hover`,
`documentSymbol`, advertised-but-not-resolved `definition`). The
actual implementation at `crates/forage-lsp/src/server.rs`
(166 lines) and `crates/forage-lsp/src/intel.rs` (which sources the
transform list at line 15) is broader — there's an
`intel` module for completion/hover sourcing. Hover behavior in
particular is undertested by the doc — what does the LSP actually
return for `$input.X` vs. `$secret.X`?

Fix: minor — flesh out hover-text examples and explicitly note that
`definition` is unimplemented (the doc says "advertised; resolution
lands when validator-side spans land (R7 followup)" which is fair,
but the actual response is "no result" rather than "404"; readers
trying it will be confused).

#### F21. The `forage-replay` crate's `Capture` enum isn't documented in full

`docs/src/lang/captures.md:73-76` and `docs/src/runtime/replay.md:32-36`
show JSONL examples but don't enumerate the `Capture` discriminated
union (`crates/forage-replay/`). Notable: `Capture::Browser` has
both `Match` and `Document` variants
(`apps/cli/src/main.rs:409-421` switches on them); the `Match`
variant has a different shape than a `Document` variant.

Fix: add a small reference table in `docs/src/runtime/replay.md`
showing the three concrete capture shapes (HTTP, browser-match,
browser-document) with their fields.

#### F22. `forage test` + workspace catalog interaction isn't covered

`docs/src/cli.md:39-50` covers `forage test` but doesn't say how it
interacts with a workspace. Real behavior: `test` uses
`load_recipe(recipe_dir)` (`apps/cli/src/main.rs:271-273`) which
chains through `build_catalog_for` (`apps/cli/src/main.rs:294-304`).
If the recipe dir sits inside a `forage.toml` workspace, the catalog
includes hub-dep declarations files plus workspace declarations.
This matters for users who put recipes inside workspaces.

Fix: one sentence in `docs/src/cli.md` (or a cross-reference to
`docs/src/lang/imports.md` once that's fixed per F2).

### 💭 Inconsistencies

#### F23. mdBook says `import` is a directive; mdBook (separately) and site say it's a workspace dep

`docs/src/lang/imports.md` (recipe `import` directive) contradicts
`docs/src/hub/publishing.md:3-6` ("the unit of publication is a
**package**: a workspace's `forage.toml` plus every `.forage` file …")
and `site/docs/hub.md:99-119` ("Sharing types is a workspace-level
concern — recipes never write `import` directives"). The mdBook is
self-contradictory; reconcile via F2's fix.

#### F24. `docs/src/lang/transforms.md` says transforms ship "with both the CLI and the wasm core"; `web-ide.md` says the IDE uses `forage-ts` (TS port)

`docs/src/lang/transforms.md:1-7`:

> They live in `forage-core::eval::transforms` and ship with both the
> CLI and the wasm core, so the same names work in every host.

`docs/src/web-ide.md:6-12`:

> The IDE is a VitePress + Vue component backed by `forage-wasm`: the
> same Rust parser, validator, and HTTP runner the CLI uses, compiled
> to WebAssembly.

But `docs/src/contributing/roadmap.md:28-29`:

> The web IDE's `RecipeIDE.vue` still imports `forage-ts` instead of
> `forage-wasm`; flipping the import + benchmarking is a small PR.

So one doc says the IDE uses wasm-compiled Rust transforms, another
says the IDE imports a separate TS port. The transforms doc's claim
about "wasm core" is aspirational, not current. Either fix the
roadmap statement (now flipped) or fix transforms.md / web-ide.md
to say "currently the IDE re-implements; wasm wiring is roadmap."

#### F25. `docs/src/lang/captures.md` says replay/live "bit-for-bit equivalent"; runtime says replay isn't a substitute

`docs/src/lang/captures.md:78-82`:

> Replay and live runs are bit-for-bit equivalent against the same
> capture stream; the only difference is where the captures come from.

`site/docs/archive-replay.md:94-96`:

> Replay isn't a substitute for end-to-end live runs — it just gives
> you a fast iteration cycle.

Both can be true (bit-for-bit equivalent given the same fixture vs.
not equivalent vs. live data), but the wording in `captures.md` reads
overly strong. A reader could conclude "fixtures = production" and
skip live runs.

Fix: tighten `captures.md` to "bit-for-bit equivalent given the same
fixture stream; fixtures lag live data, so replay isn't a substitute
for periodic live verification."

#### F26. `docs/src/runtime/sessions.md` calls Linux cache `$XDG_CACHE_HOME`; the workspace cache path comment uses `~/Library/Forage/`

`docs/src/runtime/sessions.md:34-36`:

> The cache lands at
> `~/Library/Forage/Cache/sessions/<recipe-slug>/<fingerprint>.json`
> (`$XDG_CACHE_HOME` on Linux, `%LOCALAPPDATA%` on Windows).

The workspace cache code (`crates/forage-core/src/workspace/mod.rs:475`)
uses `~/Library/Forage/Cache/hub/…`. Since the session cache isn't
implemented (F3), this is moot, but the path convention should match.
For documentation consistency: pick one OS-specific story (the dirs
crate's `dirs::cache_dir()` or a Forage-specific layout) and use it
across all four cache-location references in mdBook (session cache,
hub cache, sessions/, deployments/).

## Summary

The published docs at foragelang.com/docs/ (`site/docs/`) are
substantively wrong — they document a Swift / SwiftUI / WKWebView
implementation that the repo has deleted. Twelve of the thirteen
public pages reference Swift APIs (`URLSession`, `WKWebView`,
`Task.cancel()`, `Archive.write(...)`, `BrowserReplayer`,
`MFAProvider`, etc.) that don't exist anywhere in the current code.
The cleanest fix is to delete `site/docs/` and either point the
deployed site at the mdBook output in `docs/src/` or rebuild the
public surface from the mdBook content. Auditing line-by-line is
work, and the result is still a Swift-era doc.

The in-repo mdBook at `docs/src/` is mostly correct against the
Rust implementation but has localized issues: an `import` keyword
fiction (F2, F15, F23), session-auth features documented as live but
not wired (F3 — `cache`, MFA, `cookiePersist`, 401 reauth), an
`--interactive` and `--input` CLI flag fiction (F4), cookbook recipes
that don't match the live `.forage` files (F5, F6), a stale Studio doc
that describes a tab layout the Tauri UI never had (F10), and complete
silence on the `forage-daemon` crate (F19) and the recently landed
deployment / scheduling runtime. Fixing the mdBook is tractable —
mostly edits, plus implementing or downgrading the unwired auth
features.

Net: the mdBook is salvageable with a focused pass; the published
VitePress site is not. Treat them as two separate doc remediation
projects.
