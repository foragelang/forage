#!/usr/bin/env bash
set -euo pipefail

# Smoke test against a live forage-hub-api deployment.
#
# Required env:
#   HUB_URL              base URL, e.g. https://api.foragelang.com
#   HUB_PUBLISH_TOKEN    bearer token for POST/DELETE
#
# Optional env:
#   HUB_SLUG             slug to use (default: smoke-test-recipe)
#   RECIPE_PATH          path to a .forage file (default: ../recipes/sweed/recipe.forage)

HUB_URL="${HUB_URL:?HUB_URL is required}"
HUB_PUBLISH_TOKEN="${HUB_PUBLISH_TOKEN:?HUB_PUBLISH_TOKEN is required}"
SLUG="${HUB_SLUG:-smoke-test-recipe}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RECIPE_PATH="${RECIPE_PATH:-$SCRIPT_DIR/../../recipes/sweed/recipe.forage}"

if [[ ! -f "$RECIPE_PATH" ]]; then
    echo "recipe not found: $RECIPE_PATH" >&2
    exit 1
fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

echo "==> using slug=$SLUG"
echo "==> using recipe=$RECIPE_PATH"

# Helper that captures both body + HTTP status and asserts on the status.
expect_status() {
    local expected="$1" actual="$2" label="$3"
    if [[ "$actual" != "$expected" ]]; then
        echo "FAIL ($label): expected HTTP $expected, got $actual" >&2
        exit 1
    fi
    echo "  ok ($label): HTTP $actual"
}

# 1. health
echo "==> GET /v1/health"
status=$(curl -sS -o "$TMP/health.json" -w "%{http_code}" "$HUB_URL/v1/health")
expect_status 200 "$status" "health"
grep -q '"status":"ok"' "$TMP/health.json"

# Clean up prior test runs so this script is rerunnable.
echo "==> DELETE /v1/packages/$SLUG (pre-clean)"
curl -sS -o /dev/null -X DELETE \
    -H "Authorization: Bearer $HUB_PUBLISH_TOKEN" \
    "$HUB_URL/v1/packages/$SLUG" || true

# 2. publish v1
echo "==> POST /v1/packages (v1)"
BODY=$(python3 -c '
import json, sys
slug, recipe_path = sys.argv[1], sys.argv[2]
with open(recipe_path, "r", encoding="utf-8") as f:
    body = f.read()
print(json.dumps({
    "slug": slug,
    "author": "smoke",
    "displayName": "Smoke Test Recipe",
    "summary": "Used by hub-api/test/smoke.sh.",
    "tags": ["test", "smoke"],
    "platform": "sweed",
    "files": [{"name": "recipe.forage", "body": body}],
}))
' "$SLUG" "$RECIPE_PATH")
status=$(curl -sS -o "$TMP/publish1.json" -w "%{http_code}" \
    -X POST \
    -H "Authorization: Bearer $HUB_PUBLISH_TOKEN" \
    -H "Content-Type: application/json" \
    -d "$BODY" \
    "$HUB_URL/v1/packages")
expect_status 201 "$status" "publish v1"
grep -q "\"slug\":\"$SLUG\"" "$TMP/publish1.json"
grep -q '"version":1' "$TMP/publish1.json"

# 3. list contains the slug
echo "==> GET /v1/packages"
status=$(curl -sS -o "$TMP/list.json" -w "%{http_code}" "$HUB_URL/v1/packages")
expect_status 200 "$status" "list"
grep -q "\"slug\":\"$SLUG\"" "$TMP/list.json"

# 4. detail body matches
echo "==> GET /v1/packages/$SLUG"
status=$(curl -sS -o "$TMP/detail.json" -w "%{http_code}" "$HUB_URL/v1/packages/$SLUG")
expect_status 200 "$status" "detail"
python3 -c '
import json, sys
with open(sys.argv[1], "r") as f: detail = json.load(f)
with open(sys.argv[2], "r") as f: body = f.read()
file_bodies = detail["file_bodies"]
assert any(f["name"] == "recipe.forage" and f["body"] == body for f in file_bodies), "body mismatch"
print("  ok (detail body roundtrips)")
' "$TMP/detail.json" "$RECIPE_PATH"

# 5. publish v2
echo "==> POST /v1/packages (v2, same slug)"
status=$(curl -sS -o "$TMP/publish2.json" -w "%{http_code}" \
    -X POST \
    -H "Authorization: Bearer $HUB_PUBLISH_TOKEN" \
    -H "Content-Type: application/json" \
    -d "$BODY" \
    "$HUB_URL/v1/packages")
expect_status 201 "$status" "publish v2"
grep -q '"version":2' "$TMP/publish2.json"

# 6. versions returns length 2
echo "==> GET /v1/packages/$SLUG/versions"
status=$(curl -sS -o "$TMP/versions.json" -w "%{http_code}" \
    "$HUB_URL/v1/packages/$SLUG/versions")
expect_status 200 "$status" "versions"
python3 -c '
import json, sys
with open(sys.argv[1], "r") as f: versions = json.load(f)
assert isinstance(versions, list), "expected array"
assert len(versions) == 2, f"expected 2 versions, got {len(versions)}"
print(f"  ok (versions count = {len(versions)})")
' "$TMP/versions.json"

# 7. delete
echo "==> DELETE /v1/packages/$SLUG"
status=$(curl -sS -o /dev/null -w "%{http_code}" \
    -X DELETE \
    -H "Authorization: Bearer $HUB_PUBLISH_TOKEN" \
    "$HUB_URL/v1/packages/$SLUG")
expect_status 204 "$status" "delete"

# 8. detail after delete returns 410
echo "==> GET /v1/packages/$SLUG (post-delete)"
status=$(curl -sS -o "$TMP/gone.json" -w "%{http_code}" \
    "$HUB_URL/v1/packages/$SLUG")
expect_status 410 "$status" "post-delete"

echo ""
echo "all smoke tests passed."
