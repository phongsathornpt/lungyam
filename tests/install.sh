#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${REPO_ROOT}"

bash -n install.sh
bash -n scripts/package-release.sh

case "$(uname -m)" in
  x86_64|amd64)
    asset_arch="x86_64"
    target="x86_64-unknown-linux-gnu"
    ;;
  aarch64|arm64)
    asset_arch="aarch64"
    target="aarch64-unknown-linux-gnu"
    ;;
  *)
    echo "installer test does not support architecture $(uname -m)" >&2
    exit 1
    ;;
esac

work_dir="$(mktemp -d)"
trap 'rm -rf "${work_dir}"' EXIT

target_dir="${work_dir}/target"
release_dir="${work_dir}/release"
install_dir="${work_dir}/bin"
mkdir -p "${target_dir}/${target}/release" "${release_dir}" "${install_dir}"

cat > "${target_dir}/${target}/release/lungyam" <<'FAKE_BINARY'
#!/usr/bin/env bash
set -euo pipefail
if [[ "${1:-}" == "--version" ]]; then
  printf 'lungyam 0.1.0\n'
  exit 0
fi
printf 'fixture lungyam\n'
FAKE_BINARY
chmod +x "${target_dir}/${target}/release/lungyam"

asset="lungyam-linux-${asset_arch}"
CARGO_TARGET_DIR="${target_dir}" \
SOURCE_DATE_EPOCH=1 \
  bash scripts/package-release.sh "${target}" "${asset}" "${release_dir}"

archive="${release_dir}/${asset}.tar.gz"
checksum="${archive}.sha256"
[[ -f "${archive}" ]]
[[ -f "${checksum}" ]]
tar -tzf "${archive}" | grep -qx 'lungyam'
tar -tzf "${archive}" | grep -qx 'LICENSE'

VERSION=v0.1.0 \
INSTALL_DIR="${install_dir}" \
LUNGYAM_DOWNLOAD_BASE="file://${release_dir}" \
  bash install.sh

[[ -x "${install_dir}/lungyam" ]]
[[ "$("${install_dir}/lungyam" --version)" == "lungyam 0.1.0" ]]

printf 'corruption' >> "${archive}"
if VERSION=v0.1.0 \
  INSTALL_DIR="${work_dir}/bad-install" \
  LUNGYAM_DOWNLOAD_BASE="file://${release_dir}" \
  bash install.sh >/dev/null 2>&1; then
  echo "installer accepted an archive with an invalid checksum" >&2
  exit 1
fi

printf 'release packaging and installer test passed\n'
