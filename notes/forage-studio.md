# Forage Studio architecture

Studio is the desktop authoring app for `.forage` recipes. It hosts an editor +
debugger, owns an in-process daemon that schedules and persists production runs,
and serves both surfaces from one Tauri binary.

## Workspaces

Studio operates on exactly one workspace at a time, rooted at
`workspace_root()` (default `~/Library/Forage/Recipes`; overridable with
`FORAGE_WORKSPACE_ROOT`). A workspace is a directory marked by a
`forage.toml` manifest. Studio drops an empty manifest on first launch so an
existing user folder silently becomes a workspace — no migration.

Workspace contents:

- `forage.toml` — name + `[deps]` table for hub packages.
- `<slug>/recipe.forage` — one recipe per directory.
- `<slug>/fixtures/*.json`, `<slug>/snapshot.json` — fixtures and snapshots
  for the recipe.
- `*.forage` at the root — header-less declarations files. They contribute
  shared `type` / `enum` definitions to every recipe in the workspace.
- `.forage/` — runtime state owned by the daemon (DB, output stores).

The workspace is loaded once at `Daemon::open`. Studio doesn't keep its own
copy — every command path reads through `Daemon::workspace()` so the two views
can't drift. `Daemon::refresh_workspace()` re-reads from disk; Studio invokes
it on filesystem events.

A `TypeCatalog` for a recipe is the merge of: every workspace-level
declarations file + every cached hub-dep declarations file + the recipe's own
local types (which override on collision). Cross-declarations-file name
collisions are a hard error.

## Daemon

In-process, per-workspace, lives under `<workspace>/.forage/`:

- `daemon.sqlite` — `runs` (one row per recipe, with cadence + health) and
  `scheduled_runs` (one row per execution, with outcome, duration, per-type
  counts, stall reason).
- `data/<slug>.sqlite` — the output store. Tables are derived from the
  recipe's emit catalog (`type Foo` → table `Foo`). Every row carries
  `_scheduled_run_id` and `_emitted_at`.

The scheduler is one tokio task that holds a min-heap of next-fire times.
Cadence is `Manual`, `Interval { every_n, unit }`, or `Cron { expr }`.
`configure_run` and `remove_run` wake the loop via a `Notify` so the next-fire
computation always reflects the latest config.

`run_once` is the execution path: build catalog, parse + validate, pick the
engine (HTTP or browser), run, write emitted rows in one transaction, persist
the `ScheduledRun`, recompute `Run.health` (drift rule below), fire the host
`run-completed` callback.

Drift rule: a `Run` is in `Health::Drift` when the latest run is `Ok` but at
least one record type's emit count fell ≤70% of its median across the prior
7 `Ok` runs (window constant: `PRIOR_WINDOW`). Fewer than 3 prior `Ok` runs
yields `Health::Ok` regardless — no signal yet.

The daemon is engine-agnostic for browser recipes: it doesn't ship a `wry`
driver. Studio plugs `StudioLiveBrowserDriver` in via `set_browser_driver`,
so the daemon's scheduler can run `engine browser` recipes against Studio's
WebView. A daemon without a registered driver fails browser runs at
`run_once` time and is otherwise fine for HTTP.

`DaemonStatus { running, version, started_at, active_count }` — `running`
mirrors the scheduler task's liveness; `active_count` is enabled,
non-manual `Run`s.

## Tauri command surface

Studio's frontend talks to the backend through `#[tauri::command]` handlers
in `apps/studio/src-tauri/src/commands.rs`. The surface is **path-based**:
the editor opens / saves files by their workspace-relative path, and the
sidebar walks a filesystem tree rather than asking for a flat recipe list.

Files:

- `current_workspace` — manifest summary (root + name + deps).
- `list_workspace_files` — recursive tree of `FileNode { kind, ... }`.
- `load_file(path)` / `save_file(path, source)` — both path-traversal-checked
  against the workspace root (canonicalized).

Recipe-scoped:

- `create_recipe()` — scaffolds `<workspace>/untitled-N/recipe.forage`.
- `delete_recipe(slug)` — refuses anything that isn't a single segment under
  the workspace root.
- `validate_recipe(source)` — debounced live validation off the in-memory
  buffer. `save_file` also re-validates after writing.
