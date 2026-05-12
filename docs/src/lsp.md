# Language Server

`forage lsp` starts the Forage Language Server on stdio. It's built on
`tower-lsp` and reuses `forage-core` for parsing and validation — the
same code path the CLI's `forage run` uses, so diagnostics in the
editor match diagnostics from the command line.

## Capabilities

- **`textDocument/didOpen` / `didChange` / `didClose`** — full-sync
  document store keyed by URI.
- **`textDocument/publishDiagnostics`** — parse + validate errors,
  re-published after every change. Each issue carries an LSP code
  (`UnknownInput`, `UnknownTransform`, etc.) so editors can filter or
  branch by kind.
- **`textDocument/completion`** — keyword completion (every reserved
  word), transform completion (every registered transform), plus
  per-document additions: `$input.X` items pulled from the recipe's
  declared inputs, `$secret.X` from declared secrets, recipe type
  names, enum names.
- **`textDocument/hover`** — hover over a transform name to see "X —
  built-in transform"; over a recipe type to see its fields; over an
  input to see its declared type; over an enum to see its variants.
- **`textDocument/documentSymbol`** — outline of types, enums, inputs
  with the recipe name as the container.
- **`textDocument/definition`** — advertised; resolution lands when
  validator-side spans land (R7 followup).

## Editor configuration

### VS Code

Install the `foragelang.forage` extension when it's published. Until
then, hand-wire via a generic LSP client:

```jsonc
{
    "forage.lsp.path": "/usr/local/bin/forage",
    "forage.lsp.args": ["lsp"]
}
```

### Neovim (`lspconfig`)

```lua
local lspconfig = require('lspconfig')
local util = require('lspconfig.util')

require('lspconfig.configs').forage = {
    default_config = {
        cmd = { 'forage', 'lsp' },
        filetypes = { 'forage' },
        root_dir = util.root_pattern('Cargo.toml', '.git'),
    },
}
lspconfig.forage.setup({})
```

Add `*.forage` to your filetype detection:

```lua
vim.filetype.add({ extension = { forage = 'forage' } })
```

### Forage Studio

Studio embeds the LSP automatically via a child process — it's wired
in transparently and editor markers come from the same `forage lsp`
binary.

## Why a real LSP

- **Validation that doesn't drift.** The LSP is the parser + validator
  themselves — when the language gains a feature, the editor's
  understanding gains it for free.
- **Errors with code-readability.** The LSP carries the validator's
  semantic information (which input is unknown, which transform is
  missing) directly into editor squiggles.
- **One process, one language.** Forage authors edit recipes in
  editors that already speak LSP; nothing in the toolchain is bespoke.
