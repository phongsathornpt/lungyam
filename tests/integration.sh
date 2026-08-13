#!/usr/bin/env bash
set -euo pipefail

backend_pid=""
proxy_pid=""

cleanup() {
  if [[ -n "$proxy_pid" ]]; then kill "$proxy_pid" 2>/dev/null || true; fi
  if [[ -n "$backend_pid" ]]; then kill "$backend_pid" 2>/dev/null || true; fi
}
trap cleanup EXIT

python3 tests/fixtures/backend.py > /tmp/lungyam-backend.log 2>&1 &
backend_pid=$!

RUST_LOG=info cargo run -p lungyam-cli -- --config tests/fixtures/lungyam.yaml > /tmp/lungyam-proxy.log 2>&1 &
proxy_pid=$!

ready=false
for _ in $(seq 1 80); do
  if curl --silent --fail http://127.0.0.1:18080/health >/dev/null 2>&1 \
    && curl --silent --fail http://127.0.0.1:19090/admin/health >/dev/null 2>&1; then
    ready=true
    break
  fi
  if ! kill -0 "$proxy_pid" 2>/dev/null; then
    cat /tmp/lungyam-proxy.log
    exit 1
  fi
  sleep 0.25
done

if [[ "$ready" != "true" ]]; then
  echo "Lungyam did not become ready"
  cat /tmp/lungyam-proxy.log
  exit 1
fi

admin_health_visible=false
for _ in $(seq 1 40); do
  admin_body=$(curl --silent --fail http://127.0.0.1:19090/admin)
  if grep -q '127.0.0.1:39999' <<<"$admin_body" \
    && grep -q 'health-unhealthy">Unhealthy' <<<"$admin_body"; then
    admin_health_visible=true
    break
  fi
  if ! kill -0 "$proxy_pid" 2>/dev/null; then
    cat /tmp/lungyam-proxy.log
    exit 1
  fi
  sleep 0.25
done

if [[ "$admin_health_visible" != "true" ]]; then
  echo "Admin UI did not surface unhealthy upstream status"
  curl --silent http://127.0.0.1:19090/admin || true
  cat /tmp/lungyam-proxy.log
  exit 1
fi

grep -q '/admin/assets/htmx.min.js' <<<"$admin_body"
grep -q 'hx-get="/admin/fragments/upstream-health"' <<<"$admin_body"
grep -q 'hx-trigger="every 5s"' <<<"$admin_body"

htmx_body=$(curl --silent --fail http://127.0.0.1:19090/admin/assets/htmx.min.js)
grep -q 'version:"2.0.10"' <<<"$htmx_body"

health_fragment=$(curl --silent --fail http://127.0.0.1:19090/admin/fragments/upstream-health)
grep -q '127.0.0.1:39999' <<<"$health_fragment"
grep -q 'health-unhealthy">Unhealthy' <<<"$health_fragment"

headers=$(mktemp)
body=$(mktemp)
status=$(curl --silent --show-error \
  --output "$body" \
  --dump-header "$headers" \
  --write-out '%{http_code}' \
  --request POST 'http://127.0.0.1:18080/echo?hello=world' \
  --header 'Host: api.test' \
  --header 'content-type: text/plain' \
  --header 'x-remove-me: remove-this' \
  --data-binary 'ping-body')

test "$status" = "200"
grep -q '^method=POST$' "$body"
grep -q '^path=/echo?hello=world$' "$body"
grep -q '^body=ping-body$' "$body"
grep -q '^x-added=from-lungyam$' "$body"
grep -q '^x-remove-me=$' "$body"
grep -Eq '^x-request-id=ly-[0-9]+$' "$body"
grep -q '^x-lungyam-route=echo$' "$body"
grep -qi '^x-response-transform: yes' "$headers"
grep -qi '^x-request-id: ly-' "$headers"

# The fixture pool intentionally contains a closed endpoint. Repeated requests
# must still succeed through health-based selection and bounded connect retry.
for attempt in 1 2 3 4; do
  failover_status=$(curl --silent --show-error \
    --output /dev/null \
    --write-out '%{http_code}' \
    --request POST 'http://127.0.0.1:18080/echo?failover=true' \
    --header 'Host: api.test' \
    --data-binary "attempt-$attempt")
  test "$failover_status" = "200"
done

not_found=$(curl --silent --output /dev/null --write-out '%{http_code}' \
  --header 'Host: api.test' \
  http://127.0.0.1:18080/echo)
test "$not_found" = "404"

too_large=$(printf '%080d' 0 | curl --silent --output /dev/null --write-out '%{http_code}' \
  --request POST \
  --header 'Host: api.test' \
  --data-binary @- \
  http://127.0.0.1:18080/echo)
test "$too_large" = "413"

echo "integration test passed"
