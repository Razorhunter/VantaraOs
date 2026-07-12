#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BOOTIMAGE="${ROOT_DIR}/target/kernel/x86_64-vantara_os/debug/bootimage-kernel.bin"
INITRD="${ROOT_DIR}/target/vantara-initrd.tar"
INITRD_MANIFEST="${ROOT_DIR}/target/initrd-manifest.tsv"
BUILD_METADATA="${ROOT_DIR}/target/generated/build-metadata.tsv"
ARTIFACT_MANIFEST="${ROOT_DIR}/target/artifact-manifest.tsv"
STAGING_PARENT="$(mktemp -d /tmp/vantara-package.XXXXXX)"
PACKAGE_ROOT="${STAGING_PARENT}/vantara-dev"
PACKAGE_MANIFEST="${PACKAGE_ROOT}/metadata/package-manifest.tsv"
PACKAGE_ARCHIVE="${ROOT_DIR}/target/vantara-dev.tar.gz"
PACKAGE_ARCHIVE_TEMP="$(mktemp /tmp/vantara-dev.XXXXXX.tar.gz)"
SOURCE_EPOCH="${SOURCE_DATE_EPOCH:-0}"
PERSIST_SIZE="${PERSIST_SIZE:-1M}"

cleanup() {
  rm -rf "${STAGING_PARENT}" "${PACKAGE_ARCHIVE_TEMP}"
}
trap cleanup EXIT

for required in \
  "${BOOTIMAGE}" \
  "${INITRD}" \
  "${INITRD_MANIFEST}" \
  "${BUILD_METADATA}" \
  "${ARTIFACT_MANIFEST}"; do
  if [[ ! -f "${required}" ]]; then
    echo "package build failed: missing ${required#${ROOT_DIR}/}" >&2
    exit 1
  fi
done

mkdir -p \
  "${PACKAGE_ROOT}/boot" \
  "${PACKAGE_ROOT}/disk" \
  "${PACKAGE_ROOT}/metadata"

install -m 0644 "${BOOTIMAGE}" "${PACKAGE_ROOT}/boot/vantara-kernel.img"
install -m 0644 "${INITRD}" "${PACKAGE_ROOT}/boot/vantara-initrd.tar"
truncate -s "${PERSIST_SIZE}" "${PACKAGE_ROOT}/disk/vantara-persist.img"
install -m 0644 "${BUILD_METADATA}" "${PACKAGE_ROOT}/metadata/build-metadata.tsv"
install -m 0644 "${ARTIFACT_MANIFEST}" "${PACKAGE_ROOT}/metadata/artifact-manifest.tsv"
install -m 0644 "${INITRD_MANIFEST}" "${PACKAGE_ROOT}/metadata/initrd-manifest.tsv"

{
  printf '%s\n' \
    'Vantara OS development package' \
    '' \
    'boot/vantara-kernel.img  BIOS boot disk image' \
    'boot/vantara-initrd.tar  Canonical userland initrd artifact' \
    'disk/vantara-persist.img Blank persistent VANTFS disk template' \
    'metadata/                 Build and integrity manifests' \
    '' \
    'Current boot compatibility embeds userland in the kernel image.' \
    'The initrd is packaged now for the future boot-module handoff.'
} >"${PACKAGE_ROOT}/README.txt"

{
  printf 'path\tbytes\tsha256\n'
  while IFS= read -r path; do
    relative="${path#${PACKAGE_ROOT}/}"
    [[ "${relative}" == "metadata/package-manifest.tsv" ]] && continue
    size="$(stat -c '%s' "${path}")"
    digest="$(sha256sum "${path}")"
    digest="${digest%% *}"
    printf '%s\t%s\t%s\n' "${relative}" "${size}" "${digest}"
  done < <(find "${PACKAGE_ROOT}" -type f | LC_ALL=C sort)
} >"${PACKAGE_MANIFEST}"

tar \
  --sort=name \
  --format=ustar \
  --mtime="@${SOURCE_EPOCH}" \
  --owner=0 \
  --group=0 \
  --numeric-owner \
  -C "${STAGING_PARENT}" \
  -cf - vantara-dev \
  | gzip -n >"${PACKAGE_ARCHIVE_TEMP}"

mkdir -p "${ROOT_DIR}/target"
chmod 0644 "${PACKAGE_ARCHIVE_TEMP}"
mv -f "${PACKAGE_ARCHIVE_TEMP}" "${PACKAGE_ARCHIVE}"

echo "package archive: ${PACKAGE_ARCHIVE}"
