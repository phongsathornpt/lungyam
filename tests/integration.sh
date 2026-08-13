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
grep -q 'href="/admin/routes"' <<<"$admin_body"

htmx_body=$(curl --silent --fail http://127.0.0.1:19090/admin/assets/htmx.min.js)
grep -q 'version:"2.0.10"' <<<"$htmx_body"

health_fragment=$(curl --silent --fail http://127.0.0.1:19090/admin/fragments/upstream-health)
grep -q '127.0.0.1:39999' <<<"$health_fragment"
grep -q 'health-unhealthy">Unhealthy' <<<"$health_fragment"

routes_body=$(curl --silent --fail http://127.0.0.1:19090/admin/routes)
grep -q '>Routes<' <<<"$routes_body"
grep -q '>echo<' <<<"$routes_body"
grep -q '>api.test<' <<<"$routes_body"
grep -q '>POST<' <<<"$routes_body"
grep -q '>/echo<' <<<"$routes_body"
grep -q '>fixture<' <<<"$routes_body"
grep -q 'Body 64 B' <<<"$routes_body"
grep -q 'href="/admin/routes/new"' <<<"$routes_body"
grep -q 'hx-post="/admin/routes/simulate"' <<<"$routes_body"

matched_route=$(curl --silent --fail \
  --request POST \
  --header 'content-type: application/x-www-form-urlencoded' \
  --data 'host=api.test%3A8443&path=%2Fecho%2Fusers&method=POST' \
  http://127.0.0.1:19090/admin/routes/simulate)
grep -q 'Matched route' <<<"$matched_route"
grep -q 'echo' <<<"$matched_route"
grep -q 'fixture' <<<"$matched_route"

unmatched_route=$(curl --silent --fail \
  --request POST \
  --header 'content-type: application/x-www-form-urlencoded' \
  --data 'host=api.test&path=%2Fecho&method=GET' \
  http://127.0.0.1:19090/admin/routes/simulate)
grep -q 'No route matched' <<<"$unmatched_route"

invalid_simulation=$(curl --silent --fail \
  --request POST \
  --header 'content-type: application/x-www-form-urlencoded' \
  --data 'host=&path=echo&method=POST' \
  http://127.0.0.1:19090/admin/routes/simulate)
grep -q 'Simulation input is invalid' <<<"$invalid_simulation"
grep -q 'path must start' <<<"$invalid_simulation"

new_route_body=$(curl --silent --fail http://127.0.0.1:19090/admin/routes/new)
grep -q '>New route<' <<<"$new_route_body"
grep -q 'hx-post="/admin/routes/validate"' <<<"$new_route_body"
grep -q 'name="name"' <<<"$new_route_body"
grep -q 'name="host"' <<<"$new_route_body"
grep -q 'name="path"' <<<"$new_route_body"
grep -q 'name="methods"' <<<"$new_route_body"
grep -q 'name="priority"' <<<"$new_route_body"
grep -q 'name="upstream"' <<<"$new_route_body"
grep -q 'value="fixture"' <<<"$new_route_body"
grep -q 'name="rate_limit_requests"' <<<"$new_route_body"
grep -q 'name="rate_limit_window_seconds"' <<<"$new_route_body"
grep -q 'name="max_request_body_bytes"' <<<"$new_route_body"
grep -q 'Validation only' <<<"$new_route_body"

valid_route=$(curl --silent --fail \
  --request POST \
  --header 'content-type: application/x-www-form-urlencoded' \
  --data 'name=new-route&host=api.test&path=%2Fnew&methods=GET%2C+POST&upstream=fixture&priority=50&rate_limit_requests=10&rate_limit_window_seconds=60&max_request_body_bytes=1024' \
  http://127.0.0.1:19090/admin/routes/validate)
grep -q 'Configuration is valid' <<<"$valid_route"
grep -q 'new-route' <<<"$valid_route"
grep -q 'Nothing has been persisted' <<<"$valid_route"

invalid_route=$(curl --silent --fail \
  --request POST \
  --header 'content-type: application/x-www-form-urlencoded' \
  --data 'name=bad-route&host=&path=missing-slash&methods=&upstream=fixture&priority=0&rate_limit_requests=&rate_limit_window_seconds=&max_request_body_bytes=' \
  http://127.0.0.1:19090/admin/routes/validate)
grep -q 'Route is invalid' <<<"$invalid_route"
grep -q 'path must start with' <<<"$invalid_route"

duplicate_route=$(curl --silent --fail \
  --request POST \
  --header 'content-type: application/x-www-form-urlencoded' \
  --data 'name=echo&host=api.test&path=%2Fother&methods=POST&upstream=fixture&priority=0&rate_limit_requests=&rate_limit_window_seconds=&max_request_body_bytes=' \
  http://127.0.0.1:19090/admin/routes/validate)
grep -q 'Route is invalid' <<<"$duplicate_route"
grep -q 'duplicate route name' <<<"$duplicate_route"
grep -q 'echo' <<<"$duplicate_route"

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
