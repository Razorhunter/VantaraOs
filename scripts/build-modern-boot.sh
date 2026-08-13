#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET_SPEC="${ROOT_DIR}/target/generated/x86_64-vantara_os.json"
KERNEL_TARGET_DIR="${ROOT_DIR}/target/modern-kernel"
IMAGE_DIR="${ROOT_DIR}/target/modern-boot"
KERNEL_ELF="${KERNEL_TARGET_DIR}/x86_64-vantara_os/debug/kernel"
CARGO_WRAPPER="${ROOT_DIR}/scripts/cargo-modern-boot.sh"

if [[ "${VANTARA_PINNED_TOOLCHAIN:-0}" == "1" ]]; then
  :
else
  echo "modern boot image requires the pinned Docker toolchain; run: make docker-modern-boot" >&2
  exit 2
fi

bash "${ROOT_DIR}/scripts/generate-rust-target.sh" "${TARGET_SPEC}"
(
  cd "${ROOT_DIR}/kernel"
  CARGO_TARGET_DIR="${KERNEL_TARGET_DIR}" \
    cargo build --bin kernel --features modern-boot --target "${TARGET_SPEC}"
)

(
  cd "${ROOT_DIR}/tools/boot-image"
  CARGO="${CARGO_WRAPPER}" \
  CARGO_TARGET_DIR="${ROOT_DIR}/target/boot-image-builder" \
    "${CARGO_WRAPPER}" run --release -- "${KERNEL_ELF}" "${IMAGE_DIR}"
)

LEGACY_BIOS="${ROOT_DIR}/target/kernel/x86_64-vantara_os/debug/bootimage-kernel.bin"
if [[ -f "${LEGACY_BIOS}" ]]; then
  cp "${LEGACY_BIOS}" "${IMAGE_DIR}/vantara-bios.img"
  echo "copied legacy BIOS image to ${IMAGE_DIR}/vantara-bios.img"
fi
