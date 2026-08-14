#!/usr/bin/env bash
set -euo pipefail

TARGET="${1:?usage: package-release.sh <target-triple> <asset-name> [output-dir]}"
ASSET_NAME="${2:?usage: package-release.sh <target-triple> <asset-name> [output-dir]}"
OUTPUT_DIR="${3:-dist}"
TARGET_DIR="${CARGO_TARGET_DIR:-target}"
BINARY="${TARGET_DIR}/${TARGET}/release/lungyam"
ARCHIVE="${OUTPUT_DIR}/${ASSET_NAME}.tar.gz"
CHECKSUM="${ARCHIVE}.sha256"

if [[ ! -x "${BINARY}" ]]; then
  echo "release binary not found or not executable: ${BINARY}" >&2
  exit 1
fi

if [[ ! -f LICENSE ]]; then
  echo "LICENSE not found; run this script from the repository root" >&2
  exit 1
fi

mkdir -p "${OUTPUT_DIR}"
WORK_DIR="$(mktemp -d)"
trap 'rm -rf "${WORK_DIR}"' EXIT

install -m 0755 "${BINARY}" "${WORK_DIR}/lungyam"
install -m 0644 LICENSE "${WORK_DIR}/LICENSE"

SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-0}"
tar \
  --sort=name \
  --owner=0 \
  --group=0 \
  --numeric-owner \
  --mtime="@${SOURCE_DATE_EPOCH}" \
  -C "${WORK_DIR}" \
  -czf "${ARCHIVE}" \
  lungyam LICENSE

(
  cd "${OUTPUT_DIR}"
  sha256sum "${ASSET_NAME}.tar.gz" > "${ASSET_NAME}.tar.gz.sha256"
)

printf 'created %s\n' "${ARCHIVE}"
printf 'created %s\n' "${CHECKSUM}"
