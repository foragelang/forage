# Releasing

Forage ships three artifacts on every tagged release:

1. `forage-vX.Y.Z-macos.tar.gz` — the `forage` CLI (universal binary when the runner can cross-compile, arm64-only otherwise).
2. `Forage-Toolkit-vX.Y.Z-macos.dmg` — the macOS Toolkit app.
3. `*.sha256` files alongside each artifact.

`/.github/workflows/release.yml` builds, signs, notarizes (when secrets are configured), packages, and publishes everything to a GitHub Release. The `install.sh` installer and the Homebrew tap point at the assets in that release.

## Cutting a release

1. Bump `version` in `Sources/forage-cli/Forage.swift` (`CommandConfiguration.version`) and in `Sources/Forage/Forage.swift` (`Forage.version`). Keep them in sync.
2. Update `CHANGELOG.md` if it exists. The release-notes step in the workflow extracts the section that matches the tag heading.
3. Tag:

   ```sh
   git tag -a v0.x.0 -m "release notes here"
   git push origin main --tags
   ```

4. The `release` workflow fires on the tag push. Watch it on the Actions tab.
5. When the workflow finishes, verify:

   - The GitHub Release exists with the tarball, the DMG, and both `.sha256` files attached.
   - `curl -fsSL https://foragelang.com/install.sh | sh` installs the new version.
   - If the Homebrew tap is wired up (see below), `brew upgrade foragelang/forage/forage` picks up the new version.

## Required secrets for signed/notarized Toolkit

If any of these are missing, the workflow still publishes — but the Toolkit DMG will be ad-hoc signed and Gatekeeper will warn on first launch.

| Secret | What it is |
| --- | --- |
| `APPLE_DEVELOPER_ID_APPLICATION` | base64-encoded `.p12` of the **Developer ID Application** certificate |
| `APPLE_DEVELOPER_ID_APPLICATION_PASSWORD` | password used to encrypt the `.p12` |
| `APPLE_API_KEY_ID` | App Store Connect API key ID (10-char alphanumeric) |
| `APPLE_API_KEY_ISSUER_ID` | issuer UUID for the API key |
| `APPLE_API_KEY_P8` | full contents of the downloaded `.p8` file (including the `-----BEGIN PRIVATE KEY-----` line) |

To export the `.p12` from Keychain Access:

```sh
# After "Export" → "Personal Information Exchange (.p12)":
base64 -i developer_id_application.p12 | pbcopy
```

The clipboard now contains the value for `APPLE_DEVELOPER_ID_APPLICATION`. The export password is `APPLE_DEVELOPER_ID_APPLICATION_PASSWORD`.

## Homebrew tap automation

The tap repo is `foragelang/homebrew-tap`. The canonical formula source-of-truth in this repo is `homebrew-tap/Formula/forage.rb`; the release workflow can copy it into the tap repo automatically.

To enable automatic updates after each release:

1. Create the `foragelang/homebrew-tap` repo (see `homebrew-tap/README.md` for the initial setup).
2. Create a fine-grained PAT with `contents: write` scoped to `foragelang/homebrew-tap`.
3. Add the PAT as the `HOMEBREW_TAP_TOKEN` secret on `foragelang/forage`.
4. Set the repository **variable** `ENABLE_HOMEBREW_TAP_UPDATE=1`.

After that, every tag push rewrites `Formula/forage.rb` in the tap with the new `url` + `sha256` and pushes the change.

If the automation is not wired, update the formula by hand after each release:

```sh
# In foragelang/homebrew-tap, edit Formula/forage.rb:
#   url    → https://github.com/foragelang/forage/releases/download/vX.Y.Z/forage-vX.Y.Z-macos.tar.gz
#   sha256 → the value from the .sha256 file in the release
git commit -am "forage vX.Y.Z"
git push
```

## Re-running a failed release

If the release fails midway (e.g. notarization timeout), delete the tag, the failed run's artifacts, and the partially-created GitHub Release, then re-tag:

```sh
git tag -d v0.x.0
git push --delete origin v0.x.0
# Delete the GitHub Release (Releases UI or `gh release delete v0.x.0 --yes --cleanup-tag`).
git tag -a v0.x.0 -m "release notes"
git push origin v0.x.0
```

The workflow also supports `workflow_dispatch` for manual re-runs against a specific ref.
