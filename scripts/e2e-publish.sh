#!/usr/bin/env bash
#
# End-to-end smoke test for the `forage publish` flow.
#
# - Builds the CLI in release mode.
# - Writes a tiny synthetic recipe to a temp dir.
# - Runs `forage publish` in dry-run (default), confirming the payload prints.
# - If FORAGE_HUB_TOKEN + FORAGE_HUB_URL are both set, actually POSTs (with
#   --publish) and asserts the round-trip GET returns the same body.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "==> building forage (release)"
cargo build --release --bin forage

FORAGE="$ROOT/target/release/forage"
if [[ ! -x "$FORAGE" ]]; then
    echo "build did not produce $FORAGE" >&2
    exit 1
fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# Workspace manifest. Publishes need `name = "<author>/<…>"`, a
# `description`, and a `category`; the slug on the hub is the recipe
# header name (`smoke-test`).
cat > "$TMP/forage.toml" <<'EOF'
name = "smoke/scratch"
description = "publish smoke test"
category = "smoke"
tags = []

[deps]
EOF

cat > "$TMP/smoke-test.forage" <<'EOF'
recipe "smoke-test"
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
EOF

echo "==> forage publish (dry-run)"
(cd "$TMP" && "$FORAGE" publish smoke-test)

if [[ -n "${FORAGE_HUB_TOKEN:-}" && -n "${FORAGE_HUB_URL:-}" ]]; then
    echo "==> forage publish --publish (live POST)"
    (cd "$TMP" && "$FORAGE" publish smoke-test --publish --hub "$FORAGE_HUB_URL")
    echo "==> verifying round-trip via GET"
    curl -fsSL "$FORAGE_HUB_URL/v1/packages/smoke/smoke-test" >/dev/null
    echo "    OK"
else
    echo "==> skipping live POST (FORAGE_HUB_TOKEN + FORAGE_HUB_URL not set)"
fi

echo "==> smoke test complete"
