# forage-file simplification: workspace migration log

Phase 10 record. This branch ships `forage migrate`, the one-shot tool
that restructures any pre-1.0 workspace from the legacy
`<slug>/recipe.forage` plus `<slug>/fixtures/` plus
`<slug>/snapshot.json` shape into the flat
`<recipe>.forage` plus `_fixtures/<recipe>.jsonl` plus
`_snapshots/<recipe>.json` shape every other Phase-1..9 surface now
expects.

The migration is greenfield: no compat shims, no fallback for the legacy
shape past this branch.

## The author's local workspace

`~/Library/Forage/Recipes/` was migrated through the new
`forage migrate --apply` against the worktree-built CLI:

```
$ cargo run --bin forage -- migrate ~/Library/Forage/Recipes/ --apply
```

Nine recipes flattened from `<slug>/recipe.forage` to `<slug>.forage`
at the workspace root (every header name matches its old folder slug,
so no rename was needed):

```
remedy-baltimore  remedy-columbia  trilogy-med  trilogy-rec
untitled-1  untitled-2  untitled-3  untitled-4
zen-leaf-elkridge
```

Five recipes carried a sibling `fixtures/inputs.json` (legacy daemon
auto-load convention). The migrator leaves these files in place and
logs a `warn!` per occurrence — the new CLI takes `--inputs <path>`
explicitly, and the user can either point `forage run` at the existing
file or rehome it as they prefer. The four `untitled-*` recipes had no
`inputs.json`; their empty `fixtures/` and parent `<slug>/` directories
were removed.

No `_fixtures/` or `_snapshots/` data needed migrating — the
workspace had no recorded captures or snapshots in the legacy layout.

The daemon's SQLite state under `.forage/` was already keyed by recipe
header name (Phase 4 took care of that), so the workspace-level
restructure left the daemon's run history untouched.

## Pre-existing recipe issues surfaced by the migration

The five "real" recipes (`remedy-baltimore`, `remedy-columbia`,
`trilogy-med`, `trilogy-rec`, `zen-leaf-elkridge`) reference
transforms — `prevalenceNormalize`, `parseSize`, `normalizeOzToGrams`,
`sizeValue`, `sizeUnit`, `normalizeUnitToGrams` — that aren't
registered in the built-in transform registry and aren't declared as
`fn` in any workspace file. They failed validation **before** the
migration on the legacy shape just as they do after on the flat shape.
The migration is shape-only; resolving the missing transforms (either
land them in `default_registry` or write them as workspace
`share fn`s) is a separate concern.

The four `untitled-*` recipes validate cleanly. They fail at run time
because they have no captured fixtures — also pre-existing.

## What `forage migrate` does

A migration plan covers five kinds of action; dry-run prints the plan,
`--apply` materializes it:

1. **Recipe moves**: each `<root>/<slug>/recipe.forage` becomes
   `<root>/<header-name>.forage`. The header name beats the folder
   slug when they differ.
2. **Fixture moves**: every `*.jsonl` under `<slug>/fixtures/` is
   concatenated into `<root>/_fixtures/<header-name>.jsonl`. Source
   ordering is deterministic (sorted by file name).
3. **Snapshot moves**: `<slug>/snapshot.json` becomes
   `<root>/_snapshots/<header-name>.json`.
4. **Share-keyword insertion**: every header-less `.forage` file at the
   workspace root has each of its bare `type`/`enum`/`fn` declarations
   surgically rewritten with a `share ` prefix at the declaration's
   byte-span start. Existing `share`-marked declarations are left
   untouched (idempotent on re-run).
5. **Empty-dir cleanup**: `<slug>/fixtures/` and `<slug>/` are removed
   when empty after the moves. A leftover the migrator doesn't
   recognize (an `inputs.json`, a stray text file) pins the directory
   open and surfaces a `warn!` rather than getting deleted.

All actions log through `tracing` at `info!` for moves and `warn!`
for skip-or-leave decisions. Destination collisions (a file already
exists at the target path) abort the plan **before** any write so the
operator can resolve the conflict and rerun.

## Rollback story (and its limits)

`forage migrate` makes filesystem moves, not copies, and renames
in-place via `fs::rename`. If the apply pass aborts midway — say, a
permissions error halfway through — the workspace is half-flat,
half-legacy. Re-running `--apply` after fixing the cause will pick up
whatever's still in the legacy shape; the already-moved files become
no-op skips because they no longer match the legacy-shape predicate.

No automatic rollback is offered. The author cloned
`~/Library/Forage/Recipes/` to
`~/Library/Forage/Recipes.pre-migration-backup-2026-05-15` before
running `--apply`; users running this against a non-backed-up workspace
should do the same. The dry-run mode (the default — `--apply` is opt-in)
exists precisely to let operators inspect the plan before committing
to it.

## Idempotence

Running `forage migrate --apply` against an already-flat workspace is
a clean no-op. The planner reports "nothing to migrate" in dry-run
and exits without touching the filesystem in `--apply`. This was
verified end-to-end on the migrated `~/Library/Forage/Recipes/`.
