#!/usr/bin/env bash
set -euo pipefail

# Smoke test against a live forage-hub-api deployment. Exercises the
# per-version-atomic publish path, stars, downloads, and fork-with-
# lineage. Mirrors the test plan in plans/hub-social-api.md.
#
# Required env:
#   HUB_URL              base URL, e.g. https://api.foragelang.com
#   HUB_PUBLISH_TOKEN    bearer token for POST/DELETE
#
# Optional env:
#   AUTHOR               author segment to publish under (default: smoke)
#   SLUG                 slug segment to publish under (default: zen-leaf)

HUB_URL="${HUB_URL:?HUB_URL is required}"
HUB_PUBLISH_TOKEN="${HUB_PUBLISH_TOKEN:?HUB_PUBLISH_TOKEN is required}"
AUTHOR="${AUTHOR:-smoke}"
SLUG="${SLUG:-zen-leaf}"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

echo "==> using author=$AUTHOR slug=$SLUG"

# Helper that captures both body + HTTP status and asserts on the status.
expect_status() {
    local expected="$1" actual="$2" label="$3"
    if [[ "$actual" != "$expected" ]]; then
        echo "FAIL ($label): expected HTTP $expected, got $actual" >&2
        cat "$TMP/last.json" >&2 || true
        exit 1
    fi
    echo "  ok ($label): HTTP $actual"
}

# 1. health
echo "==> GET /v1/health"
status=$(curl -sS -o "$TMP/last.json" -w "%{http_code}" "$HUB_URL/v1/health")
expect_status 200 "$status" "health"
grep -q '"status":"ok"' "$TMP/last.json"

# 2. publish v1 of @author/slug
echo "==> POST /v1/packages/$AUTHOR/$SLUG/versions (v1)"
BODY_V1=$(python3 -c '
import json, sys
print(json.dumps({
    "description": "Smoke test recipe",
    "category": "dispensary",
    "tags": ["smoke"],
    "recipe": "recipe \"smoke\" {\n  step list { source = \"https://example.com\" }\n}\n",
    "decls": [],
    "fixtures": [],
    "snapshot": None,
    "base_version": None,
    "forked_from": None,
}))
')
status=$(curl -sS -o "$TMP/last.json" -w "%{http_code}" \
    -X POST \
    -H "Authorization: Bearer $HUB_PUBLISH_TOKEN" \
    -H "Content-Type: application/json" \
    -d "$BODY_V1" \
    "$HUB_URL/v1/packages/$AUTHOR/$SLUG/versions")
expect_status 201 "$status" "publish v1"
grep -q '"version":1' "$TMP/last.json"

# 3. atomic version artifact carries everything
echo "==> GET /v1/packages/$AUTHOR/$SLUG/versions/latest"
status=$(curl -sS -o "$TMP/last.json" -w "%{http_code}" \
    "$HUB_URL/v1/packages/$AUTHOR/$SLUG/versions/latest")
expect_status 200 "$status" "versions/latest"
grep -q "\"recipe\":" "$TMP/last.json"
grep -q "\"decls\":" "$TMP/last.json"
grep -q "\"fixtures\":" "$TMP/last.json"

# 4. stale-base publish returns 409 with current latest
echo "==> POST /v1/packages/$AUTHOR/$SLUG/versions (stale base) → 409"
BODY_STALE=$(python3 -c '
import json
print(json.dumps({
    "description": "Smoke test recipe",
    "category": "dispensary",
    "tags": ["smoke"],
    "recipe": "recipe \"smoke\" {\n  step list { source = \"https://example.com\" }\n}\n",
    "decls": [],
    "fixtures": [],
    "snapshot": None,
    "base_version": 0,
    "forked_from": None,
}))
')
status=$(curl -sS -o "$TMP/last.json" -w "%{http_code}" \
    -X POST \
    -H "Authorization: Bearer $HUB_PUBLISH_TOKEN" \
    -H "Content-Type: application/json" \
    -d "$BODY_STALE" \
    "$HUB_URL/v1/packages/$AUTHOR/$SLUG/versions")
expect_status 409 "$status" "stale-base 409"
grep -q '"latest_version":1' "$TMP/last.json"

# 5. downloads counter bump
echo "==> POST /v1/packages/$AUTHOR/$SLUG/downloads"
status=$(curl -sS -o "$TMP/last.json" -w "%{http_code}" \
    -X POST "$HUB_URL/v1/packages/$AUTHOR/$SLUG/downloads")
expect_status 200 "$status" "downloads"
grep -q '"downloads":' "$TMP/last.json"

# 6. listing surfaces the new package
echo "==> GET /v1/packages"
status=$(curl -sS -o "$TMP/last.json" -w "%{http_code}" "$HUB_URL/v1/packages")
expect_status 200 "$status" "list"
grep -q "\"author\":\"$AUTHOR\"" "$TMP/last.json"

# 7. categories list contains "dispensary"
echo "==> GET /v1/categories"
status=$(curl -sS -o "$TMP/last.json" -w "%{http_code}" "$HUB_URL/v1/categories")
expect_status 200 "$status" "categories"
grep -q '"dispensary"' "$TMP/last.json"

# 8. old singleton sub-resources return 404
echo "==> GET /v1/packages/$AUTHOR/$SLUG/fixtures (old shape) → 404"
status=$(curl -sS -o "$TMP/last.json" -w "%{http_code}" \
    "$HUB_URL/v1/packages/$AUTHOR/$SLUG/fixtures")
expect_status 404 "$status" "old-fixtures-route"

echo ""
echo "all smoke tests passed."
