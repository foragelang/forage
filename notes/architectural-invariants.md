# Architectural invariants

- **Flat workspace.** A workspace is a directory marked by `forage.toml`.
  `.forage` source files live at any depth — typically flat at the
  workspace root, but subdirectories are allowed. File position carries
  no semantics. Every `.forage` file may declare a recipe (via a
  `recipe "<name>" engine <kind>` header) and/or `type` / `enum` / `fn`
  declarations. Declarations marked `share` join a workspace-wide
  catalog visible to every other file; un-`share`d declarations are
  file-scoped. The CLI, the LSP, and Studio all
  `forage_core::workspace::discover` on the workspace root and build a
  `TypeCatalog` per recipe by merging that recipe's file-local
  declarations with every `share`d declaration in the workspace plus
  every cached hub-dep declaration.
- **Recipe identity = header name.** A recipe is named by the string in
  its `recipe "<name>"` header, not by its file path or folder. The
  daemon, output stores, fixtures, and snapshots all key on header name.
  File basenames are incidental — file organization is the user's call.
- **Data dirs at the workspace root.** Fixtures and snapshots live in
  `_fixtures/` and `_snapshots/`, named by recipe name:
  `_fixtures/<recipe>.jsonl`, `_snapshots/<recipe>.json`. The hidden
  `.forage/` directory holds the daemon's runtime state
  (`daemon.sqlite`, per-recipe output stores under
  `data/<recipe>.sqlite`).
- **In-process daemon, per workspace.** `forage-daemon` owns the
  `daemon.sqlite` (runs + scheduled runs) and the output stores under
  `<workspace>/.forage/`. Studio embeds it as `Arc<Daemon>`; an
  out-of-process binary is a future drop-in against the same crate API.
- **Path-based editing surface, name-based running surface.** Studio's
  file commands (`load_file`, `save_file`, `list_workspace_files`) take
  workspace-relative paths — editing is filesystem-shaped. Run /
  configure / debug commands take recipe names (from headers). The two
  namespaces are separate: a file at `foo.forage` may contain a recipe
  named `bar`. The active file is a path; the active recipe is a name;
  they're tracked independently.
- **One source of truth per concern.** The daemon holds the canonical
  `Workspace`; Studio reads through `Daemon::workspace()` rather than
  caching a duplicate. Cross-boundary types are defined once in Rust with
  ts-rs export; the TS side imports them — no hand-maintained mirrors.

See `grammar.md` for the file format, `forage-studio.md` for the Studio
architecture in detail, and `../plans/workspaces.md` for the original
workspaces design — note that this invariants doc supersedes the
slug-folder layout from that plan.
