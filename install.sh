#!/usr/bin/env bash
set -euo pipefail

REPOSITORY="${LUNGYAM_REPOSITORY:-phongsathornpt/lungyam}"
VERSION="${VERSION:-latest}"
INSTALL_DIR="${INSTALL_DIR:-}"
DOWNLOAD_BASE="${LUNGYAM_DOWNLOAD_BASE:-}"
BINARY_NAME="lungyam"

fail() {
  printf 'lungyam installer: %s\n' "$*" >&2
  exit 1
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || fail "required command not found: $1"
}

for command_name in curl tar sha256sum install mv uname grep id mktemp; do
  require_command "${command_name}"
done

if [[ "$(uname -s)" != "Linux" ]]; then
  fail "only Linux is supported by this installer"
fi

machine="$(uname -m)"
case "${machine}" in
  x86_64|amd64)
    architecture="x86_64"
    ;;
  aarch64|arm64)
    architecture="aarch64"
    ;;
  *)
    fail "unsupported Linux architecture: ${machine}"
    ;;
esac

if command -v ldd >/dev/null 2>&1 && ldd --version 2>&1 | grep -qi 'musl'; then
  fail "musl Linux is not supported yet; use a glibc-based distribution"
fi

if [[ -z "${INSTALL_DIR}" ]]; then
  if [[ "$(id -u)" -eq 0 ]]; then
    INSTALL_DIR="/usr/local/bin"
  else
    INSTALL_DIR="${HOME:?HOME must be set}/.local/bin"
  fi
fi

if [[ "${VERSION}" == "latest" || -z "${VERSION}" ]]; then
  release_path="latest/download"
  expected_version=""
else
  if [[ "${VERSION}" != v* ]]; then
    VERSION="v${VERSION}"
  fi
  release_path="download/${VERSION}"
  expected_version="${VERSION#v}"
fi

if [[ -n "${DOWNLOAD_BASE}" ]]; then
  base_url="${DOWNLOAD_BASE%/}"
else
  base_url="https://github.com/${REPOSITORY}/releases/${release_path}"
fi

asset="lungyam-linux-${architecture}.tar.gz"
checksum_asset="${asset}.sha256"
work_dir="$(mktemp -d)"
install_tmp=""

cleanup() {
  rm -rf "${work_dir}"
  if [[ -n "${install_tmp}" ]]; then
    rm -f "${install_tmp}" 2>/dev/null || true
  fi
}
trap cleanup EXIT

printf 'Downloading %s\n' "${asset}"
curl -fL --retry 3 --retry-delay 1 --connect-timeout 15 \
  "${base_url}/${asset}" \
  -o "${work_dir}/${asset}"
curl -fL --retry 3 --retry-delay 1 --connect-timeout 15 \
  "${base_url}/${checksum_asset}" \
  -o "${work_dir}/${checksum_asset}"

(
  cd "${work_dir}"
  sha256sum -c "${checksum_asset}"
)

mkdir -p "${work_dir}/package"
tar -xzf "${work_dir}/${asset}" -C "${work_dir}/package"

candidate="${work_dir}/package/${BINARY_NAME}"
if [[ ! -x "${candidate}" ]]; then
  fail "release archive does not contain an executable ${BINARY_NAME} binary"
fi

version_output="$("${candidate}" --version)"
if [[ -n "${expected_version}" && "${version_output}" != "${BINARY_NAME} ${expected_version}" ]]; then
  fail "downloaded binary version mismatch: expected ${expected_version}, got ${version_output}"
fi

mkdir -p "${INSTALL_DIR}"
install_tmp="${INSTALL_DIR}/.${BINARY_NAME}.tmp.$$"
install -m 0755 "${candidate}" "${install_tmp}"
mv -f "${install_tmp}" "${INSTALL_DIR}/${BINARY_NAME}"
install_tmp=""

printf 'Installed %s to %s\n' "${version_output}" "${INSTALL_DIR}/${BINARY_NAME}"
case ":${PATH}:" in
  *":${INSTALL_DIR}:"*) ;;
  *) printf 'Add %s to PATH to run %s directly.\n' "${INSTALL_DIR}" "${BINARY_NAME}" ;;
esac
