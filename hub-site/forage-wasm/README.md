# forage-wasm bindings

Compiled output of `crates/forage-wasm`, plus a thin TypeScript adapter
(`adapter.ts`) that presents the surface `RecipeIDE.vue` consumes
(parse, validate, parse+validate, version, HubClient). The Rust core
compiled to WebAssembly is the single implementation; there is no
parallel TS port.

## Building

```sh
cd crates/forage-wasm
wasm-pack build --target web --out-dir ../../hub-site/forage-wasm/pkg
```

Output: `pkg/forage_wasm.js`, `pkg/forage_wasm_bg.wasm`, type
definitions. Loaded once per page; subsequent calls are sync.

## Using

```ts
import { Parser, validate, parseAndValidate, HubClient } from "../../../forage-wasm/adapter.ts";
```

The adapter `init()`s the wasm module lazily, so the import is sync but
`Parser.parse` / `validate` are async (one-time wait on first call).

## Note on `run`

The TS port used to run recipes in-browser via `fetch`. The Rust HTTP
engine doesn't compile to WASM in the current build (reqwest, native
keychain, tokio-multi-thread); recipe execution lives in Studio. The
adapter's `run` is a stub that surfaces this to the user.
