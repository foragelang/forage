# Workspace lifecycle PR audit

Branch: `workspace-lifecycle` rebased onto `origin/main` (head `ed544d0`).
Plan: `plans/workspace-lifecycle.md`.

## Acceptance commands

| Command                                | Result                  |
| -------------------------------------- | ----------------------- |
| `cargo check --workspace`              | PASS (clean)            |
| `cargo test --workspace`               | PASS (all suites green) |
| `npm test` in `apps/studio/ui`         | PASS (13/13)            |
| Welcome on launch / no eager daemon    | PASS (`lib.rs:42–66`)   |
| ⌘W wired + disabled when no workspace  | PASS                    |

Boot path (`lib.rs:46–65`) only constructs an empty `StudioState`, captures the
menu handle, and stashes it. No `Daemon::open`, no eager workspace load. The
`Close Workspace` MenuItem is built with `.enabled(false)` (`menu.rs:17`); the
open/close commands flip it via `set_close_workspace_enabled`
(`commands.rs:1660`).

## Structural checks (numbered per request)

1. **StudioState shape** — Confirmed. `state.rs:48` `daemon: ArcSwapOption<Daemon>`,
   `state.rs:55` `workspace: ArcSwapOption<Workspace>`, `state.rs:72`
   `menu_close_workspace: Mutex<Option<MenuItem<Wry>>>`. Extra
   `workspace_switch: tokio::sync::Mutex<()>` at `state.rs:78` matches the plan's
   close→open serialization note.
2. **`require_daemon` / `require_workspace`** — `commands.rs:48–63`. Every
   workspace-scoped command goes through them; only callers that bypass are the
   intentional lifecycle writers (`state.workspace.store` at lines 1110/1623/1652,
   `state.daemon.store` at 1624/1647) and `current_workspace` itself
   (`commands.rs:1085`, which returns `Option`).
3. **Recents persistence** — Atomic write via `NamedTempFile::new_in` + `persist`
   (`workspace.rs:566–569`). Dedup-by-canonical-path + 10-entry truncation in
   `record_recent` (`workspace.rs:606–608`). `read_recents` filters missing-path
   entries without rewriting the sidecar (`workspace.rs:537–547`). Path is
   `dirs::data_dir()/Forage/recents.json` (with `FORAGE_DATA_DIR` test override).
4. **`open_workspace` flow** — Path checks → load → daemon → `install_daemon`
   shared helper (`commands.rs:69–81`, used by setup-via-no-op and by open) →
   start scheduler → swap into state → record recents → enable menu item → emit
   event (`commands.rs:1590–1633`).
5. **`close_workspace` flow** — Take prior daemon, `daemon.close()`, take
   workspace, disable menu item, emit `forage:workspace-closed`
   (`commands.rs:1638–1658`). Idempotent: when no prior daemon, the function
   returns without emitting (`was_open == false` branch).
6. **`new_workspace` flow** — `workspace::write_empty_manifest` does
   `mkdir -p + manifest write` and rejects existing `forage.toml`
   (`workspace.rs:29–41`), then delegates to `open_workspace_inner`.
7. **Native menu** — `open_workspace` (⌘O) and `close_workspace` (⌘W) at top of
   File submenu (`menu.rs:38–47`). `PredefinedMenuItem::close_window` dropped.
   `on_menu_event` emits both events (`menu.rs:85–90`).
8. **Frontend boot branch** — `App.tsx:27` `<BootSplash />` while pending;
   `App.tsx:28` Welcome when `null/undefined`; `<StudioShell />` otherwise.
9. **Welcome view fidelity** — Tagline exact (`Welcome/index.tsx:157`). Footer
   `forage · v{version} · daemon offline` (`Welcome/index.tsx:208`). Recent
   rows render name, path, last-opened; section hides on empty list
   (`Welcome/index.tsx:188`). The ForageMark SVG dimensions (`36/36/1.4`) match
   `plans/workspace-lifecycle-design/workspace-lifecycle-variants.html` — the
   canonical reference cited in the plan — not the older `welcome.jsx` source
   (`40/40/1.5`). Treating variants.html as canonical: ✓.
10. **Sidebar switcher** — `Sidebar.tsx:215–303`. `PopoverTrigger` is the
    folder-icon · name · chevron header. Popover shows "CURRENT WORKSPACE" label
    + truncated path, divider, Open Workspace ⌘O, Close Workspace ⌘W (red via
    `is-danger` class). Amber ring via `.workspace-switcher-trigger` rule in
    `styles.css:330–334`.
11. **Daemon swap correctness** — See 🟡 finding below: writers serialize via
    `workspace_switch`, but readers (`require_daemon`/`require_workspace`) load
    each slot independently and can observe a mid-swap state where one is
    `Some` and the other is still `None`.
12. **Greenfield discipline** — No `#[allow(dead_code)]`, no
    `--no-verify`. `#[serde(default)]` is present on `RecentsFile.workspaces`
    (`workspace.rs:494`) but it's an initial-schema default, not a
    rename-absorbing one; CLAUDE.md only bans the latter, so permitted.
    See 🟡 finding on `unwrap_or_else` masking the `dirs::data_dir()` None case.
