#!/usr/bin/env bash
# Regenerate the Studio Xcode project from project.yml, then open it.
set -euo pipefail

DIR="$(cd "$(dirname "$0")" && pwd)"
STUDIO_DIR="$DIR/Studio"
PROJECT="$STUDIO_DIR/Studio.xcodeproj"

if ! command -v xcodegen >/dev/null 2>&1; then
    echo "xcodegen not installed. Install with: brew install xcodegen" >&2
    exit 1
fi

( cd "$STUDIO_DIR" && xcodegen )
open "$PROJECT"
