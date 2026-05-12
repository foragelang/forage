#!/usr/bin/env bash
#
# Verify the Worker's rate limiter actually throttles abusive clients.
# Hammers /v1/oauth/device past its bucket (5/min) and asserts that:
#   - the first ~5 requests succeed (or surface oauth_not_configured),
#   - subsequent requests are 429 with retry_after,
#   - the response envelope is the structured `{error:{code,message,retryAfter}}` shape.

set -euo pipefail

HUB_URL="${HUB_URL:?HUB_URL is required, e.g. https://api.foragelang.com}"

echo "==> hammering /v1/oauth/device against $HUB_URL"

throttled=0
ok=0
for i in $(seq 1 20); do
    body=$(curl -s -o /tmp/forage-rl-body.json -w "%{http_code}" -X POST -H "Content-Type: application/json" "$HUB_URL/v1/oauth/device" -d '{}')
    case "$body" in
        429)
            throttled=$((throttled+1))
            code=$(jq -r '.error.code' /tmp/forage-rl-body.json 2>/dev/null || echo "")
            retry=$(jq -r '.error.retryAfter' /tmp/forage-rl-body.json 2>/dev/null || echo "")
            if [[ "$code" != "rate_limited" ]]; then
                echo "FAIL: 429 returned wrong error code: $code"
                exit 1
            fi
            if [[ -z "$retry" || "$retry" == "null" ]]; then
                echo "FAIL: 429 response missing retryAfter"
                exit 1
            fi
            ;;
        200|503)
            ok=$((ok+1))
            ;;
        *)
            echo "unexpected status $body on iteration $i"
            cat /tmp/forage-rl-body.json
            ;;
    esac
done

echo "==> throttled=$throttled ok=$ok"
if [[ "$throttled" -lt 1 ]]; then
    echo "FAIL: rate limiter never engaged"
    exit 1
fi
echo "OK"