13. **PE deviations**
    - `tempfile` move from `[dev-dependencies]` to runtime is a clean shift,
      no extra features (`Cargo.toml` diff: only the line position changes).
    - ⌘N context-aware: single keydown handler in `useStudioEffects.ts:129–136`
      branches on `readWorkspace(qc)`; the matching `menu:new_recipe` listener
      at lines 155–161 does the same. One listener per surface, both gated on
      workspace presence. Coherent.
14. **Tests** — Frontend: all four plan tests present in `App.test.tsx`
    (lines 72, 92, 121, 139). Backend: see 🟡 findings on missing
    `open_workspace_rejects_dir_without_manifest`,
    `close_workspace_idempotent`, and the absent log-capture in the corrupt-JSON
    test.

## Findings

### 🔴 Critical
None.

### 🟡 Significant

1. **`workspace.rs:507–510` — silent fallback when `dirs::data_dir()` returns
   `None`.** `recents_path()` does
   `.unwrap_or_else(|| PathBuf::from(".forage-data"))`. CLAUDE.md's
   "Never mask errors with defaults" rule bans exactly this pattern: "no
   `unwrap_or_else(|| "default")`". The fallback puts the recents sidecar under
   the process cwd, which would persist recents in surprising places if the
   platform ever lacks a data dir. Surface with `.expect()` or change the
   signature to `Result<PathBuf, _>`. The companion plan §"Style / discipline
   reminders" reiterates: "No `.unwrap_or_default()` / `?? ""` masking. Surface
   ... via `Result<T, String>` at the command boundary."

2. **Half-installed state observable between `workspace` and `daemon` stores.**
   Plan §6 "Daemon swap": "the implementation is `close → open` under a Mutex
   guard so nothing reads half-installed state mid-swap." The mutex
   (`workspace_switch`, `state.rs:78`) only serializes writers — readers
   (`require_daemon`/`require_workspace`, `commands.rs:48–63`) load each
   `ArcSwapOption` independently and never lock the mutex. In `open_workspace_inner`
   (`commands.rs:1623–1624`) the workspace store happens before the daemon
   store; in `close_workspace_inner` (`commands.rs:1647–1652`) the daemon clear
   happens before the workspace clear. A concurrent command landing in the
   window between the two stores sees `workspace=Some, daemon=None` (open
   path) or the inverse (close path). Not crash-causing because both helpers
   bail with `"no workspace open"`, but the plan promised the swap looked
   atomic to readers. Fix: pack both into a single `ArcSwapOption<(Arc<Daemon>,
   Arc<Workspace>)>` so one swap installs/clears the pair.

3. **Missing plan-mandated tests.** Plan §"Test plan / Backend" enumerates eight
   tests; two are missing from `apps/studio/src-tauri/src/workspace.rs` (or
   anywhere else):
   - `open_workspace_rejects_dir_without_manifest` — the dir-without-manifest
     rejection happens in `open_workspace_inner` (`commands.rs:1602–1607`), but
     no test exercises it. The helper paths it builds on (`write_empty_manifest`,
     `forage_core::workspace::load`) are covered, but the rejection branch
     itself isn't.
   - `close_workspace_idempotent` — the idempotent guard at `commands.rs:1648`
     (`was_open` flag) is testable through `close_workspace_inner` directly
     (no `State` needed if you refactor to take `&StudioState` instead of
     `&State<'_, StudioState>`), or via a small helper. As-is, the contract
     "second call returns Ok, no panic" is unverified.

4. **`read_recents_on_corrupt_json_returns_empty` skips the log assertion.**
   Plan: "assert `Ok(vec![])` **and a `tracing` warn event was recorded**". The
   test in `workspace.rs:955–963` only asserts emptiness; the `tracing::warn!`
   at `workspace.rs:533` is not exercised. Plan acknowledged the
   `tracing-test` dep gate ("check existing usage in the repo before adding a
   dep") — there is none — so the simplification is defensible, but the test
   name omits the `logs_and_` segment and the log path is untested. Either add
   the assertion (via a custom `tracing` subscriber) or rename to match the
   reduced coverage.

### 🔵 Minor

5. **`workspace.rs:565` `path.parent().unwrap_or(Path::new("."))`.** Used as the
   tempfile placement directory in `write_recents`. The `recents_path()` always
   returns a path with a parent (it joins `Forage/recents.json`), so the fallback
   is structurally unreachable. `.expect("recents path has parent")` would
   document the invariant; the silent `"."` fallback would land tempfiles in
   the cwd if anyone ever changed `recents_path`.

6. **`new_workspace_rejects_dir_with_existing_manifest` tests the helper, not
   the command.** `workspace.rs:1042–1047` exercises `write_empty_manifest`'s
   `AlreadyExists` branch but not the `new_workspace` command path. Acceptable
   given the `State<'_, StudioState>` constraint, but worth noting the contract
   "the command refuses" is one layer of indirection from the assertion.

### 💭 Questions
None — the plan and the diff line up cleanly on the rest of the surface.

## Verdict: **Request Changes**

The lifecycle, menu wiring, frontend branching, recents sidecar, and switcher UI
all match the plan. The two notable backend defects are the masked
`dirs::data_dir()` failure (§1) and the daemon/workspace swap race (§2), both of
which the plan explicitly forbids. The missing tests (§3, §4) leave the
idempotency and "missing manifest" contracts unverified end-to-end. None of
these are gating in the "ship it broken" sense, but they're the kind of details
this PR was written to nail down.
