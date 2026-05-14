#!/usr/bin/env bash
# Build Forage Studio (release, Tauri-packaged) and open the .app.
# Use this instead of `cargo tauri dev` when you need a representative
# build — the dev server runs a debug binary that macOS doesn't fully
# register, so anything that goes through LaunchServices or the
# Accessibility / screen-recording APIs (`open -b <bundle-id>`,
# computer-use, automation hooks) sees the wrong process or nothing
# at all. The release bundle has a proper Info.plist and code signing
# state, so it behaves like a real installed app.

set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"

# Kill any running instances so the rebuild + relaunch is clean.
# Matches both the bundled .app process and a raw target/release
# binary in case a half-built tree is lying around.
pkill -f "Forage Studio.app/Contents/MacOS/forage-studio" 2>/dev/null || true
pkill -f "target/release/forage-studio$" 2>/dev/null || true
sleep 1

cd "$repo_root/apps/studio"
# `--bundles app` skips the DMG installer build. The DMG's bundling
# script mounts the disk image to verify it, which makes Finder
# auto-open the mount — a Studio launch dance shouldn't trigger an
# installer window every time.
cargo tauri build --bundles app

open "$repo_root/target/release/bundle/macos/Forage Studio.app"