- `run_recipe(slug, replay)` — dev runs. Spawns the engine with progress +
  debugger sinks; on success, calls `daemon.ensure_run(slug)` so the recipe
  shows up in the Runs sidebar.
- `cancel_run`, `debug_resume`, `set_breakpoints`, `set_recipe_breakpoints`,
  `load_recipe_breakpoints`, `set_pause_iterations` — debugger plumbing.
- `recipe_outline`, `recipe_hover`, `language_dictionary` — parser-driven
  outline (step spans), hover info, keyword/transform inventory. The hover
  and language-dictionary commands proxy `forage-lsp::intel` so Studio and
  the LSP read the same canonical lists.

Daemon-scoped:

- `daemon_status`, `list_runs`, `get_run`, `configure_run`, `remove_run`,
  `trigger_run`, `list_scheduled_runs`, `load_run_records`.
- `validate_cron_expr` — the schedule editor's Save gate uses this so the
  daemon and the client agree on what counts as valid syntax.

Auth + publishing:

- `publish_recipe(slug, hub_url, dry_run)` — single-recipe hub publish. The
  full workspace publish path is the CLI's `forage publish`.
- `auth_whoami`, `auth_start_device_flow`, `auth_poll_device`, `auth_logout`.

Cross-boundary types (`ValidationOutcome`, `RunOutcome`, `Diagnostic`,
`PausePayload`, `Run`, `ScheduledRun`, `Cadence`, `Health`, `DaemonStatus`,
`FileNode`, …) are defined in Rust with `serde` + `ts-rs`. The generated
`.ts` files under `apps/studio/ui/src/bindings/` are the source of truth on
the wire; the TS side imports from there rather than redefining shapes.

## Frontend shell

The view is a two-mode router:

- `view = 'editor'` — `EditorView` (toolbar, editor pane, inspector,
  debugger panel when paused).
- `view = 'deployment'` — `DeploymentView` (run header, schedule editor,
  trends, run log, run drawer).

Sidebar contents:

- Workspace header (path, click-to-switch placeholder).
- Runs section — every `Run` row links to the Deployment view; hover-only
  play button triggers an ad-hoc fire.
- Dependencies section — `[deps]` entries from `forage.toml`.
- Files section — workspace tree, click-to-open in editor.
- Daemon footer — running indicator + active-count + version.

State splits cleanly between two stores:

- **TanStack Query** owns server-derived data: workspace info, file tree,
  Runs list, ScheduledRuns, run records, daemon status. Refetch intervals
  in seconds, plus invalidation triggered by daemon completion events.
- **Zustand** (`useStudio`) owns transient editor state and view routing:
  `view`, `activeFilePath`, `activeRunId`, `selectedScheduledRunId`,
  `inspectorMode`, the editor buffer + dirty flag, per-step run stats,
  breakpoints, pause payload, run log.

`activeFilePath` is a path within the workspace (e.g. `trilogy-rec/recipe.forage`),
not a slug. The slug is derived via the `slugOf(path)` helper. There is one
source of truth for the active file — no parallel `activeSlug`.

## Reactive-UI invariants

- **Leaf-level reads.** Components subscribe to the smallest slice they need
  (`useStudio((s) => s.paused)`), never destructure the whole store at the
  top of a render tree.
- **One-shot commands go through pub/sub, not state.** Native menu events
  ride the Tauri event bus; the engine progress stream is a Tauri event;
  the daemon-completion notification invalidates queries by query-key
  predicate.
- **Reducers are pure.** `runAppend(event)` derives its writes from the
  event payload alone — it never reaches across slices to compute its
  output.

## Native menu events

`apps/studio/src-tauri/src/menu.rs` builds the macOS menu and routes its
items through `app.emit("menu:<id>")`. The frontend listens in
`useStudioEffects.ts`:

- `menu:new_recipe` → `createAndOpenRecipe`
- `menu:save` / `menu:validate` → `saveActive`
- `menu:run_live` / `menu:run_replay` → `runActive(replay)`
- `menu:recipe_delete` → `Sidebar`'s pending-handler dispatch (the context
  menu opens via `show_recipe_context_menu`, which builds a one-item NSMenu
  whose ID is `recipe_delete:<slug>`; selection round-trips back through
  `on_menu_event` as `menu:recipe_delete` with the slug as payload).
