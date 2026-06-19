#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
KERNEL_IMAGE="${ROOT_DIR}/kernel/target/x86_64-vantara_os/debug/bootimage-kernel.bin"
USERLAND_DIR="${ROOT_DIR}/target/userland"
REGISTRY="${ROOT_DIR}/target/generated/userland_images.rs"
BUILD_METADATA="${ROOT_DIR}/target/generated/build-metadata.tsv"
MANIFEST="${ROOT_DIR}/target/artifact-manifest.tsv"
TEMP_MANIFEST="${MANIFEST}.tmp"

require_artifact() {
  local path="$1"
  if [[ ! -f "${path}" ]]; then
    echo "artifact manifest failed: missing ${path#${ROOT_DIR}/}" >&2
    exit 1
  fi
}

write_entry() {
  local kind="$1"
  local path="$2"
  local relative="${path#${ROOT_DIR}/}"
  local size digest

  size="$(stat -c '%s' "${path}")"
  digest="$(sha256sum "${path}")"
  digest="${digest%% *}"
  printf '%s\t%s\t%s\t%s\n' "${kind}" "${relative}" "${size}" "${digest}"
}

require_artifact "${KERNEL_IMAGE}"
require_artifact "${REGISTRY}"
require_artifact "${BUILD_METADATA}"

if [[ ! -d "${USERLAND_DIR}" ]]; then
  echo "artifact manifest failed: missing target/userland" >&2
  exit 1
fi

mkdir -p "${ROOT_DIR}/target"
{
  printf 'type\tpath\tbytes\tsha256\n'
  write_entry "kernel" "${KERNEL_IMAGE}"
  write_entry "metadata" "${BUILD_METADATA}"
  write_entry "registry" "${REGISTRY}"

  while IFS= read -r artifact; do
    write_entry "userland" "${artifact}"
  done < <(find "${USERLAND_DIR}" -maxdepth 1 -type f \( -name '*.bin' -o -name '*.elf' \) | LC_ALL=C sort)
} >"${TEMP_MANIFEST}"

mv "${TEMP_MANIFEST}" "${MANIFEST}"

echo "artifact manifest generated: ${MANIFEST}"
echo "artifacts: $(( $(wc -l <"${MANIFEST}") - 1 ))"
