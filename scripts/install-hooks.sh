#!/usr/bin/env bash
# Point this repo's git at `scripts/hooks/` for pre-commit/etc.
# Idempotent: rerunning just re-confirms the config.
#
# Run once after cloning the repo; nothing else needs doing afterwards
# because git reads the hook script from `scripts/hooks/` on every
# commit, so updates to the hooks themselves take effect on the next
# commit with no re-install.

set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root"

git config core.hooksPath scripts/hooks

# Make every hook executable so git is willing to run it. This is the
# one piece of state that needs maintenance — if someone adds a new
# hook script, they should `chmod +x` it themselves or re-run this.
chmod +x scripts/hooks/*

echo "git hooks installed: core.hooksPath = scripts/hooks"
