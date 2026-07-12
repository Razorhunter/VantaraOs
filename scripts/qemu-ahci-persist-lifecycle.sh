#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CARGO_TARGET_DIR="${ROOT_DIR}/target/storage-ahci-cargo"
BOOTIMAGE="${CARGO_TARGET_DIR}/x86_64-vantara_os/debug/bootimage-kernel.bin"
DISK_IMAGE="${ROOT_DIR}/target/vantara-ahci-persist-test.img"

bash "${ROOT_DIR}/scripts/build-kernel-variant.sh" \
  "${CARGO_TARGET_DIR}" \
  --no-default-features \
  --features storage-ahci

BOOTIMAGE_OVERRIDE="${BOOTIMAGE}" \
PERSIST_IMAGE_OVERRIDE="${DISK_IMAGE}" \
QEMU_MACHINE=q35 \
PERSIST_BACKEND=ahci \
QEMU="${QEMU:-qemu-system-x86_64}" \
  bash "${ROOT_DIR}/scripts/qemu-filesystem-persistence.sh"

echo "AHCI /persist lifecycle test passed: controlled backend survived two boots"
