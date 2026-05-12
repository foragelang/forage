# forage-wasm bindings

Compiled output of `crates/forage-wasm`, plus a thin TypeScript adapter
(`adapter.ts`) that presents the same API shape `RecipeIDE.vue` currently
gets from `forage-ts/src`.

## Building

```sh
cd crates/forage-wasm
wasm-pack build --target web --out-dir ../../hub-site/forage-wasm/pkg
```

Output: `pkg/forage_wasm.js`, `pkg/forage_wasm_bg.wasm`, type
definitions. Size today: ~540 KB uncompressed; gzip should bring it to
~150 KB. Loaded once per page; subsequent calls are sync.

## Using

Swap this:

```ts
import { Parser, validate } from "../../../forage-ts/src/index.ts";
```

…with:

```ts
import { Parser, validate, parseAndValidate } from "../../../forage-wasm/adapter.ts";
```

The adapter `init()`s the wasm module lazily, so the rest of the code
can stay sync-feeling (Parser.parse / validate are async now, which is
a small surface change in `RecipeIDE.vue`).

## Followup

- Bench `parseAndValidate` against the TS port and confirm wasm wins
  on real recipes.
- Once the hub IDE flips fully, delete `hub-site/forage-ts/` so the
  Rust core is the only source of truth.
