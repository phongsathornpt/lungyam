#!/usr/bin/env bash
set -euo pipefail

version="2.0.10"
expected_blob="3b7ac1aceb211ca716c7a9c5774c649f74331ee1"
destination="crates/lungyam-admin/vendor/htmx.min.js"
url="https://raw.githubusercontent.com/bigskysoftware/htmx/v${version}/dist/htmx.min.js"

mkdir -p "$(dirname "$destination")"
curl --fail --location --silent --show-error "$url" --output "$destination"

actual_blob=$(git hash-object "$destination")
if [[ "$actual_blob" != "$expected_blob" ]]; then
  echo "htmx integrity check failed: expected $expected_blob, got $actual_blob" >&2
  rm -f "$destination"
  exit 1
fi

echo "vendored htmx v${version} ($actual_blob)"
