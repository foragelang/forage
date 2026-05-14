# Language-tools roadmap — open items

The original 10-item list is largely resolved. Spans are on every AST node
(`Statement`, `HTTPStep`, `Emission`, expressions, types) and on
`ValidationIssue`; Studio + the LSP both render diagnostics at precise
positions. ts-rs generates the cross-boundary TS types from the Rust
definitions, so there is one canonical wire shape per concept. Live
validation runs on every keystroke via `validate_recipe`; parser-driven
step locations come from `recipe_outline`. Breakpoints persist per-recipe
to a workspace sidecar; `for`-iteration pausing is gated by the
`set_pause_iterations` command. State concurrency is `ArcSwap`-driven on
the hot read paths (`breakpoints`, `debug_session`).

What's still open:

## Spawn `forage-lsp` from Studio

`crates/forage-lsp` is a complete tower-lsp server (didOpen/didChange,
hover, completion, diagnostics, semantic tokens) but Studio doesn't
instantiate it. Monaco gets its language config from a hand-rolled Monarch
tokenizer in `monaco-forage.ts`, even though the keyword + transform
inventory now comes from `language_dictionary` (which proxies
`forage-core::parse::KEYWORDS` etc).

The win: dropping the Monarch tokenizer in favor of LSP semantic tokens
removes one of two places where syntax highlighting lives, and unlocks
real completion + hover across the workspace (cross-file go-to-definition
on shared declarations files, in particular).

Approach: spawn `forage-lsp` from the Tauri backend over stdio; wire
`monaco-languageclient` against it on the frontend. Studio's existing
hover / outline / dictionary commands stay (they're useful for in-process
fast paths) but the editor's primary intelligence flows through the LSP.

## Cross-file LSP validation

Once the LSP is live in Studio, an edit to a workspace-level declarations
file should re-validate every open recipe in the same workspace and
publish updated diagnostics. The LSP already loads the workspace via
`forage_core::workspace::discover`; what's missing is the cross-file
publish-diagnostics machinery and the `forage.toml` / `*.forage` file
watcher.

## Schema-level drift detection

The daemon currently derives `Health::Drift` from emit *counts* only — a
recipe whose emit shape changes (a field disappears or changes type) but
whose count stays steady reads as healthy. The output store has the
authoritative column inventory; adding a per-Run schema fingerprint
(`type → field shape hash`) and comparing across runs gives schema-drift
detection too. Sequenced after the LSP work because both paths want a
"compare this snapshot against prior runs" primitive on the daemon side.
