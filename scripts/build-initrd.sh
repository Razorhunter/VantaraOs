#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REGISTRY="${ROOT_DIR}/target/generated/userland_images.rs"
BUILD_METADATA="${ROOT_DIR}/target/generated/build-metadata.tsv"
ARTIFACT_MANIFEST="${ROOT_DIR}/target/artifact-manifest.tsv"
INITRD="${ROOT_DIR}/target/vantara-initrd.tar"
INITRD_MANIFEST="${ROOT_DIR}/target/initrd-manifest.tsv"
SOURCE_EPOCH="${SOURCE_DATE_EPOCH:-0}"
STAGING_DIR="$(mktemp -d /tmp/vantara-initrd.XXXXXX)"
INITRD_TEMP="$(mktemp /tmp/vantara-initrd.XXXXXX.tar)"
MANIFEST_TEMP="$(mktemp /tmp/vantara-initrd-manifest.XXXXXX.tsv)"

cleanup() {
  rm -rf "${STAGING_DIR}" "${INITRD_TEMP}" "${MANIFEST_TEMP}"
}
trap cleanup EXIT

for required in "${REGISTRY}" "${BUILD_METADATA}" "${ARTIFACT_MANIFEST}"; do
  if [[ ! -f "${required}" ]]; then
    echo "initrd build failed: missing ${required#${ROOT_DIR}/}" >&2
    exit 1
  fi
done

mkdir -p "${STAGING_DIR}/bin" "${STAGING_DIR}/etc"

mapfile -t userland_files < <(
  sed -nE 's@.*target/userland/([^"]+)".*@\1@p' "${REGISTRY}" | LC_ALL=C sort -u
)

if (( ${#userland_files[@]} == 0 )); then
  echo "initrd build failed: generated registry contains no userland images" >&2
  exit 1
fi

for file_name in "${userland_files[@]}"; do
  source_path="${ROOT_DIR}/target/userland/${file_name}"
  if [[ ! -f "${source_path}" ]]; then
    echo "initrd build failed: missing target/userland/${file_name}" >&2
    exit 1
  fi
  install -m 0755 "${source_path}" "${STAGING_DIR}/bin/${file_name%.*}"
done

install -m 0644 "${BUILD_METADATA}" "${STAGING_DIR}/etc/build-metadata.tsv"
install -m 0644 "${ARTIFACT_MANIFEST}" "${STAGING_DIR}/etc/artifact-manifest.tsv"

{
  printf 'path\tbytes\tsha256\n'
  while IFS= read -r path; do
    relative="${path#${STAGING_DIR}/}"
    size="$(stat -c '%s' "${path}")"
    digest="$(sha256sum "${path}")"
    digest="${digest%% *}"
    printf '/%s\t%s\t%s\n' "${relative}" "${size}" "${digest}"
  done < <(find "${STAGING_DIR}" -type f | LC_ALL=C sort)
} >"${MANIFEST_TEMP}"

install -m 0644 "${MANIFEST_TEMP}" "${STAGING_DIR}/etc/initrd-manifest.tsv"

tar \
  --sort=name \
  --format=ustar \
  --mtime="@${SOURCE_EPOCH}" \
  --owner=0 \
  --group=0 \
  --numeric-owner \
  -C "${STAGING_DIR}" \
  -cf "${INITRD_TEMP}" .

mkdir -p "${ROOT_DIR}/target"
chmod 0644 "${MANIFEST_TEMP}" "${INITRD_TEMP}"
mv -f "${MANIFEST_TEMP}" "${INITRD_MANIFEST}"
mv -f "${INITRD_TEMP}" "${INITRD}"

echo "initrd generated: ${INITRD}"
echo "initrd files: ${#userland_files[@]} user programs + 3 metadata files"
