# Language tools cleanup — roadmap

Top-10 hacks/holes in how Studio talks to the Forage language tooling,
ordered so each item unblocks the next.

## Findings

1. **AST nodes don't carry `Span`s.** Parser gets them from chumsky and
   throws them away. Without spans on `HTTPStep`, `Emission`,
   `Statement`, the validator and Studio both have to re-derive
   positions from source text.
2. **`ValidationIssue` has no span.** Every validation error is anchored
   at byte 0 in the LSP (`docstore.rs:82–86`), so editor squigglies
   render at line 1 col 1 regardless of where the actual problem is.
3. **`forage-lsp` is built but never spawned by Studio.** A complete
   tower-lsp server (didOpen/didChange/completion/hover/symbols/
   definition) sits in `crates/forage-lsp/src/server.rs`; Studio
   imports the crate in `Cargo.toml` but never instantiates it.
4. **Monaco syntax is a hand-rolled Monarch tokenizer.** Three separate
   keyword/transform lists in `validate/mod.rs`, `lsp/server.rs`, and
   `monaco-forage.ts` — they drift.
5. **Studio re-parses recipes with a regex for breakpoint anchoring.**
   `SourceTab.tsx:16-25` runs `^\s*step\s+(\w+)` on the source instead
   of asking the real parser where steps are. Breaks silently on
   commented-out `step` declarations or future syntax changes.
6. **TS types for every cross-boundary struct are duplicated by hand.**
   `RunEvent`, `StepPause`, `DebugScope`, `ResumeAction`,
   `ValidationOutcome`, `RunOutcome`, `Snapshot` are all defined in
   Rust with serde tags + redefined in `api.ts`. No codegen.
7. **Event/command name strings are duplicated.** `"forage:run-event"`,
   `"forage:debug-paused"`, command names — typo-prone, no shared
   constants.
8. **Validation is save-only and unsituated.** No live validation on
   keystroke; error/warning footer is plain text with no line linkage.
9. **Breakpoints are global and ephemeral.** Set across all recipes,
   not persisted between sessions, no per-recipe scoping.
10. **Mutex/`.expect()` scatter in Studio state.** `breakpoints`,
    `debug_session`, `run_cancel`, `last_context_menu` all live as
    `Mutex<…>` with `.expect("…mutex")` at each site; invariants are
    undocumented; the read-mostly `breakpoints` is a poor fit for
    `Mutex`.

## Execution order

Phase 1 — span foundation (unblocks 2, 5, 8, indirectly 3):

- [F1] Spans on `Statement`, `HTTPStep`, `Emission` in the AST.
       Parser fills them; no public-API change for non-Studio callers.
- [F2] `ValidationIssue { span, … }`; validator threads spans from the
       node being checked. Existing validator tests stay green; LSP +
       Studio start surfacing precise positions.

Phase 2 — Studio bindings + live feedback (kills the manual TS sync):

- [F3] Adopt `tauri-specta` for command IO + event payloads. Generated
       `bindings.ts` becomes the single source of truth for cross-wire
       types. Delete the duplicated TS unions.
- [F4] Live, debounced validation on edit. New `validate_source`
       command (no disk write); Monaco markers use the new spans.
- [F5] Parser-driven step → span map. New `recipe_outline` command
       returns `{ steps: [{ name, span }] }`; SourceTab consumes it
       and drops the regex.

Phase 3 — real LSP (kills the duplicated keyword/transform lists,
gives hover/completion/semantic tokens for free):

- [F6] Spawn `forage-lsp` from Studio over stdio; wire
       `monaco-languageclient` against it. Studio gets diagnostics +
       completion + hover from the same source the CLI uses.
- [F7] Single source of truth for keyword/transform lists. The
       validator owns `BUILTIN_TRANSFORMS`; the LSP imports them; the
       Monaco hand-rolled list goes away once F6's semantic tokens
       cover highlighting.

Phase 4 — polish:

- [F8] Per-recipe persistent breakpoints. Sidecar JSON in Studio
       config dir, keyed by recipe slug, loaded on `setActive`.
- [F9] `for`-iteration breakpoints. Extend `Debugger` with a second
       hook (`before_iteration`); Studio UI gets a toggle.
- [F10] Studio state hygiene. `ArcSwap` for the breakpoint set,
        documented invariants on the debug-session machinery, fewer
        scattered `.expect()`s.

Each item lands as its own commit so the history reads as a clean
progression rather than one giant rewrite.
