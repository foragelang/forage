# Releasing

Forage ships from `.github/workflows/release.yml`, triggered manually
via **Actions → Release → Run workflow**. There's no tag-push path: the
workflow calculates the next tag itself, runs the full CI gate, builds
all artifacts, then creates and pushes the tag and publishes the
release.

## Cutting a release

1. Open Actions → **Release** → **Run workflow**.
2. Optionally enter a specific commit SHA. Empty means HEAD of `main`.
3. Click **Run workflow**.

The workflow does the rest:

- **calculate-version** — picks `YEAR.MONTH.patch` based on the latest
  `v<YEAR>.<MONTH>.*` tag. First release of the month is `.0`; each
  re-dispatch within the same month increments.
- **ci** — calls `rust-ci.yml` against the release commit. A red CI
  blocks the release.
- **build-cli** — five parallel jobs:
  - `aarch64-apple-darwin`
  - `x86_64-apple-darwin`
  - `x86_64-unknown-linux-gnu`
  - `aarch64-unknown-linux-gnu` (cross-compiled)
  - `x86_64-pc-windows-msvc`
- **build-studio** — three parallel jobs:
  - `aarch64-apple-darwin` (DMG + .app zip)
  - `x86_64-apple-darwin` (DMG + .app zip)
  - `x86_64-pc-windows-msvc` (MSI)
- **release** — assembles every artifact, creates and pushes the
  `v<VERSION>` tag, then publishes the GitHub Release with
  auto-generated changelog plus a fixed install-instructions header.
- **update-homebrew-tap** — bumps `foragelang/homebrew-tap`'s
  `Formula/forage.rb` (gated; see below).

Each release produces:

1. `forage-vX.Y.Z-aarch64-apple-darwin.tar.gz` — CLI for Apple Silicon Macs.
2. `forage-vX.Y.Z-x86_64-apple-darwin.tar.gz` — CLI for Intel Macs.
3. `forage-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz` — CLI for x86 Linux.
4. `forage-vX.Y.Z-aarch64-unknown-linux-gnu.tar.gz` — CLI for ARM Linux.
5. `forage-vX.Y.Z-x86_64-pc-windows-msvc.zip` — CLI for Windows.
6. `forage-studio-vX.Y.Z-aarch64-apple-darwin.dmg` (+ `.app.zip`).
7. `forage-studio-vX.Y.Z-x86_64-apple-darwin.dmg` (+ `.app.zip`).
8. `forage-studio-vX.Y.Z-x86_64-pc-windows-msvc.msi`.
9. `*.sha256` files alongside each.

## How the version is applied

Version files in the repo stay pinned to a `0.1.0` placeholder. The
release workflow sed/jq-edits them in-place at build time and does not
commit anything back:

- `Cargo.toml` — every `version = "0.1.0"` (workspace package + every
  local-crate pin in `[workspace.dependencies]`).
- `apps/studio/src-tauri/tauri.conf.json` — `.version`.
- `packages/studio-ui/package.json` — `.version`.

If you bump these by hand for some reason, keep them all on the same
placeholder so the sed/jq edits land cleanly.

## Required secrets for signed/notarized Studio

- `APPLE_DEVELOPER_ID_CERT` — base64 .p12 of the Developer ID Application certificate.
- `APPLE_DEVELOPER_ID_PASSWORD` — passphrase for the .p12.
- `APPLE_SIGNING_IDENTITY` — the identity string Tauri should pick (e.g. `Developer ID Application: Foragelang LLC (TEAMID)`).
- `APPLE_API_KEY_ID`, `APPLE_API_KEY_ISSUER_ID`, `APPLE_API_KEY` — App Store Connect API key for `xcrun notarytool`.

If any of these are missing the workflow still publishes; the Mac
artifacts are ad-hoc signed and Gatekeeper will warn on first launch.

## Homebrew tap

`foragelang/homebrew-tap`'s `Formula/forage.rb` is bumped automatically
by the release workflow's `update-homebrew-tap` job, gated on:

- `ENABLE_HOMEBREW_TAP_UPDATE=1` in repo variables.
- `HOMEBREW_TAP_TOKEN` — fine-grained PAT scoped to the tap with `contents: write`.

When either is absent the job skips silently; the tap stays where it
was and you update the formula by hand.

## Pre-1.0 compatibility breakages

Forage is pre-1.0 greenfield. Some releases ship incompatible changes
to on-disk artifacts without migration shims. When that happens, the
release notes call out what users need to clear; the list below tracks
the cumulative effect.

- **Linked-runtime release** — the daemon's deployment format changes
  from `<deployments>/<name>/v<n>/recipe.forage + catalog.json` to
  `<deployments>/<name>/v<n>/module.json` (a serialized `LinkedModule`
  carrying the recipe plus its transitive closure of composition
  stages). Existing deployment directories are unreadable under the
  new format. Users recover by removing the whole deployments tree
  and re-deploying:

  ```sh
  rm -rf <workspace>/.forage/deployments/
  ```

  Run-row pointers (`Run.deployed_version`) survive but point at
  versions whose on-disk payload is gone; the next deploy lands at
  the next version and the run row picks it up. Pause scheduled Runs
  before clearing — any scheduled fire between the wipe and the
  redeploy records a failure row pointing at the missing payload.

## Pre-flight (run locally if you don't trust CI)

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

`rust-ci.yml` runs the same checks on every push, and the release
workflow re-runs them against the release commit before building any
artifacts.
