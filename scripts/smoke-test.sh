#!/usr/bin/env bash
# Runs a built scratchpad binary headless and checks SMTP capture end to end through the web API.
#   scripts/smoke-test.sh path/to/scratchpad
set -euo pipefail
BIN="${1:?usage: smoke-test.sh <scratchpad binary>}"
DATA="$(mktemp -d)"
SMTP_PORT="${SMTP_PORT:-22525}"
WEB_PORT="${WEB_PORT:-28025}"

"$BIN" --version
"$BIN" --headless --data-dir "$DATA" --smtp-port "$SMTP_PORT" --web-port "$WEB_PORT" --web-auth smoke:test >"$DATA/log" 2>&1 &
PID=$!
trap 'kill $PID 2>/dev/null || true; rm -rf "$DATA"' EXIT

for _ in $(seq 1 50); do
  curl -fsS "http://127.0.0.1:$WEB_PORT/healthz" >/dev/null 2>&1 && break
  sleep 0.2
done
curl -fsS "http://127.0.0.1:$WEB_PORT/healthz"; echo
"$BIN" --healthcheck --data-dir "$DATA" --web-port "$WEB_PORT"
test "$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$WEB_PORT/api/status")" = 401

curl -fsS -u smoke:test -X POST -H 'content-type: application/json' -d '{"mailbox":"Smoke"}' \
  "http://127.0.0.1:$WEB_PORT/api/test-email"
for _ in $(seq 1 25); do
  COUNT="$(curl -fsS -u smoke:test "http://127.0.0.1:$WEB_PORT/api/mailboxes/Smoke/emails" | grep -o '"id"' | wc -l | tr -d ' ')"
  [[ "$COUNT" == "1" ]] && break
  sleep 0.2
done
if [[ "$COUNT" != "1" ]]; then
  echo "expected 1 captured email, found $COUNT"; cat "$DATA/log"; exit 1
fi
ID="$(curl -fsS -u smoke:test "http://127.0.0.1:$WEB_PORT/api/mailboxes/Smoke/emails" | grep -o '"id":"[^"]*"' | head -1 | cut -d'"' -f4)"
curl -fsS -u smoke:test "http://127.0.0.1:$WEB_PORT/api/emails/$ID/export" | grep -q 'id="scratchpad-data"'
echo "smoke test passed: SMTP capture, health check, web API and export work"
