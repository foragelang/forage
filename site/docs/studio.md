# Studio (macOS app)

Forage Studio is a SwiftUI macOS app for authoring Forage recipes
interactively. It hosts the same runtime the CLI uses, with a UI on top
that makes the capture / iterate / publish loop fast: edit the source,
record fresh fixtures from a live WKWebView, run live or against
fixtures, inspect the snapshot and diagnostic, then publish to the hub.

Everything the CLI can do, Studio can do. The difference is
ergonomics — instead of `forage capture && forage run && forage publish`
across three shell windows, you do it all from one window with the
results visible side-by-side.

## Install

Until M5 ships signed builds, you build from source:

```sh
git clone https://github.com/foragelang/forage
cd forage
./open-studio.sh
```

The script runs `xcodegen` and opens `Studio.xcodeproj` in Xcode; hit
**⌘R** to build and launch. Studio lives in the `Studio/` subdirectory
of the Forage repo and depends on the local Forage Swift package — no
third-party dependencies.

## Recipe layout

Recipes live under `~/Library/Forage/Recipes/<slug>/`. The directory
matches the CLI's convention so the same recipe can drive both:

```
~/Library/Forage/Recipes/<slug>/
├── recipe.forage           # source
├── fixtures/
│   ├── captures.jsonl      # recorded fetch/XHR exchanges
│   └── inputs.json         # optional inputs map (k → value)
└── snapshots/
    └── <ts>.json           # snapshot archives
```

When you create a new recipe from the sidebar, Studio scaffolds
this layout for you with a minimal template.

## First recipe

1. Launch Studio. The sidebar lists every slug under
   `~/Library/Forage/Recipes/`.
2. Click **+** at the bottom of the sidebar. A new `untitled-N` slug
   appears, selected, with a minimal HTTP-engine template in the
   editor's **Source** tab.
3. Edit the slug folder name on disk (or rename the file path) if you
   want a meaningful name. Studio picks up the change on the next
   sidebar refresh.

The **Source** tab syntax-highlights keywords, type names, strings,
comments, numbers, `$variables`, and operators (`←`, `→`, `|`, `?`).
Cmd-S saves to disk. Cmd-R runs the recipe live; Cmd-Shift-R replays
against fixtures.

Errors from the parser and validator appear in a panel beneath the
editor, debounced ~500ms after each edit.

## Capture

Recipes that talk to JSON APIs (browser-engine and most HTTP-engine
recipes both work this way) are easier to author when you can see the
exact response bodies.

1. Click **Capture** in the editor toolbar (or pick **Recipe → Capture
   from URL** from the menu bar).
2. Type a URL into the address bar in the capture sheet and hit
   Return. A real WKWebView loads the page; every fetch / XHR shows up
   in the right rail as it happens.
3. Tick the captures you want to keep. The default is "keep
   everything"; uncheck rows for ad-network traffic, third-party
   analytics, or anything else that isn't part of the menu.
4. Pick **Append** or **Replace** in the bottom-left, then click
   **Save**. The kept captures land in
   `~/Library/Forage/Recipes/<slug>/fixtures/captures.jsonl` and the
   **Fixtures** tab refreshes.

## Run + replay

Two run modes, both off the editor's toolbar:

- **Run live** uses `URLSessionTransport` (HTTP-engine recipes) or
  `BrowserEngine` with a visible WKWebView (browser-engine recipes).
  Hits the real network at ~1 req/sec with the polite UA the runtime
  uses for the CLI.
- **Run replay** loads the recipe's `fixtures/captures.jsonl` and feeds
  it to `HTTPReplayer` or `BrowserReplayer`. No network involved; the
  recipe runs against frozen response bodies.

When the run finishes, Studio jumps to the **Snapshot** tab. Each
record type from the snapshot shows up as a row in the left list; pick
a type to see a table of records with one column per field.

## Diagnose

The **Diagnostic** tab renders the `DiagnosticReport` from the most
recent run. Sections:

- **Stall reason**: how the run terminated. `settled` / `completed` is
  the happy path; anything else is a clue.
- **Unmet expectations**: `expect { records.where(...) … }` rules from
  the recipe that didn't hold against the produced snapshot.
- **Unfired capture rules** (browser engine): `captures.match` patterns
  that never saw a matching capture during the run. Usually means the
  pattern is wrong or the endpoint changed.
- **Unmatched captures** (browser engine): captures the recipe didn't
  claim. If the recipe is missing a `captures.match` rule for an
  endpoint you care about, look here.
- **Unhandled affordances** (browser engine): pagination-shaped
  buttons / links the engine saw but didn't drive. If the page has a
  "View more" button and the recipe stopped before clicking it, you'll
  see it called out here.

## Publish

The **Publish** tab carries the form for pushing a recipe to the hub:

1. Fill in display name, summary, tags, license. The slug is fixed
   (it's the directory name).
2. Click **Validate** to confirm the recipe parses + passes the
   validator. Any issues show in the output panel.
3. Click **Preview payload** to see the JSON body Studio would
   POST to `api.foragelang.com`.
4. Click **Publish**.

::: warning
M3 ships the publish flow in stub mode. Studio prints "would POST
to …" with the full payload and the API key (redacted), but doesn't
hit the network. **M4 wires this live** — the same code path will then
POST to `$hubURL/v1/packages` with `Authorization: Bearer <key>`.
:::

## Preferences

Open with **Cmd-,**.

- **Hub URL** — the base URL for the hub API. Default is
  `https://api.foragelang.com`; override for self-hosted hubs or local
  dev.
- **API key** — pasted, then saved to the user's login Keychain.
  Stored under the `com.foragelang.Studio` service. Delete with the
  matching button.

## Keyboard shortcuts

| Shortcut | Action |
|---|---|
| Cmd-N | New recipe |
| Cmd-S | Save current recipe |
| Cmd-R | Run live |
| Cmd-Shift-R | Run replay |
| Cmd-K | Capture from URL |
| Cmd-, | Preferences |

## Limitations (v1)

- **Tabs aren't editable from inside the editor.** Renaming a slug is
  done at the filesystem level (rename the folder, click refresh in
  the sidebar).
- **No multi-recipe windows.** Open one slug at a time per Studio
  window. macOS lets you open a second Studio window from the File
  menu if you need it.
