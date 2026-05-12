# Building from source

## Prerequisites

- Rust 1.85 or newer (`rustup default stable`).
- Node 20 or newer + `npm` (for the Studio frontend and the hub-site).
- `wasm-pack` (for `forage-wasm`):
  `curl -fsSL https://rustwasm.github.io/wasm-pack/installer/init.sh | sh`.
- `cargo-tauri` (for building Studio bundles):
  `cargo install tauri-cli@2.8 --locked`.
- macOS: Xcode Command Line Tools.
- Linux: `libwebkit2gtk-4.1-dev`, `libgtk-3-dev`,
  `libayatana-appindicator3-dev`, `librsvg2-dev`, `libsoup-3.0-dev`.
- Windows: WebView2 runtime (preinstalled on Windows 11; installer
  for 10).

## Quick path

```sh
git clone https://github.com/foragelang/forage
cd forage

cargo build --workspace                 # entire Rust workspace
cargo test --workspace                  # ~50 tests across 13 crates
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check

# Run the CLI:
./target/debug/forage run recipes/hacker-news

# Build forage-wasm and refresh hub-site:
( cd crates/forage-wasm && wasm-pack build --target web --out-dir ../../hub-site/forage-wasm/pkg )

# Studio dev loop:
( cd apps/studio/ui && npm install )
( cd apps/studio && cargo tauri dev )
```

## Layout

```
crates/
├── forage-core/        # AST + parser + validator + evaluator + transforms + snapshot
├── forage-http/        # HTTP engine
├── forage-browser/     # browser engine (wry under the `live` feature)
├── forage-hub/         # hub client + OAuth device flow
├── forage-keychain/    # keyring wrapper
├── forage-replay/      # capture types + replayer
├── forage-lsp/         # tower-lsp server
├── forage-wasm/        # wasm-bindgen exports for the web IDE
└── forage-test/        # shared-recipes harness
apps/
├── cli/                # `forage` binary
└── studio/             # Tauri 2 + React 19 + Monaco
hub-api/                # Cloudflare Worker (TypeScript)
hub-site/               # VitePress (Vue)
site/                   # VitePress homepage
docs/                   # this mdbook
recipes/                # in-tree platform recipes
Tests/shared-recipes/   # cross-implementation parity vectors
```

## Adding a crate

```sh
cargo new --lib crates/forage-foo --vcs none
```

Then:

1. Add a workspace-style `[package]` heading inheriting from the root
   workspace:

   ```toml
   [package]
   name = "forage-foo"
   version.workspace = true
   edition.workspace = true
   rust-version.workspace = true
   license.workspace = true
   repository.workspace = true
   authors.workspace = true
   ```

2. Add an entry to the root `Cargo.toml`'s `[workspace] members` list,
   and a `forage-foo = { path = "crates/forage-foo", version = "0.1.0" }`
   in `[workspace.dependencies]` so other crates can pull it in via
   `workspace = true`.

3. Wire any new public surface into `lib.rs` re-exports.

4. `cargo test -p forage-foo` and `cargo clippy -p forage-foo --
   -D warnings` before committing.

## Cross-platform builds

CI builds + tests on macOS-15, ubuntu-latest, windows-latest. The
`build-studio` job exercises `cargo tauri build` on all three.
Locally:

```sh
# Build for a different target:
rustup target add x86_64-unknown-linux-gnu
cargo build --release --bin forage --target x86_64-unknown-linux-gnu
```

`apps/studio` includes wry/tao transitively; rebuilds on Linux need
the GTK/WebKitGTK deps listed above.
