#!/usr/bin/env bash
# Convenience wrapper. Defaults to the production deployment.
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
HUB_URL="${HUB_URL:-https://api.foragelang.com}" \
HUB_PUBLISH_TOKEN="${HUB_PUBLISH_TOKEN:?HUB_PUBLISH_TOKEN is required}" \
    bash "$SCRIPT_DIR/smoke.sh"
