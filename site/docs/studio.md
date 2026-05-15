# Forage Studio

Forage Studio is the Tauri-based desktop app for authoring `.forage`
recipes interactively. It hosts the same runtime the CLI uses, embeds
Monaco for editing, and ships an in-process daemon that schedules and
persists production runs. The capture / iterate / publish loop happens
in one window with the results visible side-by-side.

Everything the CLI can do, Studio can do. The difference is
ergonomics — instead of `forage record && forage run && forage publish`
across three shell windows, you do it all from one.

## Install

Download the latest signed bundle from
[GitHub Releases](https://github.com/foragelang/forage/releases/latest):
`.dmg` (macOS), `.msi` (Windows), `.deb` / `.AppImage` (Linux).

For development:

```sh
git clone https://github.com/foragelang/forage
cd forage/packages/studio-ui && npm install
cd ../../apps/studio && cargo tauri dev
```

`cargo tauri dev` boots Vite on `:5173` and embeds it in a Tauri
WebView with hot reload for the React layer and `cargo` rebuilds for
the Rust backend.

## Workspace

Studio operates on exactly one workspace at a time — `~/Library/Forage/Recipes/`
by default, overridable via `FORAGE_WORKSPACE_ROOT`. A workspace is the
directory marked by `forage.toml`; Studio drops an empty manifest on
first launch.

Workspace contents:

- `forage.toml` — name + `[deps]` table for hub packages.
- `*.forage` at any depth — source files. A file may carry a recipe
  header, `share`d declarations, file-scoped declarations, or any
  combination. File position is organizational, not load-bearing.
- `_fixtures/<recipe>.jsonl` / `_snapshots/<recipe>.json` — workspace
  data keyed by recipe header name.
- `.forage/` — runtime state owned by the daemon (`daemon.sqlite`,
  per-recipe output stores under `data/<recipe>.sqlite`).

## Sidebar

The sidebar carries:

- **Workspace header** — root path, click to switch.
- **Runs** — every `Run` row links to the Deployment view; hover-only
  play button triggers an ad-hoc fire.
- **Recipes** — the list of recipes parsed from the workspace, keyed by
  header name. Clicking a row opens the file in the editor with the
  recipe's deployment surfaces (Run / Configure / Deploy) enabled.
- **Dependencies** — `[deps]` entries from `forage.toml`.
- **Files** — the filesystem tree. Lets you open header-less
  declarations files alongside header-bearing recipes.
- **Daemon footer** — running indicator + active-count + version.

A file's place in the workspace determines whether the editor enables
recipe-scoped affordances:

- **Header-bearing file** — Run / Configure / Deploy enabled; the
  active-recipe-name field tracks the parsed header.
- **Header-less file** — affordances disabled. The file is a
  declarations file contributing `share`d types / enums / fns to the
  workspace catalog.

## Editing

- **Source** — Monaco editor with Forage syntax highlighting,
  bracket auto-closing, comment toggle, and validation markers from
  the LSP. ⌘S saves and validates. The editor surface is path-based —
  files open by workspace-relative path, just like a normal editor.
- **Capture** (toolbar) — opens a fresh Tauri WebView at a URL of your
  choice; records every fetch/XHR exchange. Saved captures land in
  `_fixtures/<recipe>.jsonl` for the active recipe.

Errors from the parser and validator appear in a panel beneath the
editor, debounced ~500ms after each edit.

## Running

Two run modes from the editor's toolbar:

- **Run live** — uses the live HTTP transport (HTTP-engine recipes) or
  a visible WebView (browser-engine recipes). The polite UA and ~1
  req/sec rate-limit apply.
- **Run replay** — feeds `_fixtures/<recipe>.jsonl` through the same
  evaluator. No network involved.

When the run finishes, the Snapshot panel populates with the produced
records grouped by type.

The Deployment view tracks scheduled runs over time: a sparkline of
recent runs by health (`Ok` / `Drift` / `Failed`), the schedule editor
(Cron / Interval / Manual cadence), per-step run stats. The daemon
runs in-process, so closing Studio also stops scheduled fires.

## Diagnostic

The **Diagnostic** tab renders the `DiagnosticReport` from the most
recent run. Sections:

- **Stall reason** — how the run terminated. `settled` / `completed` is
  the happy path; anything else is a clue.
- **Unmet expectations** — `expect { records.where(...) … }` rules from
  the recipe that didn't hold against the produced snapshot.
- **Unfired capture rules** (browser engine) — `captures.match`
  patterns that never matched a capture.
- **Unmatched captures** (browser engine) — captures the recipe didn't
  claim. Useful when the recipe is missing a `captures.match` rule for
  an endpoint you care about.
- **Unhandled affordances** (browser engine) — pagination-shaped
  buttons / links the engine saw but didn't drive.

## Publishing

The **Publish** flow carries the form for pushing a recipe to the hub:

1. Fill in display name, summary, tags, license. The hub-side slug is
   fixed by the recipe's header name.
2. Click **Validate** to confirm the recipe parses + passes the
   validator.
3. Click **Preview payload** to see the JSON body Studio would
   POST to `api.foragelang.com`.
4. Click **Publish**.

Studio stores OAuth tokens in the OS keychain (macOS Keychain on
Mac, Windows Credential Manager on Windows, Secret Service on
Linux) under `com.foragelang.studio`. The **Sign in with GitHub**
button runs the device-code flow against `api.foragelang.com`.

## Keyboard shortcuts

| Shortcut | Action |
|---|---|
| Cmd-N | New recipe |
| Cmd-S | Save current file |
| Cmd-R | Run live |
| Cmd-Shift-R | Run replay |
| Cmd-, | Preferences |

Native menu items use the same shortcuts via Tauri's menu API.
