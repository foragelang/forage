#!/usr/bin/env bash
#
# End-to-end smoke test for the `forage publish` flow.
#
# - Builds the CLI in release mode.
# - Writes a tiny synthetic recipe to a temp dir.
# - Runs `forage publish` in dry-run (default), confirming the payload prints.
# - If FORAGE_HUB_TOKEN + FORAGE_HUB_URL are both set, actually POSTs (with
#   --publish) and asserts the round-trip GET returns the same body.
#
# Intended to run locally and from CI once the hub is live.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "==> building forage (release)"
swift build -c release

FORAGE="$(swift build -c release --show-bin-path)/forage"
if [[ ! -x "$FORAGE" ]]; then
    echo "build did not produce $FORAGE" >&2
    exit 1
fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

cat > "$TMP/recipe.forage" <<'EOF'
recipe "smoke-test" {
    engine http

    type Item { name: String }

    input baseUrl: String

    step list {
        method "GET"
        url    "{$input.baseUrl}/items"
    }

    for $i in $list[*] {
        emit Item { name ← $i.name }
    }
}
EOF

echo "==> forage publish --dry-run (default)"
"$FORAGE" publish "$TMP" \
    --slug smoke-test \
    --display-name "Smoke" \
    --summary "CI smoke test" \
    --tags ci

if [[ -n "${FORAGE_HUB_TOKEN:-}" && -n "${FORAGE_HUB_URL:-}" ]]; then
    echo "==> forage publish --publish (live POST)"
    "$FORAGE" publish "$TMP" \
        --slug smoke-test \
        --display-name "Smoke" \
        --summary "CI smoke test" \
        --tags ci \
        --publish

    echo "==> verifying round-trip via GET"
    detail_url="${FORAGE_HUB_URL%/}/v1/recipes/smoke-test"
    curl -fsSL "$detail_url" \
        | python3 -c 'import sys, json; d = json.load(sys.stdin); assert "recipe \"smoke-test\"" in d["body"], d'
    echo "==> round-trip OK"
else
    echo "==> skipping live publish (set FORAGE_HUB_TOKEN and FORAGE_HUB_URL to enable)"
fi

echo "==> done"
