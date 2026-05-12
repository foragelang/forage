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
cd forage/apps/studio/ui
npm install
cd ..
cargo tauri dev
```

`cargo tauri dev` boots Vite on `:5173` and embeds it in a Tauri
WebView with hot reload for the React layer and `cargo` rebuilds for
the Rust backend.

## Layout

```
┌──────────────┬───────────────────────────────────────────────┐
│  Sidebar     │  Toolbar (slug, dirty, Save / Replay / Run live) │
│              ├───────────────────────────────────────────────┤
│  • slug-1    │  Source │ Fixtures │ Snapshot │ Diagnostic │ Publish │
│  • slug-2    ├───────────────────────────────────────────────┤
│  + New       │                                               │
│              │   <active tab>                                │
│              │                                               │
└──────────────┴───────────────────────────────────────────────┘
```

Sidebar lists every recipe under `~/Library/Forage/Recipes/<slug>/`.
`+ New` scaffolds a new `untitled-N` directory with a minimal template.

## Tabs

- **Source** — Monaco editor with full Forage syntax highlighting,
  bracket auto-closing, comment toggle, and validation markers from
  the LSP. ⌘S saves and validates.
- **Fixtures** — per-recipe `inputs.json` + `captures.jsonl` view.
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
| ⌘S | Save + validate the current recipe |
| ⌘R | Run live |
| ⇧⌘R | Run replay |
| ⌘K | Capture from URL (R9 followup) |
| ⌘, | Preferences |

Native menu items use the same shortcuts via Tauri's menu API.

## Recipe library

Studio reads from `~/Library/Forage/Recipes/<slug>/` on macOS,
`$XDG_DATA_HOME/Forage/Recipes/<slug>/` on Linux, and
`%APPDATA%\Forage\Recipes\<slug>\` on Windows. The CLI shares this
directory — recipes you `forage scaffold` from the command line show
up in Studio on the next sidebar refresh.

## Browser-engine recipes

Click **Run live** on a `engine browser` recipe and Studio opens a
fresh Tauri WebviewWindow at the recipe's `initialURL`, injects the
fetch/XHR shim, scrolls until settle, and routes the collected captures
through the same evaluator the CLI's replay mode uses. The diagnostic
shows up in the **Diagnostic** tab as it would for any other run.

For M10 interactive bootstrap, Studio's WebviewWindow is the visible
window the user solves the challenge in; the resulting session lands at
`~/Library/Forage/Sessions/<slug>/session.json`.

## Authentication

Studio stores OAuth tokens in the macOS Keychain under
`com.foragelang.studio`; cross-platform via the `keyring` crate
(`forage-keychain` wraps it). The Publish tab's **Sign in with GitHub**
sheet runs the device-code flow against `api.foragelang.com`,
displaying the user code + verification URL.
