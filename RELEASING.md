# Releasing

Forage ships from `.github/workflows/rust-release.yml`. Tags on `main`
matching `v*` trigger the workflow.

Each release produces:

1. `forage-vX.Y.Z-aarch64-apple-darwin.tar.gz` — CLI for Apple Silicon Macs.
2. `forage-vX.Y.Z-x86_64-apple-darwin.tar.gz` — CLI for Intel Macs.
3. `forage-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz` — CLI for x86 Linux.
4. `forage-vX.Y.Z-aarch64-unknown-linux-gnu.tar.gz` — CLI for ARM Linux.
5. `forage-vX.Y.Z-x86_64-pc-windows-msvc.zip` — CLI for Windows.
6. `*.sha256` files alongside each.

Forage Studio (Tauri) DMG / MSI / AppImage are added once R9's
sign-and-notarize secrets are configured — `cargo tauri build` runs
in the same workflow with the platform-appropriate codesigning.

## Cutting a release

1. Bump versions:

   - `[workspace.package].version` in `Cargo.toml` (cascades to every crate).
   - `apps/studio/src-tauri/tauri.conf.json` → `version`.
   - `apps/studio/ui/package.json` → `version`.

2. Update `CHANGELOG.md` (when it exists) with the section heading
   matching the tag. The release-notes step in the workflow extracts
   that section verbatim.

3. Tag + push:

   ```sh
   git tag -a v0.x.0 -m "release notes"
   git push origin main v0.x.0
   ```

4. The workflow runs five parallel `build-cli` jobs (one per target),
   then a `release` job that assembles the GitHub Release.

5. Verify locally once the artifacts are up:

   ```sh
   curl -fsSL https://foragelang.com/install.sh | sh
   forage --version
   ```

## Required secrets for signed/notarized Studio

When R9's release-side wiring lands:

- `APPLE_DEVELOPER_ID_CERT` — base64 .p12 of the Developer ID Application certificate.
- `APPLE_DEVELOPER_ID_PASSWORD` — passphrase for the .p12.
- `APPLE_API_KEY_ID`, `APPLE_API_KEY_ISSUER_ID`, `APPLE_API_KEY` — App Store Connect API key for `xcrun notarytool`.
- `APPLE_TEAM_ID` — Apple Developer team identifier.

If any are missing the workflow still publishes; the Mac artifacts are
ad-hoc signed and Gatekeeper will warn on first launch. The release
notes call this out automatically.

## Homebrew tap

`foragelang/homebrew-tap`'s `Formula/forage.rb` is bumped automatically
by the release workflow's `update-homebrew-tap` job, gated on:

- `ENABLE_HOMEBREW_TAP_UPDATE=1` in repo variables.
- `HOMEBREW_TAP_TOKEN` — fine-grained PAT scoped to the tap with `contents: write`.

When either is absent the job skips silently; the tap stays where it
was and you update the formula by hand.

## Pre-flight checks

Before tagging:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
# wasm-pack output stays in sync:
( cd crates/forage-wasm && wasm-pack build --target web --out-dir ../../hub-site/forage-wasm/pkg )
git diff --exit-code hub-site/forage-wasm/pkg
```

The Rust CI workflow runs the same checks on every push.
