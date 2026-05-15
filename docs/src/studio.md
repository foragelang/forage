# Forage Studio

A Tauri-based macOS / Windows / Linux app for authoring recipes
interactively. Embeds Monaco, drives the same runtime the CLI uses, and
opens visible WebViews for live captures and the M10 interactive
bootstrap.

## Install

Download the latest signed bundle from
[GitHub Releases](https://github.com/foragelang/forage/releases/latest):

- macOS: `.dmg`
- Windows: `.msi`
- Linux: `.deb` or `.AppImage`

For development:

```sh
git clone https://github.com/foragelang/forage
cd forage/packages/studio-ui
npm install
cd ../../apps/studio
cargo tauri dev
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
  combination.
- `_fixtures/<recipe>.jsonl`, `_snapshots/<recipe>.json` — workspace
  data keyed by recipe header name.
- `.forage/` — runtime state owned by the daemon (`daemon.sqlite`,
  per-recipe output stores under `data/<recipe>.sqlite`).

## Sidebar

The sidebar carries the following sections:

- **Workspace header** — root path; click to switch.
- **Runs** — every `Run` row links to the Deployment view; hover-only
  play button triggers an ad-hoc fire.
- **Recipes** — the list of parsed recipes keyed by header name.
  Clicking a row opens the recipe's file with Run / Configure / Deploy
  enabled.
- **Dependencies** — `[deps]` entries from `forage.toml`.
- **Files** — the filesystem tree. Open header-less declarations files
  here.
- **Daemon footer** — running indicator + active-count + version.

Files without a `recipe "..."` header have Run / Configure / Deploy
disabled in the editor — they're declarations files contributing
`share`d types / enums / fns to the workspace catalog.

```
┌──────────────┬───────────────────────────────────────────────┐
│  Sidebar     │  Toolbar (name, dirty, Save / Replay / Run live) │
│              ├───────────────────────────────────────────────┤
│ Runs         │  Source │ Fixtures │ Snapshot │ Diagnostic │ Publish │
│ Recipes      ├───────────────────────────────────────────────┤
│  • hello     │                                               │
│  • zen-leaf  │   <active tab>                                │
│ + New        │                                               │
│ Files        │                                               │
└──────────────┴───────────────────────────────────────────────┘
```

`+ New` scaffolds `<workspace>/untitled-N.forage` with a minimal
template (`recipe "untitled-N" engine http`).

## Tabs

- **Source** — Monaco editor with full Forage syntax highlighting,
  bracket auto-closing, comment toggle, and validation markers from
  the LSP. ⌘S saves and validates.
- **Fixtures** — `_fixtures/<recipe>.jsonl` view for the active recipe.
- **Snapshot** — after a run, records grouped by type; click a type
  to see a table of records with one column per field.
- **Diagnostic** — `stall_reason`, unmet expectations, unfired capture
  rules, unmatched captures, unhandled affordances.
- **Publish** — hub URL, signed-in identity, **Sign in with GitHub**
  (device-code flow runs inline), Preview (dry-run) and Publish buttons.

## Keyboard shortcuts

| Shortcut | Action |
|---|---|
| ⌘N | New recipe |
| ⌘S | Save + validate the current file |
| ⌘R | Run live |
| ⇧⌘R | Run replay |
| ⌘K | Capture from URL |
| ⌘, | Preferences |

Native menu items use the same shortcuts via Tauri's menu API.

## Browser-engine recipes

Click **Run live** on an `engine browser` recipe and Studio opens a
fresh Tauri WebviewWindow at the recipe's `initialURL`, injects the
fetch/XHR shim, scrolls until settle, and routes the collected captures
through the same evaluator the CLI's replay mode uses. The diagnostic
shows up in the **Diagnostic** tab as it would for any other run.

For M10 interactive bootstrap, Studio's WebviewWindow is the visible
window the user solves the challenge in; the resulting session lands at
`~/Library/Forage/Sessions/<recipe>/session.json`.

## Authentication

Studio stores OAuth tokens in the OS keychain under
`com.foragelang.studio` — macOS Keychain, Windows Credential Manager,
or Linux Secret Service via the `keyring` crate (wrapped by
`forage-keychain`). The Publish tab's **Sign in with GitHub** sheet
runs the device-code flow against `api.foragelang.com`, displaying the
user code + verification URL.
