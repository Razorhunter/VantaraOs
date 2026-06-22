#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERIFY_ROOT="$(mktemp -d /tmp/vantara-package-verify.XXXXXX)"
PACKAGE_ROOT="${VERIFY_ROOT}/vantara-dev"
PACKAGE_MANIFEST="${PACKAGE_ROOT}/metadata/package-manifest.tsv"
PACKAGE_ARCHIVE="${ROOT_DIR}/target/vantara-dev.tar.gz"
INITRD="${PACKAGE_ROOT}/boot/vantara-initrd.tar"

cleanup() {
  rm -rf "${VERIFY_ROOT}"
}
trap cleanup EXIT

if [[ ! -f "${PACKAGE_ARCHIVE}" ]]; then
  echo "package verification failed: missing ${PACKAGE_ARCHIVE#${ROOT_DIR}/}" >&2
  exit 1
fi

tar -xzf "${PACKAGE_ARCHIVE}" -C "${VERIFY_ROOT}"

for required in "${PACKAGE_MANIFEST}" "${INITRD}"; do
  if [[ ! -f "${required}" ]]; then
    echo "package verification failed: missing ${required#${ROOT_DIR}/}" >&2
    exit 1
  fi
done

while IFS=$'\t' read -r path bytes digest; do
  [[ "${path}" == "path" ]] && continue
  artifact="${PACKAGE_ROOT}/${path}"
  if [[ ! -f "${artifact}" ]]; then
    echo "package verification failed: missing ${path}" >&2
    exit 1
  fi
  if [[ "$(stat -c '%s' "${artifact}")" != "${bytes}" ]]; then
    echo "package verification failed: size mismatch ${path}" >&2
    exit 1
  fi
  actual="$(sha256sum "${artifact}")"
  if [[ "${actual%% *}" != "${digest}" ]]; then
    echo "package verification failed: digest mismatch ${path}" >&2
    exit 1
  fi
done <"${PACKAGE_MANIFEST}"

initrd_entries="$(tar -tf "${INITRD}")"
for initrd_path in \
  ./bin/init \
  ./bin/login \
  ./bin/sh \
  ./etc/build-metadata.tsv \
  ./etc/artifact-manifest.tsv \
  ./etc/initrd-manifest.tsv; do
  if ! grep -Fxq "${initrd_path}" <<<"${initrd_entries}"; then
    echo "package verification failed: initrd missing ${initrd_path}" >&2
    exit 1
  fi
done

archive_entries="$(tar -tzf "${PACKAGE_ARCHIVE}")"
for package_path in \
  vantara-dev/boot/vantara-kernel.img \
  vantara-dev/boot/vantara-initrd.tar \
  vantara-dev/disk/vantara-persist.img \
  vantara-dev/metadata/package-manifest.tsv; do
  if ! grep -Fxq "${package_path}" <<<"${archive_entries}"; then
    echo "package verification failed: archive missing ${package_path}" >&2
    exit 1
  fi
done

echo "development package verification passed"
